use std::net::SocketAddr;
use std::path::Path;
use std::{io::Write, sync::Arc};

use tokio::net::UdpSocket;
use tokio::signal;
use tokio::sync::{mpsc, Notify};
use tokio::time::Duration;

use libp2p::futures::StreamExt;
// use libp2p::swarm::SwarmEvent;
use libp2p::Multiaddr;

use rust_ipfs::p2p::MultiaddrExt;
use rust_ipfs::p2p::PeerInfo;
use rust_ipfs::{unixfs::UnixfsStatus, Ipfs};
use rust_ipfs::{IpfsOptions, UninitializedIpfsNoop as UninitializedIpfs};

use env_logger::Builder;
use log::LevelFilter;

use clap::Parser;

#[macro_use]
extern crate log;

#[derive(Debug, Parser)]
#[clap(name = "lowpower-gateway-rust")]
struct Opt {
    #[clap(long)]
    addr: Option<Multiaddr>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let opt = Opt::parse();
    // Program Shutdown Token
    let cancel_token = Arc::new(Notify::new());
    let (_, mut shutdown_recv) = mpsc::unbounded_channel::<bool>();

    // Initialize Loggin System
    let (tx_log, mut rx_log) = mpsc::unbounded_channel::<(LevelFilter, String)>();
    let mut builder = Builder::from_default_env();
    builder
        .format(|buf, record| writeln!(buf, "{} - {}", record.level(), record.args()))
        .filter(None, LevelFilter::Error)
        .init();

    // Configurtaion Libp2p Transport
    let _transportonfig = rust_ipfs::p2p::TransportConfig {
        enable_webrtc: true,
        port_reuse: true,
        enable_websocket: true,
        enable_secure_websocket: false,
        ..rust_ipfs::p2p::TransportConfig::default()
    };
    let _ipfs_config = IpfsOptions {
        listening_addrs: vec![
            "/ip4/0.0.0.0/tcp/4001".parse().unwrap(),
            "/ip4/0.0.0.0/tcp/4002/ws".parse().unwrap(),
            // "/ip4/0.0.0.0/udp/4001/webrtc".parse().unwrap(),
            "/ip4/0.0.0.0/udp/4001/quic-v1".parse().unwrap(),
            "/ip6/::/tcp/4001".parse().unwrap(),
            // "/ip6/::/udp/0/quic-v1".parse().unwrap(),
        ],
        ..IpfsOptions::default()
    };
    // Initialize IPFS
    let ipfs: Ipfs = UninitializedIpfs::with_opt(_ipfs_config)
        .enable_mdns()
        .set_transport_configuration(_transportonfig)
        .enable_relay(true)
        // .enable_upnp()
        .enable_relay_server(None)
        .enable_rendezvous_server()
        .listen_as_external_addr()
        .fd_limit(rust_ipfs::FDLimit::Max)
        // .swarm_events({
        //     let cloned_tx_log = tx_log.clone();
        //     move |_, event| {
        //         if let SwarmEvent::NewListenAddr { address, .. } = event {
        //             let _ =
        //                 cloned_tx_log.send((LevelFilter::Info, format!("Listening on {address}")));
        //         }
        //         if let SwarmEvent::ConnectionClosed {
        //             peer_id,
        //             connection_id,
        //             endpoint,
        //             num_established,
        //             cause,
        //         } = event
        //         {}
        //     }
        // })
        .start()
        .await?;

    // Set this node Identify
    let identity = ipfs.identity(None).await?;
    let PeerInfo {
        peer_id,
        listen_addrs,
        ..
    } = identity;
    let _ = tx_log.send((LevelFilter::Info, format!("Your ID: {peer_id}")));
    println!("Your ID: {peer_id}");
    for addr in listen_addrs {
        let _ = tx_log.send((LevelFilter::Info, format!("{}", addr)));
        println!("{}", addr);
    }
    // Add Default Bootstrap node
    ipfs.default_bootstrap().await?;
    ipfs.add_peer(
        "QmcFf2FH3CEgTNHeMRGhN7HNHU1EXAxoEk6EFuSyXCsvRE".parse()?,
        "/dnsaddr/node-1.ingress.cloudflare-ipfs.com".parse()?,
    )
    .await?;

    // ipfs.add_bootstrap("/dns4/elastic.dag.house/tcp/443/wss/p2p/bafzbeibhqavlasjc7dvbiopygwncnrtvjd2xmryk5laib7zyjor6kf3avm".parse()?).await?;
    ipfs.add_bootstrap("/dns4/hoverboard-staging.dag.haus/tcp/443/wss/p2p/Qmc5vg9zuLYvDR1wtYHCaxjBHenfCNautRwCjG3n5v5fbs".parse()?).await?;
    if let Some(addr) = opt.addr {
        ipfs.add_bootstrap(addr).await?;
    }
    // .await?;
    // Boootstrapping
    if let Err(_e) = ipfs.bootstrap().await {
        let _ = tx_log.send((LevelFilter::Error, "{e}".to_string()));
    }

    let bootstrap_nodes = ipfs.get_bootstraps().await.expect("Bootstrap exist");
    let addrs = bootstrap_nodes.iter().cloned();

    for mut addr in addrs {
        let peer_id: libp2p::PeerId = addr
            .extract_peer_id()
            .expect("Bootstrap to contain peer id");
        ipfs.add_relay(peer_id, addr).await?;
    }

    if let Err(e) = ipfs.enable_relay(None).await {
        let _ = tx_log.send((LevelFilter::Error, format!("Error selecting a relay: {e}")));
    }

    let topic: String = format!("{peer_id}").to_string();
    let _ = tx_log.send((LevelFilter::Info, format!("TOPIC: {topic}")));

