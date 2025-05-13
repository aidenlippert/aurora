use crate::network_behaviour::{AuroraNetBehaviour, AuroraEvent, AuroraTopic};
use crate::{NetworkMessage, AppP2PEvent, message_summary};

use libp2p::{
    gossipsub::{self, MessageAuthenticity, ValidationMode, MessageId, ConfigBuilder as GossipsubConfigBuilder, PublishError},
    identity::{Keypair},
    kad::{self, store::MemoryStore, QueryId, Config as KademliaConfig, Event as KadEvent, QueryResult as KadQueryResult, BootstrapOk as KadBootstrapOk},
    mdns, noise, ping, identify,
    swarm::{SwarmEvent, dial_opts::DialOpts, ConnectionId, NetworkInfo},
    tcp, yamux, PeerId, Swarm, Transport, Multiaddr,
    core::transport::ListenerId as CoreListenerId,
    SwarmBuilder,
};
use std::collections::{hash_map::DefaultHasher, HashSet};
use std::hash::{Hash, Hasher};
use std::time::Duration;
use std::error::Error;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex as TokioMutex};
use futures::StreamExt;
use log::{info, warn, error, debug, trace};
use std::path::PathBuf;
use std::fs;

const KEYPAIR_FILENAME: &str = "node_key.pk8";

fn load_or_generate_keypair(key_path: &PathBuf) -> Result<Keypair, Box<dyn Error + Send + Sync>> {
    if key_path.exists() {
        let mut key_bytes = fs::read(key_path)?; // Make key_bytes mutable
        // Pass a mutable slice to try_from_bytes
        match libp2p::identity::ed25519::Keypair::try_from_bytes(&mut key_bytes) { // MODIFIED: &mut key_bytes
            Ok(ed_kp) => {
                info!("Loaded existing Ed25519 keypair from {:?}", key_path);
                Ok(Keypair::from(ed_kp))
            }
            Err(e) => { // Catch the specific error type if possible, or just log broadly
                warn!("Failed to decode keypair from {:?} (error: {:?}), generating new one.", key_path, e);
                generate_and_save_keypair(key_path)
            }
        }
    } else {
        generate_and_save_keypair(key_path)
    }
}

fn generate_and_save_keypair(key_path: &PathBuf) -> Result<Keypair, Box<dyn Error + Send + Sync>> {
    let ed_kp = libp2p::identity::ed25519::Keypair::generate();
    // For ed25519-dalek, to_bytes() gives the secret key bytes.
    // libp2p's ed25519::Keypair::to_bytes() method serializes the keypair (secret + public)
    // in a way that from_bytes can later decode.
    let key_pair_bytes_to_save = ed_kp.to_bytes(); // This should be the correct method for libp2p's own type
    
    if let Some(parent_dir) = key_path.parent() {
        fs::create_dir_all(parent_dir)?;
    }
    fs::write(key_path, &key_pair_bytes_to_save)?; // Save these specific bytes
    info!("Generated and saved new Ed25519 keypair to {:?}", key_path);
    Ok(Keypair::from(ed_kp))
}


#[async_trait::async_trait]
pub trait P2PService: Send + Sync + 'static {
    fn get_peer_id(&self) -> PeerId;
    async fn publish(&self, topic: AuroraTopic, message: NetworkMessage) -> Result<MessageId, String>;
    async fn listen_on(&self, addr: Multiaddr) -> Result<(), Box<dyn Error + Send + Sync>>;
    async fn dial(&self, addr: Multiaddr) -> Result<(), Box<dyn Error + Send + Sync>>;
}

#[derive(Debug)]
pub enum P2PEvent {
    SwarmBehaviour(AuroraEvent),
    ConnectionEstablished { 
        peer_id: PeerId, 
        endpoint: libp2p::core::ConnectedPoint, 
        num_established: u32, 
        connection_id: ConnectionId,
        established_in: Duration, 
        concurrent_dial_errors: Option<Vec<(Multiaddr, libp2p::swarm::DialError)>>, 
    },
    ConnectionClosed { 
        peer_id: PeerId, 
        endpoint: libp2p::core::ConnectedPoint, 
        num_established: u32, 
        cause: Option<libp2p::swarm::ConnectionError>, 
        connection_id: ConnectionId 
    },
    IncomingConnection { 
        connection_id: ConnectionId, 
        local_addr: Multiaddr, 
        send_back_addr: Multiaddr 
    },
    IncomingConnectionError { 
        connection_id: ConnectionId, 
        local_addr: Multiaddr, 
        send_back_addr: Multiaddr, 
        error: libp2p::swarm::ListenError 
    },
    OutgoingConnectionError { 
        peer_id: Option<PeerId>, 
        error: libp2p::swarm::DialError, 
        connection_id: ConnectionId 
    },
    NewListenAddr { 
        listener_id: CoreListenerId, 
        address: Multiaddr 
    },
    ExpiredListenAddr { 
        listener_id: CoreListenerId, 
        address: Multiaddr 
    },
    ListenerClosed { 
        listener_id: CoreListenerId, 
        addresses: Vec<Multiaddr>, 
        reason: Result<(), std::io::Error> 
    },
    ListenerError { 
        listener_id: CoreListenerId, 
        error: std::io::Error 
    },
    Dialing { 
        peer_id: Option<PeerId>, 
        connection_id: ConnectionId,
    },
    OtherSwarmEvent(String),
}


