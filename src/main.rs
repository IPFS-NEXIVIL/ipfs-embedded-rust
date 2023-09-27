use std::net::SocketAddr;
use std::path::Path;
use std::{io::Write, sync::Arc};

// use std::fs::rea;

// use tokio::net::UnixListener;
use tokio::net::UdpSocket;
use tokio::signal;
use tokio::sync::{mpsc, Notify};
use tokio::time::Duration;

use libp2p::futures::StreamExt;
use libp2p::swarm::SwarmEvent;

use rust_ipfs::p2p::MultiaddrExt;
use rust_ipfs::p2p::PeerInfo;
use rust_ipfs::UninitializedIpfsNoop as UninitializedIpfs;
use rust_ipfs::{unixfs::UnixfsStatus, Ipfs};

// use rustyline_async::{Readline, ReadlineError,SharedWriter};
// async fn ipfsInit() ->
use env_logger::Builder;
use log::LevelFilter;

#[macro_use]
extern crate log;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Program Shutdown Token
    let cancel_token = Arc::new(Notify::new());
    let (_, mut shutdown_recv) = mpsc::unbounded_channel::<bool>();

    // Initialize Loggin System
    let (tx_log, mut rx_log) = mpsc::unbounded_channel::<(LevelFilter, String)>();
    let mut builder = Builder::from_default_env();
    builder
        .format(|buf, record| writeln!(buf, "{} - {}", record.level(), record.args()))
        .filter(None, LevelFilter::Info)
        .init();

    // Initialize IPFS
    let ipfs: Ipfs = UninitializedIpfs::new()
        .enable_mdns()
        .enable_relay(true)
        .enable_upnp()
        .swarm_events({
            let cloned_tx_log = tx_log.clone();
            move |_, event| {
                if let SwarmEvent::NewListenAddr { address, .. } = event {
                    let _ =
                        cloned_tx_log.send((LevelFilter::Info, format!("Listening on {address}")));
                }
            }
        })
        .start()
        .await?;

    // Set this node Identify
    let identity = ipfs.identity(None).await?;
    let PeerInfo {
        peer_id,
        listen_addrs: _,
        ..
    } = identity;
    let _ = tx_log.send((LevelFilter::Info, format!("Your ID: {peer_id}")));
    // let (mut rl, mut stdout) = Readline::new(format!("{peer_id} >"))?;

    // Add Default Bootstrap node
    ipfs.default_bootstrap().await?;

    // Boootstrapping
    if let Err(_e) = ipfs.bootstrap().await {
        let _ = tx_log.send((LevelFilter::Error, "{e}".to_string()));
    }

    let bootstrap_nodes = ipfs.get_bootstraps().await.expect("Bootstrap exist");
    let addrs = bootstrap_nodes.iter().cloned();

    for mut addr in addrs {
        let peer_id = addr
            .extract_peer_id()
            .expect("Bootstrap to contain peer id");
        ipfs.add_relay(peer_id, addr).await?;
    }

    if let Err(e) = ipfs.enable_relay(None).await {
        let _ = tx_log.send((LevelFilter::Error, format!("Error selecting a relay: {e}")));
    }

    let topic: String = "fruits".to_string();

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
                        data=Some(_recv_data);
                    }
                    _=tokio::time::sleep(Duration::from_secs(1))=> {
                        if let Some(_data) = data.clone() {
                            if let Err(_e) = cloned_ipfs.pubsub_publish(cloned_topic.clone(), _data).await {
                                let _=cloned_tx_log.send((LevelFilter::Error,"{_e}".to_string()));
                                continue;
                            }
                        }
                    }
                }
            }
        }
    });

    tokio::spawn({
        // let mut stream = ipfs.add_file_unixfs(opt.file).await?;
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
    let mut buf = [0; 4096];

    tokio::spawn({
        async move {
            loop {
                println!("LISTEN UDP : {}", &"0.0.0.0:3132");
                let (len, addr) = listener.recv_from(&mut buf).await.unwrap();
                let _ = tx.send((buf[..len].to_vec(), addr));
                println!("{:?} bytes received from {:?}", len, addr);
                // tx.send((buf[..len].to_vec(), addr)).await.unwrap();
            }
        }
    });

    tokio::task::yield_now().await;
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
