use clap::{Parser, Subcommand};
use triad_web::{NetworkMessage, P2PService, initialize_tcp_p2p_service}; // Using initialize_tcp_p2p_service conceptually for sending
use ecliptic_concordance::{TransactionPayload, get_current_state_summary}; // For struct and query
use std::net::SocketAddr;
use tokio::net::TcpStream; // For direct TCP send
use tokio::io::AsyncWriteExt; // For write_u32, write_all
use serde_json;
use bincode; // For sending messages

// Copied from triad_web/src/tcp_p2p.rs for CLI to send in same format
async fn send_framed_message_cli<W: AsyncWriteExt + Unpin>(stream: &mut W, msg: &NetworkMessage) -> std::io::Result<()> {
    let encoded = bincode::serialize(msg).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    let len = encoded.len() as u32;
    stream.write_u32(len).await?;
    stream.write_all(&encoded).await?;
    stream.flush().await?;
    Ok(())
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
    QueryConsensusState,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::SubmitTx { target_addr, data } => {
            let socket_addr: SocketAddr = target_addr.parse()?;
            println!("CLI: Submitting transaction with data '{}' to node at {}", data, socket_addr);
            
            let tx_payload_struct = TransactionPayload {
                data: data.into_bytes(),
            };
            // For sending, we wrap it in NetworkMessage::Transaction.
            // The node will receive this and, if it's the sequencer, add it to its mempool.
            let serialized_payload_for_network_message = serde_json::to_vec(&tx_payload_struct)?;
            let network_msg = NetworkMessage::Transaction(serialized_payload_for_network_message);

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
        Commands::QueryConsensusState {} => {
            println!("CLI: Querying current global consensus state (mock from ecliptic_concordance)...");
            let (height, hash) = get_current_state_summary(); // Still uses the static shared state
            println!("  Current Consensus State (as per shared static):");
            println!("  Height: {}", height);
            println!("  Last Block Hash: {}", hash);
            println!("  Note: For node-specific chain, check its _blockchain.jsonl file in its data_dir.");
        }
    }
    Ok(())
}
