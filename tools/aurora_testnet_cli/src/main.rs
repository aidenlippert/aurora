use clap::{Parser, Subcommand};
use triad_web::{NetworkMessage, P2PService, get_graviton_edge_network_interface};
use ecliptic_concordance::TransactionPayload;
use std::collections::HashMap;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Cli {
    #[clap(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Submit a transaction payload to a node (typically the sequencer)
    SubmitTx {
        #[clap(long, help = "Node ID of the target node (sequencer)")]
        target_node_id: String,
        #[clap(long, help = "Data payload for the transaction (simple string for now)")]
        data: String,
    },
    /// Query the status of a node
    QueryNode {
        #[clap(long, help = "Node ID of the target node")]
        node_id: String,
        // For this query, we won't send a message TO the node via P2P,
        // as the node's state (like block height) isn't directly queryable via the simple P2P mock.
        // We'll use the shared CONSENSUS_STATE from ecliptic_concordance for this CLI demo.
        // A real CLI would make an RPC call or P2P request.
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let p2p_network = get_graviton_edge_network_interface(); // Get the shared P2P interface

    match cli.command {
        Commands::SubmitTx { target_node_id, data } => {
            println!("CLI: Submitting transaction with data '{}' to node '{}'", data, target_node_id);
            let tx_payload_struct = TransactionPayload {
                data: data.into_bytes(),
            };
            let serialized_payload = serde_json::to_vec(&tx_payload_struct)?;
            
            match p2p_network.send_to_peer(&target_node_id, NetworkMessage::Transaction(serialized_payload)) {
                Ok(()) => println!("CLI: Transaction payload sent successfully to {}.", target_node_id),
                Err(e) => eprintln!("CLI: Error sending transaction payload: {}", e),
            }
        }
        Commands::QueryNode { node_id } => {
            println!("CLI: Querying status for node '{}' (using shared consensus state for demo)...", node_id);
            // This is a direct call for demo purposes, as nodes don't expose a query API via InMemoryP2PNetwork yet.
            // In a real system, this would be an RPC or a specific P2P message.
            let (height, hash) = ecliptic_concordance::get_current_state_summary();
            println!("  Current Consensus State (visible to all in this mock):");
            println!("  Height: {}", height);
            println!("  Last Block Hash: {}", hash);
            println!("  Note: To get node-specific persisted state, you'd check its {node_id}_blockchain.jsonl file.");
        }
    }
    Ok(())
}
