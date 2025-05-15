// tools/aurora_testnet_cli/src/main.rs
use clap::{Parser, Subcommand};
use tokio::net::TcpStream;
use tokio::io::{AsyncWriteExt, AsyncBufReadExt, BufReader as TokioBufReader};
use serde::{Deserialize, Serialize}; 
use serde_json::json;
use uuid::Uuid;

#[derive(Serialize, Deserialize, Debug)]
struct RpcRequest { id: String, method: String, params: serde_json::Value }
#[derive(Serialize, Deserialize, Debug)]
struct RpcResponse { id: String, result: Option<serde_json::Value>, error: Option<RpcError> }
#[derive(Serialize, Deserialize, Debug)]
struct RpcError { code: i32, message: String }

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Cli {
    #[clap(subcommand)]
    command: Commands,
    #[clap(long, default_value = "127.0.0.1:19001", help = "Address:Port of target node RPC")]
    rpc_target: String,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Submit a public AUC transfer transaction
    SubmitTransfer {
        #[clap(long, help = "Sender's app layer public key (hex string)")]
        sender_pk_hex: String,
        #[clap(long, help = "Recipient's app layer public key (hex string)")]
        recipient_pk_hex: String,
        #[clap(long, help = "Amount of AUC to transfer (integer)")]
        amount: u64,
        #[clap(long, help = "Nonce for the sender (integer, for replay protection)")]
        nonce: u64,
    },
    /// Get the public AUC balance of an account
    GetBalance {
        #[clap(long, help = "Account's app layer public key (hex string)")]
        account_pk_hex: String,
    },
    /// Get the current state of the node
    GetNodeState {},
}

async fn send_rpc_request(target_addr: &str, request: RpcRequest) -> Result<RpcResponse, Box<dyn std::error::Error>> {
    let mut stream = TcpStream::connect(target_addr).await?;
    let request_json = serde_json::to_string(&request)? + "\n";
    stream.write_all(request_json.as_bytes()).await?;
    stream.flush().await?;
    let mut reader = TokioBufReader::new(stream);
    let mut response_line = String::new();
    reader.read_line(&mut response_line).await?;
    let response: RpcResponse = serde_json::from_str(response_line.trim())?;
    Ok(response)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let request_id = Uuid::new_v4().to_string();

    let rpc_req = match cli.command {
        Commands::SubmitTransfer { sender_pk_hex, recipient_pk_hex, amount, nonce } => {
            // Construct the TransferAucPayload directly for params
            let params = json!({
                "sender_pk_hex": sender_pk_hex,
                "recipient_pk_hex": recipient_pk_hex,
                "amount": amount,
                "nonce": nonce
            });
            RpcRequest {
                id: request_id,
                method: "submit_transaction".to_string(), // Still use this method name
                params,
            }
        }
        Commands::GetBalance { account_pk_hex } => {
            let params = json!({ "account_pk_hex": account_pk_hex });
            RpcRequest {
                id: request_id,
                method: "get_balance".to_string(),
                params,
            }
        }
        Commands::GetNodeState {} => {
            RpcRequest {
                id: request_id,
                method: "get_node_state".to_string(),
                params: serde_json::Value::Null,
            }
        }
    };

    println!("CLI: Sending to {}: {:?}", cli.rpc_target, rpc_req);
    match send_rpc_request(&cli.rpc_target, rpc_req).await {
        Ok(response) => {
            if let Some(err) = response.error {
                eprintln!("CLI: RPC Error: code {}, message: {}", err.code, err.message);
            } else if let Some(result) = response.result {
                println!("CLI: RPC Success (Req ID: {}): {}", response.id, serde_json::to_string_pretty(&result)?);
            } else {
                eprintln!("CLI: Received empty successful response.");
            }
        }
        Err(e) => eprintln!("CLI: Failed to send/receive RPC: {}", e),
    }
    Ok(())
}