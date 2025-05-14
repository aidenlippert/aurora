// sgs_foundations/triad_web/src/network_behaviour.rs
use libp2p::{
    gossipsub::{self, IdentTopic as Topic},
    identify, kad, mdns, ping,
    swarm::NetworkBehaviour,
};
use serde::{Deserialize, Serialize};

#[derive(NetworkBehaviour)]
#[behaviour(to_swarm = "AuroraEvent")]
pub struct AuroraNetBehaviour {
    pub gossipsub: gossipsub::Behaviour,
    pub kademlia: kad::Behaviour<kad::store::MemoryStore>,
    pub mdns: mdns::tokio::Behaviour,
    pub ping: ping::Behaviour,
    pub identify: identify::Behaviour,
}

#[derive(Debug)]
pub enum AuroraEvent {
    Gossipsub(gossipsub::Event),
    Kademlia(kad::Event),
    Mdns(mdns::Event),
    Ping(ping::Event),
    Identify(identify::Event),
}

impl From<gossipsub::Event> for AuroraEvent { fn from(event: gossipsub::Event) -> Self { AuroraEvent::Gossipsub(event) } }
impl From<kad::Event> for AuroraEvent { fn from(event: kad::Event) -> Self { AuroraEvent::Kademlia(event) } }
impl From<mdns::Event> for AuroraEvent { fn from(event: mdns::Event) -> Self { AuroraEvent::Mdns(event) } }
impl From<ping::Event> for AuroraEvent { fn from(event: ping::Event) -> Self { AuroraEvent::Ping(event) } }
impl From<identify::Event> for AuroraEvent { fn from(event: identify::Event) -> Self { AuroraEvent::Identify(event) } }

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq, Hash)]
pub enum AuroraTopic {
    Blocks,
    Transactions,
    Consensus, // General consensus messages, can include sync and attestations for now
    Attestations, // Or a dedicated topic for attestations
}

impl AuroraTopic {
    pub fn to_topic(&self) -> Topic {
        Topic::new(self.topic_string())
    }

    pub fn topic_string(&self) -> String {
        match self {
            AuroraTopic::Blocks => "aurora-blocks-v1".to_string(),
            AuroraTopic::Transactions => "aurora-transactions-v1".to_string(),
            AuroraTopic::Consensus => "aurora-consensus-v1".to_string(),
            AuroraTopic::Attestations => "aurora-attestations-v1".to_string(), // New topic
        }
    }
}