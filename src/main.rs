use std::fmt::Debug;
use std::str::FromStr;

use clap::Parser;
use futures::{channel::mpsc, pin_mut, FutureExt};
use libipld::ipld;
use libp2p::{futures::StreamExt, swarm::SwarmEvent};
use rust_ipfs::p2p::PeerInfo;
use rust_ipfs::unixfs::UnixfsStatus;
use rust_ipfs::{BehaviourEvent, Ipfs, IpfsOptions, IpfsPath, Protocol, PubsubEvent};

use rust_ipfs::UninitializedIpfsNoop as UninitializedIpfs;

use rustyline_async::{Readline, ReadlineError, SharedWriter};
use std::{io::Write, sync::Arc};
use tokio::sync::Notify;

#[derive(Debug, Parser)]
#[clap(name = "pubsub")]
struct Opt {
    #[clap(long)]
    disable_bootstrap: bool,
    #[clap(long)]
    disable_mdns: bool,
    #[clap(long)]
    disable_relay: bool,
    #[clap(long)]
    disable_upnp: bool,
    #[clap(long)]
    topic: Option<String>,
    #[clap(long)]
    stdout_log: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let opt = Opt::parse();
    if opt.stdout_log {
        tracing_subscriber::fmt::init();
    }

    let topic = opt.topic.unwrap_or_else(|| String::from("ipfs-chat"));

    // Initialize the repo and start a daemon
    // let opts = IpfsOptions {
    //     // Used to discover peers locally
    //     mdns: !opt.disable_mdns,
    //     // Used, along with relay [client] for hole punching
    //     dcutr: !opt.disable_relay,
    //     // Used to connect to relays
    //     relay: !opt.disable_relay,
    //     relay_server: true,
    //     listening_addrs: vec![
    //         "/ip4/0.0.0.0/tcp/0".parse().unwrap(),
    //         "/ip4/0.0.0.0/udp/0/quic-v1".parse().unwrap(),
    //         "/ip6/::/tcp/0".parse().unwrap(),
    //         "/ip6/::/udp/0/quic-v1".parse().unwrap(),
    //     ],
    //     // used to attempt port forwarding
    //     port_mapping: !opt.disable_upnp,
    //     ..Default::default()
    // };

    let (tx, mut rx) = mpsc::unbounded();

    // Initialize IPFS node which GossipSub enabled
    let ipfs: Ipfs = UninitializedIpfs::new()
        .enable_mdns()
        .enable_relay(true)
        .enable_upnp()
        .swarm_events({
            move |_, event| {
                if let SwarmEvent::Behaviour(BehaviourEvent::Autonat(
                    libp2p::autonat::Event::StatusChanged { new, .. },
                )) = event
                {
                    match new {
                        libp2p::autonat::NatStatus::Public(_) => {
                            let _ = tx.unbounded_send(true);
                        }
                        libp2p::autonat::NatStatus::Private
                        | libp2p::autonat::NatStatus::Unknown => {
                            let _ = tx.unbounded_send(false);
                        }
                    }
                }
            }
        })
        .start()
        .await?;

    // Add Default Bootstrap node
    ipfs.default_bootstrap().await?;

    // Set this node Identify
    let identity = ipfs.identity(None).await?;
    let PeerInfo {
        peer_id,
        listen_addrs: addresses,
        ..
    } = identity;
    for address in addresses {
        println!("Listening on: {address}");
    }
    let (mut rl, mut stdout) = Readline::new(format!("{peer_id} >"))?;

    // Boootstrapping
    tokio::spawn({
        let ipfs = ipfs.clone();
        async move { if let Err(_e) = ipfs.bootstrap().await {} }
    });

    let cancel = Arc::new(Notify::new());
    // Enabling
    tokio::spawn({
        let ipfs = ipfs.clone();
        let mut stdout = stdout.clone();
        let cancel = cancel.clone();
        async move {
            let mut listening_addrs = vec![];
            let mut relay_used = false;
            loop {
                let flag = tokio::select! {
                    flag = rx.next() => {
                        flag.unwrap_or_default()
                    },
                    _ = cancel.notified() => break
                };

                match flag {
                    true => {
                        if relay_used {
                            writeln!(stdout, "Disabling Relay...")?;
                            for addr in listening_addrs.drain(..) {
                                if let Err(_e) = ipfs.remove_listening_address(addr).await {}
                            }
                            relay_used = false;
                        }
                    }
                    false => {
                        if !relay_used {
                            writeln!(stdout, "Enabling Relay...")?;
                            for addr in ipfs.get_bootstraps().await? {
                                let circuit = addr.with(Protocol::P2pCircuit);
                                if let Ok(addr) = ipfs.add_listening_address(circuit.clone()).await
                                {
                                    listening_addrs.push(addr)
                                }
                            }
                            relay_used = !listening_addrs.is_empty();
                        }
                    }
                }
            }
            Ok::<_, anyhow::Error>(())
        }
    });

    // // Retrieve IPFS file
    // tokio::spawn({
    //     let ipfs = ipfs.clone();
    //     let mut stdout = stdout.clone();
    //     let cancel = cancel.clone();

    //     async move {
    //         let stream = ipfs
    //             .cat_unixfs(
    //                 IpfsPath::from_str("/ipfs/QmWfVY9y3xjsixTgbd9AorQxH7VtMpzfx2HaWtsoUYecaX")?,
    //                 None,
    //             )
    //             .await?
    //             .boxed();

    //         pin_mut!(stream);

