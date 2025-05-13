use libp2p::{
    gossipsub::{self, IdentTopic as Topic}, // Using IdentTopic for simplicity
    identify, kad, mdns, ping,
    swarm::NetworkBehaviour,
};
use serde::{Deserialize, Serialize}; // For topic serialization if needed

// Define the custom NetworkBehaviour struct
// This combines all the protocols our P2P node will use.
#[derive(NetworkBehaviour)]
#[behaviour(to_swarm = "AuroraEvent")] // Custom event type emitted by this behaviour
pub struct AuroraNetBehaviour {
    pub gossipsub: gossipsub::Behaviour,
    pub kademlia: kad::Behaviour<kad::store::MemoryStore>, // Kademlia for DHT
    pub mdns: mdns::tokio::Behaviour,                      // mDNS for local discovery
    pub ping: ping::Behaviour,
    pub identify: identify::Behaviour, // For identifying peers & their capabilities
                                   // We could add request-response behaviour here if needed for direct messages
                                   // pub request_response: request_response::Behaviour<YourCodec>,
}

// Custom event type that our AuroraNetBehaviour can emit upwards to the Swarm manager
// These are events from the individual protocols within the behaviour
#[derive(Debug)]
pub enum AuroraEvent {
    Gossipsub(gossipsub::Event),
    Kademlia(kad::Event),
    Mdns(mdns::Event),
    Ping(ping::Event),
    Identify(identify::Event),
    // RequestResponse(request_response::Event<YourRequest, YourResponse>),
}

// Implement conversions from each protocol's event type to our custom AuroraEvent
impl From<gossipsub::Event> for AuroraEvent {
    fn from(event: gossipsub::Event) -> Self {
        AuroraEvent::Gossipsub(event)
    }
}

impl From<kad::Event> for AuroraEvent {
    fn from(event: kad::Event) -> Self {
        AuroraEvent::Kademlia(event)
    }
}

impl From<mdns::Event> for AuroraEvent {
    fn from(event: mdns::Event) -> Self {
        AuroraEvent::Mdns(event)
    }
}

impl From<ping::Event> for AuroraEvent {
    fn from(event: ping::Event) -> Self {
        AuroraEvent::Ping(event)
    }
}

impl From<identify::Event> for AuroraEvent {
    fn from(event: identify::Event) -> Self {
        AuroraEvent::Identify(event)
    }
}

// Topics for Gossipsub
// We'll define topics as simple strings for now.
// For production, consider more structured topic management.
#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq, Hash)]
pub enum AuroraTopic {
    Blocks,
    Transactions,
    Consensus,
    // Add more topics as needed
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
        }
    }
}