use async_std::io;
use futures::{future::Either, prelude::*, select};
use libp2p::{
    autonat,
    core::{muxing::StreamMuxerBox, transport::OrTransport, upgrade},
    development_transport, gossipsub, identify, identity,
    kad::{store::MemoryStore, Kademlia, KademliaEvent, QueryResult},
    mdns, noise, ping, relay,
    swarm::NetworkBehaviour,
    swarm::{SwarmBuilder, SwarmEvent},
    tcp, yamux, Multiaddr, PeerId, Transport,
};
use libp2p_quic as quic;
use std::collections::hash_map::DefaultHasher;
use std::error::Error;
use std::hash::{Hash, Hasher};
use std::time::Duration;

mod behaviour;

#[async_std::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let _ = env_logger::try_init();
    // Create a random PeerId
    let id_keys = identity::Keypair::generate_ed25519();
    let local_peer_id = PeerId::from(id_keys.public());
    println!("Local peer id: {local_peer_id}");

    // Set up an encrypted DNS-enabled TCP Transport over the yamux protocol.
    let tcp_transport = tcp::async_io::Transport::new(tcp::Config::default().nodelay(true))
        .upgrade(upgrade::Version::V1Lazy)
        .authenticate(noise::Config::new(&id_keys).expect("signing libp2p-noise static keypair"))
        .multiplex(yamux::Config::default())
        .timeout(std::time::Duration::from_secs(20));
    let quic_transport = quic::async_std::Transport::new(quic::Config::new(&id_keys));
    let tcp_quic_transport = OrTransport::new(quic_transport, tcp_transport).map(
        |either_output, _| match either_output {
            Either::Left((peer_id, muxer)) => (peer_id, StreamMuxerBox::new(muxer)),
            Either::Right((peer_id, muxer)) => (peer_id, StreamMuxerBox::new(muxer)),
        },
    );

    let websocket = libp2p::websocket::WsConfig::new(tcp::async_io::Transport::new(
        libp2p::tcp::Config::default(),
    ))
    .upgrade(upgrade::Version::V1)
    .authenticate(noise::Config::new(&id_keys)?)
    .multiplex(yamux::Config::default())
    .timeout(std::time::Duration::from_secs(20));

    let transport = OrTransport::new(websocket, tcp_quic_transport)
        .map(|either_output, _| match either_output {
            Either::Left((peer_id, muxer)) => (peer_id, StreamMuxerBox::new(muxer)),
            Either::Right((peer_id, muxer)) => (peer_id, StreamMuxerBox::new(muxer)),
        })
        .boxed();

    let transport2 = development_transport(id_keys.clone()).await?;

    // To content-address message, we can take the hash of message and use it as an ID.
    let message_id_fn = |message: &gossipsub::Message| {
        let mut s = DefaultHasher::new();
        message.data.hash(&mut s);
        gossipsub::MessageId::from(s.finish().to_string())
    };

    // Set a custom gossipsub configuration
    let gossipsub_config = gossipsub::ConfigBuilder::default()
        .heartbeat_interval(Duration::from_secs(10)) // This is set to aid debugging by not cluttering the log space
        .validation_mode(gossipsub::ValidationMode::Strict) // This sets the kind of message validation. The default is Strict (enforce message signing)
        .message_id_fn(message_id_fn) // content-address messages. No two messages of the same content will be propagated.
        .build()
        .expect("Valid config");

    // build a gossipsub network behaviour
    let mut gossipsub = gossipsub::Behaviour::new(
        gossipsub::MessageAuthenticity::Signed(id_keys.clone()),
        gossipsub_config,
    )
    .expect("Correct configuration");
    // Create a Gossipsub topic
    let topic = gossipsub::IdentTopic::new("fruits");
    // subscribes to our topic
    gossipsub.subscribe(&topic)?;

    // Create a Swarm to manage peers and events
    let mut swarm = {
        let mdns = mdns::async_io::Behaviour::new(mdns::Config::default(), local_peer_id)?;
        // let behaviour = MyBehaviour { gossipsub, mdns };
        let behaviour = behaviour::Behaviour::new(id_keys.public(), gossipsub, mdns);
        // SwarmBuilder::with_async_std_executor(transport, behaviour, local_peer_id).build()
        SwarmBuilder::with_async_std_executor(transport2, behaviour, local_peer_id).build()
    };

    // Read full lines from stdin
    let mut stdin = io::BufReader::new(io::stdin()).lines().fuse();

    // Listen on all interfaces and whatever port the OS assigns
    // swarm.listen_on("/ip4/0.0.0.0/udp/0/quic-v1".parse()?)?;
    swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;
    let ws_addr: Multiaddr = "/ip4/0.0.0.0/tcp/4006/ws".parse()?;
    println!("{}", ws_addr);
    swarm.listen_on(ws_addr.clone())?;

    // Order Kademlia to search for a peer.
    let to_search: PeerId = identity::Keypair::generate_ed25519().public().into();
   
    swarm.behaviour_mut().kademlia.get_closest_peers(to_search);
    // let mut _md: Multiaddr =
    //     "/ip4/127.0.0.1/tcp/4001/p2p/12D3KooWKpCMEGZ2fjmGeKxX6CZ6MvbpdAC898go5UJGQPAnL18A"
    //         .parse()
    //         .unwrap();
    // swarm.dial(_md)?;
    println!("Enter messages via STDIN and they will be sent to connected peers using Gossipsub");

    // Kick it off
    loop {
        select! {
            line = stdin.select_next_some() => {
                if let Err(e) = swarm
                    .behaviour_mut().gossipsub
                    .publish(topic.clone(), line.expect("Stdin not to close").as_bytes()) {
                    println!("Publish error: {e:?}");
                }
            },
            event = swarm.select_next_some() => match event {
                SwarmEvent::Behaviour(behaviour::BehaviourEvent::Mdns(mdns::Event::Discovered(list))) => {
                    for (peer_id, _multiaddr) in list {
                        println!("mDNS discovered a new peer: {peer_id}");
                        swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                    }
                },
                SwarmEvent::Behaviour(behaviour::BehaviourEvent::Mdns(mdns::Event::Expired(list))) => {
                    for (peer_id, _multiaddr) in list {
                        println!("mDNS discover peer has expired: {peer_id}");
                        swarm.behaviour_mut().gossipsub.remove_explicit_peer(&peer_id);
                    }
                },
                SwarmEvent::Behaviour(behaviour::BehaviourEvent::Kademlia(KademliaEvent::OutboundQueryProgressed  {id:_, result: QueryResult::GetClosestPeers(result), stats:_,step:_ })) => {
                    match result {
                        Ok(ok) => {
                          if !ok.peers.is_empty() {
                            for pid in &ok.peers {
                                // swarm.behaviour().kademlia.add_address(&pid);
                            };
                            println!("Query finished with closest peers: {:#?}", ok.peers);
                            for pid in swarm.connected_peers() {
                                println!("Searching for the closest peers to {:?}", pid);
                            };
                          } else {
                            // The example is considered failed as there
                            // should always be at least 1 reachable peer.
                            println!("Query finished with no closest peers.")
                          }
                        }
                        Err(_) => {
                            println!("Query timed out with no closest peers.");

                        }
                      };
                    // println!("Kad updated: {}",peer.to_string());
                    // swarm.behaviour_mut().gossipsub.remove_explicit_peer(&peer);
                },
                SwarmEvent::Behaviour(behaviour::BehaviourEvent::Gossipsub(gossipsub::Event::Message {
                    propagation_source: peer_id,
                    message_id: id,
                    message,
                })) => println!(
                        "Got message: '{}' with id: {id} from peer: {peer_id}",
                        String::from_utf8_lossy(&message.data),
                    ),
                SwarmEvent::NewListenAddr { address, .. } => {
                    println!("Local node is listening on {address}");
                }
                _ => {}
            }
        }
    }
}
