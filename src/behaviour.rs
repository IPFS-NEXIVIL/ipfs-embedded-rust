use libp2p::autonat;
use libp2p::identify;
use libp2p::kad::{record::store::MemoryStore, Kademlia, KademliaConfig, Mode};
use libp2p::ping;
use libp2p::relay;
// use libp2p::swarm::behaviour::toggle::Toggle;
use libp2p::gossipsub as p2pgossipsub;
use libp2p::mdns as p2pmdns;
use libp2p::{identity, swarm::NetworkBehaviour, Multiaddr, PeerId};
use std::str::FromStr;
use std::time::Duration;

const BOOTNODES: [&str; 4] = [
    "QmNnooDu7bfjPFoTZYxMNLWUQJyrVwtbZg5gBMjTezGAJN",
    "QmQCU2EcMqAqQPR2i9bChDtGNJchTbq5TbXJJ16u19uLTa",
    "QmbLHAnMoJPWSCR5Zhtx6BHJX9KiKNN6tpvbUcqanj75Nb",
    "QmcZf59bWwK5XFi76CZX8cbJ4BhTzzA3gU1ZjYZcYW3dwt",
];

// #[behaviour(to_swarm = "Event", event_process = false)]
#[derive(NetworkBehaviour)]
pub struct Behaviour {
    pub gossipsub: p2pgossipsub::Behaviour,
    mdns: p2pmdns::async_io::Behaviour,
    relay: relay::Behaviour,
    ping: ping::Behaviour,
    identify: identify::Behaviour,
    pub kademlia: Kademlia<MemoryStore>,
    autonat: autonat::Behaviour,
}

impl Behaviour {
    pub fn new(
        pub_key: identity::PublicKey,
        gossipsub: p2pgossipsub::Behaviour,
        mdns: p2pmdns::Behaviour<p2pmdns::async_io::AsyncIo>,
    ) -> Self {
        let mut kademlia_config = KademliaConfig::default();
        // Instantly remove records and provider records.
        //
        // TODO: Replace hack with option to disable both.
        // kademlia_config.set_record_ttl(Some(Duration::from_secs(0)));
        // kademlia_config.set_provider_record_ttl(Some(Duration::from_secs(0)));
        kademlia_config.set_query_timeout(Duration::from_secs(5 * 60));
        let mut kademlia = Kademlia::with_config(
            pub_key.to_peer_id(),
            MemoryStore::new(pub_key.to_peer_id()),
            kademlia_config,
        );
        kademlia.set_mode(Some(Mode::Server));
        let bootaddr = Multiaddr::from_str("/dnsaddr/bootstrap.libp2p.io").unwrap();
        for peer in &BOOTNODES {
            kademlia.add_address(&PeerId::from_str(peer).unwrap(), bootaddr.clone());
        }
        // kademlia.add_address(
        //     &PeerId::from_str("12D3KooWKpCMEGZ2fjmGeKxX6CZ6MvbpdAC898go5UJGQPAnL18A").unwrap(),
        //     Multiaddr::from_str("/ip4/127.0.0.1/tcp/4001").unwrap(),
        // );
        kademlia.bootstrap().unwrap();

        let autonat = autonat::Behaviour::new(PeerId::from(pub_key.clone()), Default::default());

        Self {
            mdns,
            gossipsub,
            relay: relay::Behaviour::new(PeerId::from(pub_key.clone()), Default::default()),
            ping: ping::Behaviour::new(ping::Config::new()),
            identify: identify::Behaviour::new(
                identify::Config::new("ipfs/0.1.0".to_string(), pub_key).with_agent_version(
                    format!("rust-libp2p-server/{}", env!("CARGO_PKG_VERSION")),
                ),
            ),
            kademlia,
            autonat,
        }
    }
}

// #[derive(Debug)]
// pub enum Event {
//     Ping(ping::Event),
//     Identify(Box<identify::Event>),
//     Relay(relay::Event),
//     Kademlia(KademliaEvent),
//     Autonat(autonat::Event),
// }

// impl From<ping::Event> for Event {
//     fn from(event: ping::Event) -> Self {
//         Event::Ping(event)
//     }
// }

// impl From<identify::Event> for Event {
//     fn from(event: identify::Event) -> Self {
//         Event::Identify(Box::new(event))
//     }
// }

// impl From<relay::Event> for Event {
//     fn from(event: relay::Event) -> Self {
//         Event::Relay(event)
//     }
// }

// impl From<KademliaEvent> for Event {
//     fn from(event: KademliaEvent) -> Self {
//         Event::Kademlia(event)
//     }
// }

// impl From<autonat::Event> for Event {
//     fn from(event: autonat::Event) -> Self {
//         Event::Autonat(event)
//     }
// }
