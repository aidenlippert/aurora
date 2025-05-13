use clap::{Parser, Subcommand};
use tokio::net::TcpStream;
use tokio::io::{AsyncWriteExt, AsyncBufReadExt, BufReader as TokioBufReader}; // Use Tokio's BufReader
use serde::{Deserialize, Serialize}; 
use serde_json::json; // For constructing JSON params
use uuid::Uuid; // For request IDs

// --- RPC Structs (mirrored from node) ---
#[derive(Serialize, Deserialize, Debug)]
struct RpcRequest {
    id: String,
    method: String,
    params: serde_json::Value,
}

#[derive(Serialize, Deserialize, Debug)]
struct RpcResponse {
    id: String,
    result: Option<serde_json::Value>,
    error: Option<RpcError>,
}

#[derive(Serialize, Deserialize, Debug)]
struct RpcError {
    code: i32,
    message: String,
}
// --- End RPC Structs ---

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Cli {
    #[clap(subcommand)]
    command: Commands,
    #[clap(long, default_value = "127.0.0.1:10001", help = "Address:Port of the target node's RPC server")]
    rpc_target: String,
}

#[derive(Subcommand, Debug)]
enum Commands {
    SubmitTx {
        #[clap(long, help = "Data payload for the transaction (will be sent as a string)")]
        data: String,
    },
    GetNodeState {},
}

async fn send_rpc_request(target_addr: &str, request: RpcRequest) -> Result<RpcResponse, Box<dyn std::error::Error>> {
    let mut stream = TcpStream::connect(target_addr).await?;
    let request_json = serde_json::to_string(&request)? + "\n"; // Add newline delimiter
    stream.write_all(request_json.as_bytes()).await?;
    stream.flush().await?;

    let mut reader = TokioBufReader::new(stream); // Use Tokio's BufReader
    let mut response_line = String::new();
    reader.read_line(&mut response_line).await?;
    
    let response: RpcResponse = serde_json::from_str(response_line.trim())?;
    Ok(response)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::SubmitTx { data } => {
            let request_id = Uuid::new_v4().to_string();
            let params = json!({ "data": data });
            let rpc_req = RpcRequest {
                id: request_id.clone(),
                method: "submit_transaction".to_string(),
                params,
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
        }
        Commands::GetNodeState {} => {
            let request_id = Uuid::new_v4().to_string();
            let rpc_req = RpcRequest {
                id: request_id.clone(),
                method: "get_node_state".to_string(),
                params: serde_json::Value::Null, // No params for get_node_state
            };
            println!("CLI: Sending to {}: {:?}", cli.rpc_target, rpc_req);
            match send_rpc_request(&cli.rpc_target, rpc_req).await {
                Ok(response) => {
                     if let Some(err) = response.error {
                        eprintln!("CLI: RPC Error: code {}, message: {}", err.code, err.message);
                    } else if let Some(result) = response.result {
                        println!("CLI: RPC Success (Req ID: {}): Node State:", response.id);
                        println!("{}", serde_json::to_string_pretty(&result)?);
                    } else {
                        eprintln!("CLI: Received empty successful response.");
                    }
                }
                Err(e) => eprintln!("CLI: Failed to send/receive RPC: {}", e),
            }
        }
    }
    Ok(())
}