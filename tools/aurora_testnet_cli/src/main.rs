use clap::{Parser, Subcommand};
use triad_web::NetworkMessage;
use ecliptic_concordance::TransactionPayload;
use std::net::SocketAddr;
use tokio::net::TcpStream;
use tokio::io::{AsyncWriteExt, AsyncReadExt, BufReader}; // Added BufReader
use serde_json;
use bincode;
use serde::Deserialize; // For NodeStateSummary

// Copied from triad_web/src/tcp_p2p.rs for CLI to interact in same format
async fn send_framed_message_cli<W: AsyncWriteExt + Unpin>(stream: &mut W, msg: &NetworkMessage) -> std::io::Result<()> {
    let encoded = bincode::serialize(msg).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    let len = encoded.len() as u32;
    stream.write_u32(len).await?;
    stream.write_all(&encoded).await?;
    stream.flush().await?;
    Ok(())
}
async fn read_framed_message_cli<R: AsyncReadExt + Unpin>(stream: &mut R) -> std::io::Result<Option<NetworkMessage>> {
    match stream.read_u32().await {
        Ok(len) => {
            if len == 0 { return Ok(None); }
            if len > 10 * 1024 * 1024 { return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Message too large"));}
            let mut buffer = vec![0; len as usize];
            stream.read_exact(&mut buffer).await?;
            bincode::deserialize(&buffer).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e)).map(Some)
        }
        Err(ref e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(None),
        Err(e) => Err(e),
    }
}


#[derive(Debug, Serialize, Deserialize)] // Matching node's response struct
struct NodeStateSummary {
    node_id: String,
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
    QueryNodeState {
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
            let serialized_payload_for_network_message = serde_json::to_vec(&tx_payload_struct)?; // CLI sends JSON payload
            let network_msg = NetworkMessage::Transaction(serialized_payload_for_network_message);
            match TcpStream::connect(socket_addr).await {
                Ok(mut stream) => {
                    if let Err(e) = send_framed_message_cli(&mut stream, &network_msg).await {
                        eprintln!("CLI: Error sending transaction over TCP: {}", e);
                    } else { println!("CLI: Transaction payload sent successfully to {}.", target_addr); }
                }
                Err(e) => { eprintln!("CLI: Failed to connect to target node {}: {}", target_addr, e); }
            }
        }
        Commands::QueryNodeState { target_addr } => {
            let socket_addr: SocketAddr = target_addr.parse()?;
            println!("CLI: Querying state from node at {}", target_addr);
            match TcpStream::connect(socket_addr).await {
                Ok(mut stream) => {
                    let query_msg = NetworkMessage::NodeStateQuery;
                    if let Err(e) = send_framed_message_cli(&mut stream, &query_msg).await {
                        eprintln!("CLI: Error sending query: {}", e);
                        return Ok(());
                    }
                    // Use BufReader for read_framed_message_cli
                    let (reader, _) = stream.into_split();
                    let mut buf_reader = BufReader::new(reader);
                    match read_framed_message_cli(&mut buf_reader).await {
                        Ok(Some(NetworkMessage::NodeStateResponse(data))) => {
                            match bincode::deserialize::<NodeStateSummary>(&data) {
                                Ok(summary) => {
                                    println!("CLI: Node State Response from {}:", summary.node_id);
                                    println!("  Height: {}", summary.height);
                                    println!("  Last Block Hash: {}", summary.last_block_hash);
                                }
                                Err(e) => eprintln!("CLI: Error deserializing node state response: {}", e),
                            }
                        }
                        Ok(Some(other_msg)) => eprintln!("CLI: Received unexpected message type: {:?}", other_msg),
                        Ok(None) => eprintln!("CLI: Connection closed by node before response."),
                        Err(e) => eprintln!("CLI: Error receiving state response: {}", e),
                    }
                }
                Err(e) => { eprintln!("CLI: Failed to connect to target node {}: {}", target_addr, e); }
            }
        }
    }
    Ok(())
}
