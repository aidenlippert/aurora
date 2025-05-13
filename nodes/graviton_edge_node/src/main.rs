use ecliptic_concordance::{
    sequencer_create_block, submit_transaction_payload, validate_and_apply_block,
    get_current_state_summary, Block, TransactionPayload
};
use triad_web::{NetworkMessage, P2PService, get_graviton_edge_network_interface};
use std::sync::mpsc::{channel, Receiver};
use std::thread;
use std::time::Duration;
use serde_json;
use clap::Parser;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(long, help = "Node ID for this instance")]
    node_id: String,

    #[clap(long, help = "Port for this node (conceptual for InMemoryP2P)")]
    port: u16,

    #[clap(long, help = "Designate this node as the initial sequencer")]
    is_sequencer: bool,

    #[clap(long, value_delimiter = ',', help = "Comma-separated list of peer IDs to connect to (for InMemoryP2P)")]
    peers: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    println!("[GravitonEdgeNode:{}] Starting node. Sequencer: {}", args.node_id, args.is_sequencer);

    let p2p_network = get_graviton_edge_network_interface();
    let (msg_sender, msg_receiver): (std::sync::mpsc::Sender<NetworkMessage>, Receiver<NetworkMessage>) = channel();
    
    p2p_network.register_peer(args.node_id.clone(), msg_sender);
    println!("[GravitonEdgeNode:{}] Registered with InMemoryP2PNetwork.", args.node_id);

    // If this node is the sequencer, start producing blocks periodically
    if args.is_sequencer {
        let node_id_clone = args.node_id.clone();
        let p2p_broadcast = p2p_network.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(10)).await; // Create block every 10s
                match sequencer_create_block(&node_id_clone) {
                    Ok(new_block) => {
                        println!("[GravitonEdgeNode:{}:Sequencer] Created new block (Height: {}). Broadcasting...", node_id_clone, new_block.height);
                        match serde_json::to_vec(&new_block) {
                            Ok(serialized_block) => {
                                p2p_broadcast.broadcast(NetworkMessage::BlockProposal(serialized_block));
                            }
                            Err(e) => eprintln!("[GravitonEdgeNode:{}:Sequencer] Error serializing block: {}", node_id_clone, e),
                        }
                    }
                    Err(e) => {
                        // This can happen if no transactions are pending, which is fine.
                        // In a real system, sequencers might create empty blocks.
                        // println!("[GravitonEdgeNode:{}:Sequencer] Error creating block: {}", node_id_clone, e);
                    }
                }
            }
        });
    }

    // Main loop for receiving and processing messages
    println!("[GravitonEdgeNode:{}] Listening for messages...", args.node_id);
    loop {
        match msg_receiver.recv() { // Blocking receive
            Ok(network_message) => {
                println!("[GravitonEdgeNode:{}] Received message: {:?}", args.node_id, network_message);
                match network_message {
                    NetworkMessage::BlockProposal(serialized_block) => {
                        match serde_json::from_slice::<Block>(&serialized_block) {
                            Ok(block) => {
                                if block.proposer_id == args.node_id {
                                    // Node doesn't need to validate its own blocks in this simple model
                                    // but would if it could receive them via broadcast too
                                    println!("[GravitonEdgeNode:{}] Received own proposed block (Height {}), ignoring for validation.", args.node_id, block.height);
                                    continue;
                                }
                                match validate_and_apply_block(&block) {
                                    Ok(()) => {
                                        println!("[GravitonEdgeNode:{}] Validated and applied block (Height: {}) from {}", args.node_id, block.height, block.proposer_id);
                                        // Persist block to file (simple append)
                                        // In a real setup, use a proper DB like sled or RocksDB
                                        let block_file_name = format!("{}_blockchain.jsonl", args.node_id);
                                        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(block_file_name).unwrap();
                                        use std::io::Write;
                                        writeln!(file, "{}", serde_json::to_string(&block).unwrap()).unwrap();

                                    }
                                    Err(e) => eprintln!("[GravitonEdgeNode:{}] Invalid block received (Height: {}): {}", args.node_id, block.height, e),
                                }
                            }
                            Err(e) => eprintln!("[GravitonEdgeNode:{}] Error deserializing received block: {}", args.node_id, e),
                        }
                    }
                    NetworkMessage::Transaction(serialized_tx_payload) => {
                        // In this simple sequencer model, only the sequencer processes new tx directly into blocks.
                        // Other nodes would get them via blocks.
                        // If this node IS the sequencer, add to its mempool.
                        if args.is_sequencer {
                             match serde_json::from_slice::<TransactionPayload>(&serialized_tx_payload) {
                                Ok(tx_payload) => {
                                    match submit_transaction_payload(tx_payload.data) {
                                        Ok(tx_id) => println!("[GravitonEdgeNode:{}:Sequencer] Added payload to mempool, TxID: {}", args.node_id, tx_id),
                                        Err(e) => eprintln!("[GravitonEdgeNode:{}:Sequencer] Error submitting payload: {}", args.node_id, e),
                                    }
                                }
                                Err(e) => eprintln!("[GravitonEdgeNode:{}:Sequencer] Error deserializing tx payload: {}", args.node_id, e),
                             }
                        }
                    }
                    _ => {
                        println!("[GravitonEdgeNode:{}] Received other P2P message type (ignoring).", args.node_id);
                    }
                }
            }
            Err(e) => {
                eprintln!("[GravitonEdgeNode:{}] Error receiving message: {}. Shutting down.", args.node_id, e);
                break; // Exit loop on channel error
            }
        }
    }
    p2p_network.unregister_peer(&args.node_id);
    Ok(())
}