    //         loop {
    //             // let flag = tokio::select! {
    //             //     flag = rx.next() => {
    //             //         flag.unwrap_or_default()
    //             //     },
    //             //     _ = cancel.notified() => break
    //             // };
    //             // match flag {
    //             //     true =>
    //             match stream.next().await {
    //                 Some(Ok(bytes)) => {
    //                     writeln!(stdout, "{}", "sdsd")?;
    //                     // stdout.write_all(&bytes).await?;
    //                 }
    //                 Some(Err(e)) => {
    //                     eprintln!("Error: {e}");
    //                     // exit(1);
    //                     break;
    //                 }
    //                 None => break,
    //                 // },
    //                 // false => break,
    //             }
    //             // This could be made more performant by polling the stream while writing to stdout.
    //         }
    //         Ok::<_, anyhow::Error>(())
    //     }
    // });

    // Pinned file test
    tokio::spawn({
        let ipfs = ipfs.clone();
        let mut stdout = stdout.clone();
        let cancel = cancel.clone();

        async move {
            let stream = ipfs.add_file_unixfs("./test").await?.boxed();

            pin_mut!(stream);

            loop {
                // let flag = tokio::select! {
                //     flag = rx.next() => {
                //         flag.unwrap_or_default()
                //     },
                //     _ = cancel.notified() => break
                // };
                // match flag {
                //     true => {
                let status = stream.next().await.unwrap();
                match status {
                    UnixfsStatus::ProgressStatus {
                        written,
                        total_size,
                    } => match total_size {
                        Some(size) => writeln!(stdout, "{written} out of {size} stored")?,
                        None => writeln!(stdout, "{written} been stored")?,
                    },
                    UnixfsStatus::FailedStatus {
                        written,
                        total_size,
                        error,
                    } => {
                        match total_size {
                            Some(size) => {
                                writeln!(stdout, "failed with {written} out of {size} stored")?
                            }
                            None => writeln!(stdout, "failed with {written} stored")?,
                        }

                        if let Some(error) = error {
                            anyhow::bail!(error);
                        } else {
                            anyhow::bail!("Unknown error while writting to blockstore");
                        }
                        // break;
                    }
                    UnixfsStatus::CompletedStatus { path, written, .. } => {
                        writeln!(stdout, "{written} been stored with path {path}")?;
                        break;
                    }
                }
                // }
                //     false => break,
                // }
            }
            Ok::<_, anyhow::Error>(())
        }
    });

    // Subscibe topic
    let mut event_stream = ipfs.pubsub_events(&topic).await?;

    let stream = ipfs.pubsub_subscribe(topic.to_string()).await?;

    pin_mut!(stream);

    tokio::spawn(topic_discovery(ipfs.clone(), topic.clone(), stdout.clone()));

    tokio::task::yield_now().await;

    loop {
        tokio::select! {
            data = stream.next() => {
                if let Some(msg) = data {
                    writeln!(stdout, "{}: {}", msg.source.expect("Message should contain a source peer_id"), String::from_utf8_lossy(&msg.data))?;
                }
            }
            Some(event) = event_stream.next() => {
                match event {
                    PubsubEvent::Subscribe { peer_id } => writeln!(stdout, "{} subscribed", peer_id)?,
                    PubsubEvent::Unsubscribe { peer_id } => writeln!(stdout, "{} unsubscribed", peer_id)?,
                }
            }
            line = rl.readline().fuse() => match line {
                Ok(line) => {
                    if let Err(e) = ipfs.pubsub_publish(topic.clone(), line.as_bytes().to_vec()).await {
                        writeln!(stdout, "Error publishing message: {e}")?;
                        continue;
                    }
                    writeln!(stdout, "{peer_id}: {line}")?;
                }
                Err(ReadlineError::Eof) => {
                    cancel.notify_one();
                    break
                },
                Err(ReadlineError::Interrupted) => {
                    cancel.notify_one();
                    break
                },
                Err(e) => {
                    writeln!(stdout, "Error: {e}")?;
                    writeln!(stdout, "Exiting...")?;
                    break
                },
            }
        }
    }
    // Exit
    ipfs.exit_daemon().await;
    Ok(())
}

//Note: This is temporary as a similar implementation will be used internally in the future
async fn topic_discovery(
    ipfs: Ipfs,
    topic: String,
    mut stdout: SharedWriter,
) -> anyhow::Result<()> {
    let cid = ipfs.put_dag(ipld!(topic.clone())).await?;
    ipfs.provide(cid).await?;
    writeln!(stdout, "{}", topic)?;
    writeln!(stdout, "{cid}")?;
    loop {
        let mut stream = ipfs.get_providers(cid).await?.boxed();
        while let Some(_providers) = stream.next().await {}
    }
}

// async fn get_file(ipfs: Ipfs) -> anyhow::Result<()> {
//     let mut stdout = tokio::io::stdout();

//     let stream = ipfs
//         .cat_unixfs(
//             IpfsPath::from_str("/ipfs/QmWfVY9y3xjsixTgbd9AorQxH7VtMpzfx2HaWtsoUYecaX")?,
//             None,
//         )
//         .await?
//         .boxed();

//     pin_mut!(stream);

//     loop {
//         // This could be made more performant by polling the stream while writing to stdout.
//         match stream.next().await {
//             Some(Ok(bytes)) => {
//                 stdout.write_all(&bytes).await?;
//             }
//             Some(Err(e)) => {
//                 eprintln!("Error: {e}");
//                 // exit(1);
//                 break;
//             }
//             None => break,
//         }
//     }
//     Ok::<_, anyhow::Error>(())
// }