    // Socket for UNiX
    // let listener = UnixListener::bind("./test").unwrap();

    // Socket for Windows/Mac
    let (tx, mut rx) = mpsc::unbounded_channel::<(Vec<u8>, SocketAddr)>();
    let (tx_cid, mut rx_cid) = mpsc::unbounded_channel::<Vec<u8>>();

    tokio::spawn({
        let mut data: Option<Vec<u8>> = None;
        let cloned_ipfs = ipfs.clone();
        let cloned_topic = topic.clone();
        let cloned_tx_log = tx_log.clone();

        async move {
            loop {
                tokio::select! {
                    Some(_recv_data)=rx_cid.recv() =>{
                        data=Some(_recv_data.clone());
                        if let Err(_e) = cloned_ipfs.pubsub_publish(cloned_topic.clone(), _recv_data).await {
                            let _=cloned_tx_log.send((LevelFilter::Error,format!("{_e}")));
                        }
                    }
                    _=tokio::time::sleep(Duration::from_secs(1))=> {
                        if let Some(_data) = data.clone() {
                            if let Err(_e) = cloned_ipfs.pubsub_publish(cloned_topic.clone(), _data).await {
                                let _=cloned_tx_log.send((LevelFilter::Error,format!("{_e}")));
                                continue;
                            }
                        }
                    }
                }
            }
        }
    });

    tokio::spawn({
        let cloned_ipfs = ipfs.clone();
        let cloned_tx_cid = tx_cid.clone();
        let cloned_tx_log = tx_log.clone();

        async move {
            loop {
                let (bytes, _) = rx.recv().await.unwrap();

                let _ =
                    cloned_tx_log.send((LevelFilter::Debug, format!("{:#?} bytes sent", &bytes)));
                tokio::spawn({
                    let cloned_ipfs_inner = cloned_ipfs.clone();
                    let cloned_tx_cid_inner = cloned_tx_cid.clone();
                    let cloned_tx_log_inner = cloned_tx_log.clone();

                    let ss = String::from_utf8(bytes).unwrap();
                    let _path = Path::new(&ss).to_owned().clone();
                    async move {
                        let mut stream = cloned_ipfs_inner.add_file_unixfs(_path).await.unwrap();
                        while let Some(status) = stream.next().await {
                            match status {
                                UnixfsStatus::CompletedStatus { path, written, .. } => {
                                    let _ = cloned_tx_log_inner.send((
                                        LevelFilter::Debug,
                                        format!("{written} been stored with path {path}"),
                                    ));

                                    let _cid = path.root().cid().unwrap();

                                    cloned_tx_cid_inner
                                        .send(_cid.to_string().into_bytes())
                                        .unwrap();
                                    cloned_ipfs_inner.insert_pin(&_cid, true).await.unwrap();
                                    break;
                                }
                                _ => {}
                            }
                        }
                    }
                });
            }
        }
    });

    let listener = Arc::new(UdpSocket::bind("0.0.0.0:3132".parse::<SocketAddr>().unwrap()).await?);
    let mut buf = [0; 1024];

    tokio::spawn({
        async move {
            loop {
                // println!("LISTEN UDP : {}", &"0.0.0.0:3132");
                let (len, addr) = listener.recv_from(&mut buf).await.unwrap();
                let _ = tx.send((buf[..len].to_vec(), addr));
                // println!("{:?} bytes received from {:?}", len, addr);
                // tx.send((buf[..len].to_vec(), addr)).await.unwrap();
            }
        }
    });

    // let _asdasd = ipfs
    //     .find_peer("12D3KooWJvMFqsvvSojDmBeffhQFfRPNQRCmErvbYFsN89i5Czwy".parse()?)
    //     .await?;
    // println!("{:?}", _asdasd);

    tokio::task::yield_now().await;
    println!("Successfully Loaded");
    loop {
        tokio::select! {
            _ = signal::ctrl_c() => {
                println!("SHUTDOWN");
                cancel_token.notify_one();
                break
            }
            Some(flag) = shutdown_recv.recv() => {
                if flag == true {
                    println!("SHUTDOWN2");
                    cancel_token.notify_one();
                    break
                }
            }
            Some((log_type, log_str)) = rx_log.recv() => match log_type {
                LevelFilter::Info => {
                    info!("{}",&log_str);
                }
                LevelFilter::Warn => {
                    warn!("{}",&log_str);
                }
                LevelFilter::Error => {
                    error!("{}",&log_str);
                }
                LevelFilter::Debug => {
                    debug!("{}",&log_str);
                }
                _=>{
                    println!("{}",&"Disabled Logging")
                }
            }
            // line = rl.readline().fuse() => match line {
            //     Ok(line) => {
            //         if let Err(e) = ipfs.pubsub_publish(topic.clone(), line.as_bytes().to_vec()).await {
            //             writeln!(stdout, "Error publishing message: {e}")?;
            //             // continue;
            //         }
            //         writeln!(stdout, "{peer_id}: {line}")?;
            //     }
            //     Err(ReadlineError::Eof) => {
            //         println!("SHUTDOWN");
            //         cancel_token.notify_one();
            //         // break
            //     },
            //     Err(ReadlineError::Interrupted) => {
            //         println!("SHUTDOWN2");
            //         cancel_token.notify_one();
            //         // break
            //     },
            //     Err(e) => {
            //         // cancel_token.notify_one();

            //         writeln!(stdout, "Error: {e}")?;
            //         writeln!(stdout, "Exiting...")?;
            //         // break
            //     },
            // }
        }
    }
    ipfs.exit_daemon().await;
    Ok(())
    // send shutdown signal to application and wait
}