pub struct Libp2pService {
    peer_id: PeerId,
    swarm: Arc<TokioMutex<Swarm<AuroraNetBehaviour>>>,
    app_event_tx: mpsc::Sender<AppP2PEvent>,
    node_id_log_prefix: String,
}

impl Libp2pService {
    pub async fn new(
        node_id_for_logs: String,
        data_dir: PathBuf, 
        listen_multiaddrs: Vec<Multiaddr>,
        bootstrap_peers: Vec<Multiaddr>,
    ) -> Result<(Arc<Self>, mpsc::Receiver<AppP2PEvent>), Box<dyn Error + Send + Sync>> {
        
        let key_file_path = data_dir.join(KEYPAIR_FILENAME);
        let local_keypair = load_or_generate_keypair(&key_file_path)?; 
        
        let local_peer_id = PeerId::from(local_keypair.public());
        let log_prefix = format!("[Libp2p:{}]", node_id_for_logs);
        info!("{} Local Peer ID: {}", log_prefix, local_peer_id);

        let transport = tcp::tokio::Transport::new(tcp::Config::default().nodelay(true))
            .upgrade(libp2p::core::upgrade::Version::V1Lazy)
            .authenticate(noise::Config::new(&local_keypair)?) 
            .multiplex(yamux::Config::default())
            .timeout(Duration::from_secs(20))
            .boxed();

        let gossipsub_config = GossipsubConfigBuilder::default()
            .heartbeat_interval(Duration::from_secs(10))
            .validation_mode(ValidationMode::Strict) 
            .message_id_fn(|message: &gossipsub::Message| {
                let mut s = DefaultHasher::new();
                message.data.hash(&mut s);
                message.topic.hash(&mut s);
                gossipsub::MessageId::from(s.finish().to_string())
            })
            .mesh_n_low(0) 
            .mesh_outbound_min(0) 
            .build()?;
        
        let gossipsub_behaviour = gossipsub::Behaviour::new(
            MessageAuthenticity::Signed(local_keypair.clone()), 
            gossipsub_config,
        )?;

        let kad_config = KademliaConfig::default();
        let store = MemoryStore::new(local_peer_id);
        let kad_behaviour = kad::Behaviour::with_config(local_peer_id, store, kad_config);
        
        let mdns_behaviour = mdns::tokio::Behaviour::new(mdns::Config::default(), local_peer_id)?;

        let identify_config = identify::Config::new("aurora/0.1.0".to_string(), local_keypair.public()) 
            .with_agent_version(format!("aurora-node/{}", env!("CARGO_PKG_VERSION")));
        let identify_behaviour = identify::Behaviour::new(identify_config);

        let behaviour = AuroraNetBehaviour {
            gossipsub: gossipsub_behaviour,
            kademlia: kad_behaviour,
            mdns: mdns_behaviour,
            ping: ping::Behaviour::default(),
            identify: identify_behaviour,
        };

        let swarm = SwarmBuilder::with_existing_identity(local_keypair) 
            .with_tokio()
            .with_tcp(
                tcp::Config::default().nodelay(true),
                noise::Config::new,
                yamux::Config::default,
            )?
            .with_behaviour(|_key| Ok(behaviour) )? 
            .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(60)))
            .build();
        
        let swarm_arc = Arc::new(TokioMutex::new(swarm));

        let (app_event_tx, app_event_rx) = mpsc::channel(128);

        let service = Arc::new(Self {
            peer_id: local_peer_id,
            swarm: swarm_arc.clone(),
            app_event_tx: app_event_tx.clone(),
            node_id_log_prefix: log_prefix.clone(),
        });

        let event_loop_log_prefix = log_prefix.clone();
        tokio::spawn(async move {
            let mut locked_swarm = swarm_arc.lock().await;
            
            for addr in listen_multiaddrs {
                if let Err(e) = locked_swarm.listen_on(addr.clone()) {
                    error!("{} Error listening on {}: {}", event_loop_log_prefix, addr, e);
                } else {
                    info!("{} Listening on {}", event_loop_log_prefix, addr);
                }
            }

            for peer_addr in bootstrap_peers.iter() {
                info!("{} Dialing bootstrap peer: {}", event_loop_log_prefix, peer_addr);
                if let Some(peer_id_from_addr) = peer_addr.iter().find_map(|p| {
                    if let libp2p::multiaddr::Protocol::P2p(peer_id_val) = p {
                        Some(peer_id_val)
                    } else {
                        None
                    }
                }) {
                     locked_swarm.behaviour_mut().kademlia.add_address(&peer_id_from_addr, peer_addr.clone());
                     debug!("{} Added bootstrap peer {} to Kademlia table", event_loop_log_prefix, peer_id_from_addr);
                } else {
                    warn!("{} Could not extract PeerId from bootstrap Multiaddr: {}", event_loop_log_prefix, peer_addr);
                }

                if let Err(e) = locked_swarm.dial(peer_addr.clone()) {
                     error!("{} Error dialing bootstrap peer {}: {}", event_loop_log_prefix, peer_addr, e);
                }
            }
            
            let topics_to_subscribe = vec![AuroraTopic::Blocks, AuroraTopic::Transactions, AuroraTopic::Consensus];
            for topic_enum in topics_to_subscribe {
                 if let Err(e) = locked_swarm.behaviour_mut().gossipsub.subscribe(&topic_enum.to_topic()) {
                    warn!("{} Failed to subscribe to {} topic: {:?}", event_loop_log_prefix, topic_enum.topic_string(), e);
                } else {
                    info!("{} Subscribed to topic: {}", event_loop_log_prefix, topic_enum.topic_string());
                }
            }

            if !bootstrap_peers.is_empty() {
                 info!("{} Starting Kademlia bootstrap...", event_loop_log_prefix);
                 match locked_swarm.behaviour_mut().kademlia.bootstrap() {
                    Ok(_query_id) => info!("{} Kademlia bootstrap process initiated.", event_loop_log_prefix),
                    Err(e) => warn!("{} Kademlia bootstrap failed to initiate: {:?}", event_loop_log_prefix, e),
                }
            } else {
                info!("{} Skipping Kademlia bootstrap as no bootstrap peers are configured.", event_loop_log_prefix);
            }
            drop(locked_swarm);

            loop {
                let mut locked_swarm_loop = swarm_arc.lock().await; 
                
                tokio::select! {
                    biased; 
                    event = locked_swarm_loop.select_next_some() => {
                        let log_p = &event_loop_log_prefix; 
                        match event {
                            SwarmEvent::Behaviour(AuroraEvent::Gossipsub(gossip_event)) => {
                                match gossip_event {
                                    gossipsub::Event::Message {
                                        propagation_source,
                                        message_id,
                                        message,
                                    } => {
                                        trace!("{} Gossipsub: Received raw message: ID {:?}, From {:?}, Topic: {:?}", log_p, message_id, message.source, message.topic);
                                        match bincode::deserialize::<NetworkMessage>(&message.data) {
                                            Ok(app_message) => {
                                                debug!("{} Gossipsub: Deserialized App Message: {}", log_p, message_summary(&app_message));
                                                if app_event_tx.send(AppP2PEvent::GossipsubMessage {
                                                    source: message.source.unwrap_or(propagation_source),
                                                    topic_hash: message.topic.clone(),
                                                    message: app_message,
                                                }).await.is_err() {
                                                    error!("{} Failed to send Gossipsub message to app layer: receiver dropped.", log_p);
                                                }
                                            }
                                            Err(e) => {
                                                warn!("{} Gossipsub: Failed to deserialize NetworkMessage: {:?}. Data: {:02X?}", log_p, e, message.data);
                                            }
                                        }
                                    }
                                    _ => { trace!("{} Unhandled Gossipsub event: {:?}", log_p, gossip_event); }
                                }
                            }
                            SwarmEvent::Behaviour(AuroraEvent::Kademlia(kad_event)) => {
                                match kad_event {
                                    KadEvent::OutboundQueryProgressed { result: KadQueryResult::Bootstrap(Ok(KadBootstrapOk{peer, num_remaining})), ..} => {
                                         info!("{} Kademlia: Bootstrapped with peer: {:?}, Num remaining: {:?}", log_p, peer, num_remaining);
                                    }
                                    KadEvent::OutboundQueryProgressed { result: KadQueryResult::Bootstrap(Err(err)), .. } => {
                                        warn!("{} Kademlia: Bootstrap error: {:?}", log_p, err);
                                    }
                                    KadEvent::RoutingUpdated { peer, addresses, old_peer, .. } => {
                                        if old_peer.is_none() {
                                            debug!("{} Kademlia: New routing entry for PeerId: {:?}, Addresses: {:?}", log_p, peer, addresses);
                                        } else {
                                             debug!("{} Kademlia: Routing entry updated for PeerId: {:?}, New Addresses: {:?}, Old PeerId was: {:?}", log_p, peer, addresses, old_peer);
                                        }
                                    }
                                    _ => { trace!("{} Kademlia event: {:?}", log_p, kad_event); }
                                }
                            }
                            SwarmEvent::Behaviour(AuroraEvent::Mdns(mdns_event)) => {
                                match mdns_event {
                                    mdns::Event::Discovered(list) => {
                                        for (peer_id, multiaddr) in list {
                                            info!("{} mDNS: Discovered peer {:?} at {}", log_p, peer_id, multiaddr);
                                            locked_swarm_loop.behaviour_mut().kademlia.add_address(&peer_id, multiaddr.clone());
                                        }
                                    }
                                    mdns::Event::Expired(list) => {
                                        for (peer_id, _multiaddr) in list {
                                            debug!("{} mDNS: Expired peer {:?}", log_p, peer_id);
                                        }
                                    }
                                }
                            }
                            SwarmEvent::Behaviour(AuroraEvent::Ping(ping_event)) => {
                                trace!("{} Ping event: {:?}", log_p, ping_event);
                            }
                             SwarmEvent::Behaviour(AuroraEvent::Identify(identify_event)) => {
                                match identify_event {
                                    identify::Event::Received { peer_id, info } => {
                                        info!("{} Identify: Received info from {:?}: Agent: {}, Protocols: {:?}, ListenAddrs: {:?}", log_p, peer_id, info.agent_version, info.protocols, info.listen_addrs);
                                        for addr in info.listen_addrs {
                                            locked_swarm_loop.behaviour_mut().kademlia.add_address(&peer_id, addr);
                                        }
                                    }
                                    _ => { trace!("{} Unhandled Identify event: {:?}", log_p, identify_event); }
                                }
                            }
                            SwarmEvent::NewListenAddr { address, listener_id } => {
                                info!("{} Swarm: New listen address: {} (Listener ID: {:?})", log_p, address, listener_id);
                            }
                            SwarmEvent::ConnectionEstablished { 
                                peer_id, 
                                endpoint, 
                                num_established, 
                                connection_id,
                                concurrent_dial_errors, 
                                established_in, 
                            } => { 
                                info!("{} Swarm: Connection {} established with peer: {:?} via {:?} (Total: {}, In: {:?}, DialErrors: {:?})", 
                                    log_p, connection_id, peer_id, endpoint.get_remote_address(), num_established, established_in, concurrent_dial_errors.map(|v| v.len()));
                                 if app_event_tx.send(AppP2PEvent::PeerConnected(peer_id)).await.is_err() {
                                    error!("{} Failed to send PeerConnected to app layer.", log_p);
                                }
                            }
                            SwarmEvent::ConnectionClosed { peer_id, cause, num_established, connection_id, endpoint } => {
                                info!("{} Swarm: Connection {} closed with peer: {:?}, Cause: {:?} (Total: {})", log_p, connection_id, peer_id, cause, num_established);
                                if app_event_tx.send(AppP2PEvent::PeerDisconnected(peer_id)).await.is_err() {
                                     error!("{} Failed to send PeerDisconnected to app layer.", log_p);
                                }
                                locked_swarm_loop.behaviour_mut().kademlia.remove_peer(&peer_id);
                            }
                            SwarmEvent::IncomingConnection { local_addr, send_back_addr, connection_id } => { 
                                debug!("{} Swarm: Incoming connection {} on {} from {}", log_p, connection_id, local_addr, send_back_addr);
                            }
                            SwarmEvent::IncomingConnectionError { local_addr, send_back_addr, error, connection_id } => { 
                                warn!("{} Swarm: Incoming connection error {} on {} from {}: {:?}", log_p, connection_id, local_addr, send_back_addr, error);
                            }
                            SwarmEvent::OutgoingConnectionError { peer_id, error, connection_id } => {
                                warn!("{} Swarm: Outgoing connection error {} to {:?}: {:?}", log_p, connection_id, peer_id, error);
                            }
                            SwarmEvent::ExpiredListenAddr { listener_id, address } => {
                                info!("{} Swarm: Expired listen address {} (Listener ID: {:?})", log_p, address, listener_id);
                            }
                            SwarmEvent::ListenerClosed { listener_id, addresses, reason } => {
                                info!("{} Swarm: Listener {:?} closed for addresses {:?}, Reason: {:?}", log_p, listener_id, addresses, reason);
                            }
                            SwarmEvent::ListenerError { listener_id, error } => {
                                error!("{} Swarm: Listener {:?} error: {:?}", log_p, listener_id, error);
                            }
                            SwarmEvent::Dialing { peer_id, connection_id } => { 
                                trace!("{} Swarm: Dialing (PeerId: {:?}, Connection ID: {})", log_p, peer_id, connection_id);
                            }
                            other_event => {
                                trace!("{} Swarm: Other event: {:?}", log_p, other_event);
                            }
                        }
                    }
                    _ = tokio::time::sleep(Duration::from_secs(30)) => { 
                        let network_info = locked_swarm_loop.network_info();
                        let num_connected_peers = network_info.num_peers();
                        
                        let num_gossip_peers = locked_swarm_loop.behaviour().gossipsub.all_peers().count();
                        let mut known_kad_peers = std::collections::HashSet::new();
                        for bucket in locked_swarm_loop.behaviour_mut().kademlia.kbuckets() {
                            for entry in bucket.iter() {
                                known_kad_peers.insert(*entry.node.key.preimage());
                            }
                        }
                        debug!("{} Periodic: Total connected: {}, Gossipsub mesh: {}, Kademlia known: {}", 
                               event_loop_log_prefix, num_connected_peers, num_gossip_peers, known_kad_peers.len());

                        if num_connected_peers == 0 && !bootstrap_peers.is_empty() {
                            warn!("{} No connected peers and bootstrap peers configured. Re-attempting Kademlia bootstrap.", event_loop_log_prefix);
                            match locked_swarm_loop.behaviour_mut().kademlia.bootstrap() {
                                Ok(_) => info!("{} Kademlia bootstrap re-initiated.", event_loop_log_prefix),
                                Err(e) => warn!("{} Kademlia bootstrap re-initiation failed: {:?}", event_loop_log_prefix, e),
                            }
                        } else if num_connected_peers > 0 && known_kad_peers.len() < (num_connected_peers as usize / 2 + 1).max(1) { 
                             info!("{} Kademlia known peers seem low ({}) compared to connections ({}). Attempting random walk.", event_loop_log_prefix, known_kad_peers.len(), num_connected_peers);
                             locked_swarm_loop.behaviour_mut().kademlia.get_closest_peers(PeerId::random());
                        }
                    }
                }
                drop(locked_swarm_loop);
            }
        });

        Ok((service, app_event_rx))
    }
}

