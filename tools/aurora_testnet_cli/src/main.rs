use clap::{Parser, Subcommand};
use triad_web::NetworkMessage; // For sending NetworkMessage::Transaction
use ecliptic_concordance::TransactionPayload;
use std::net::SocketAddr;
use tokio::net::TcpStream;
use tokio::io::{AsyncWriteExt, AsyncReadExt};
use serde_json;
use bincode; // Using bincode for network messages

// Copied from triad_web/src/tcp_p2p.rs for CLI to send in same format
async fn send_framed_message_cli<W: AsyncWriteExt + Unpin>(stream: &mut W, msg: &NetworkMessage) -> std::io::Result<()> {
    let encoded = bincode::serialize(msg).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    let len = encoded.len() as u32;
    stream.write_u32(len).await?;
    stream.write_all(&encoded).await?;
    stream.flush().await?;
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)] // For node to send back state
struct NodeStateResponse {
    height: u64,
    last_block_hash: String,
}

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Cli {
    #[clap(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    SubmitTx {
        #[clap(long, help = "Address:Port of the target node (sequencer), e.g., 127.0.0.1:8001")]
        target_addr: String,
        #[clap(long, help = "Data payload for the transaction")]
        data: String,
    },
    QueryNodeState { // Changed from QueryConsensusState
        #[clap(long, help = "Address:Port of the target node, e.g., 127.0.0.1:8001")]
        target_addr: String,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::SubmitTx { target_addr, data } => {
            let socket_addr: SocketAddr = target_addr.parse()?;
            println!("CLI: Submitting transaction with data '{}' to node at {}", data, socket_addr);
            
            let tx_payload_struct = TransactionPayload { data: data.into_bytes() };
            // Serialize the TransactionPayload struct itself to send as the data within NetworkMessage::Transaction
            let serialized_transaction_payload_data = serde_json::to_vec(&tx_payload_struct)?;
            
            let network_msg = NetworkMessage::Transaction(serialized_transaction_payload_data);

            match TcpStream::connect(socket_addr).await {
                Ok(mut stream) => {
                    if let Err(e) = send_framed_message_cli(&mut stream, &network_msg).await {
                        eprintln!("CLI: Error sending transaction over TCP: {}", e);
                    } else {
                        println!("CLI: Transaction payload sent successfully to {}.", target_addr);
                    }
                }
                Err(e) => {
                    eprintln!("CLI: Failed to connect to target node {}: {}", target_addr, e);
                }
            }
        }
        Commands::QueryNodeState { target_addr } => {
            let socket_addr: SocketAddr = target_addr.parse()?;
            println!("CLI: Querying state from node at {} (very basic TCP query)", target_addr);

            match TcpStream::connect(socket_addr).await {
                Ok(mut stream) => {
                    // Send a simple query request string (nodes aren't set up to handle this complexly yet)
                    // For v0.0.1, we'll have the CLI just print the node's local file path
                    // as a reminder, since implementing the query-response in the node is more work.
                    println!("CLI: To get node state, check its _blockchain.jsonl file.");
                    println!("     (A real RPC/P2P query is needed for live state from a running node).");
                    // As a placeholder for future:
                    // stream.write_all(b"QUERY_STATE_REQUEST").await?;
                    // let mut buffer = [0; 1024];
                    // let n = stream.read(&mut buffer).await?;
                    // let response_str = String::from_utf8_lossy(&buffer[..n]);
                    // println!("Node response: {}", response_str);
                }
                Err(e) => {
                    eprintln!("CLI: Failed to connect to target node {}: {}", target_addr, e);
                }
            }
        }
    }
    Ok(())
}