#[async_trait::async_trait]
impl P2PService for Libp2pService {
    fn get_peer_id(&self) -> PeerId {
        self.peer_id
    }

    async fn publish(&self, topic: AuroraTopic, message: NetworkMessage) -> Result<MessageId, String> {
        let mut swarm = self.swarm.lock().await;
        let topic_ident = topic.to_topic();
        
        match bincode::serialize(&message) {
            Ok(data) => {
                match swarm.behaviour_mut().gossipsub.publish(topic_ident.clone(), data) {
                    Ok(message_id) => {
                        debug!("{} Successfully published message ID {:?} to topic '{}'", self.node_id_log_prefix, message_id, topic.topic_string());
                        Ok(message_id)
                    }
                    Err(e @ PublishError::InsufficientPeers) => { 
                        warn!("{} Failed to publish to topic '{}' due to InsufficientPeers. Message not sent to mesh.", self.node_id_log_prefix, topic.topic_string());
                        Err(format!("Gossipsub publish error: {:?}", e))
                    }
                    Err(e) => {
                        error!("{} Failed to publish to topic '{}': {:?}", self.node_id_log_prefix, topic.topic_string(), e);
                        Err(format!("Gossipsub publish error: {:?}", e))
                    }
                }
            }
            Err(e) => {
                error!("{} Failed to serialize message for topic '{}': {:?}", self.node_id_log_prefix, topic.topic_string(), e);
                Err(format!("Serialization error: {:?}", e))
            }
        }
    }
    
    async fn listen_on(&self, addr: Multiaddr) -> Result<(), Box<dyn Error + Send + Sync>> {
        let mut swarm = self.swarm.lock().await;
        swarm.listen_on(addr)?;
        Ok(())
    }

    async fn dial(&self, addr: Multiaddr) -> Result<(), Box<dyn Error + Send + Sync>> {
        let mut swarm = self.swarm.lock().await;
        swarm.dial(addr)?;
        Ok(())
    }
}