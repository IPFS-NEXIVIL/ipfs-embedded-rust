
use clap::Parser;
use futures::prelude::*;
use ipfs_embed::{Config, GossipEvent, Ipfs, Multiaddr, NetworkConfig, PeerId, StorageConfig};
use libipld::{store::DefaultParams, store::StoreParams, Cid, IpldCodec, Result};
use std::time::Duration;
// use tempdir::TempDir;

#[derive(Debug, Parser)]
struct Args {
    /// Peer id to dial
    #[arg(short, long, default_missing_value = None)]
    peerid: Option<String>,

    #[arg(short, long, default_missing_value = None)]
    mport: Option<String>,
}

#[derive(Debug, Clone)]
struct Sp;

impl StoreParams for Sp {
    type Hashes = libipld::multihash::Code;
    type Codecs = IpldCodec;
    const MAX_BLOCK_SIZE: usize = 1024 * 1024 * 4;
}

// fn tracing_try_init() {
//     tracing_subscriber::fmt()
//         .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
//         .init()
// }

async fn create_store(enable_mdns: bool) -> Result<Ipfs<DefaultParams>> {
    let sweep_interval = Duration::from_millis(10000);
    let storage = StorageConfig::new(None, None, 10, sweep_interval);

    let mut network = NetworkConfig::default();
    if !enable_mdns {
        network.mdns = None;
    }

    let mut ipfs = Ipfs::new(Config { storage, network }).await?;
    ipfs.listen_on("/ip4/0.0.0.0/tcp/0".parse().unwrap())
        .next()
        .await
        .unwrap();
    let peer: PeerId = "12D3KooWQBoWvqL54AAn3EXnLcX4km1S5WHg57a1wBgGCWpdBht7".parse()?;
    let addr: Multiaddr = "/ip4/127.0.0.1/tcp/4001".parse()?;
    ipfs.bootstrap(vec![(peer, addr)]).await?;

    Ok(ipfs)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _args = Args::parse();

    // tracing_try_init();
    // let config = Config::default();
    // let mut ipfs = Ipfs::<Sp>::new(config).await?;
    // let peer: PeerId = "12D3KooWPwBa1pHTcFbhUj5nehr6R8tDjyYbRUoEU8UjmPrrmGWp".parse()?;
    // let addr: Multiaddr = "/ip4/192.168.1.165/tcp/4001".parse()?;
    // ipfs.bootstrap(vec![(peer, addr)]).await?;
    let mut ipfs = create_store(true).await.unwrap();

    println!("Booted: {}", ipfs.is_bootstrapped());
    if let Some(_pid) = _args.peerid {
        let _peer: PeerId = _pid.parse()?;
        let _port: Multiaddr = _args.mport.unwrap().parse().unwrap();
        ipfs.dial_address(
            _peer.clone(),
            // "/dns4/libp2p.nexivil.com/tcp/4001".parse().unwrap(),
            _port.clone(),
        );
        tokio::time::sleep(tokio::time::Duration::from_millis(2500)).await;
        println!("Connected: {}", ipfs.is_connected(&_peer));
    }
    // 10 random bytes
    // let _cid_rand10: Cid = "QmXQsqVRpp2W7fbYZHi4aB2Xkqfd3DpwWskZoLVEYigMKC".parse()?;
    // a dag-cbor leaf
    let cid_leaf_cbor: Cid = "QmYqNKAJakPzQZy8gKwarv7EAgTYrMuVcYxSMVU9qhPhBt".parse()?;
    // // a very simple dag-cbor dag with 1 child
    // let cid_simple_dag: Cid = "bafkreic37cvfp7c2npcupxwpdtdnwy7rbxvvli6gyxpus7ldd6z5sxq2x4".parse()?;
    // a unixfs v1 movie
    // let _cid_movie: Cid = "QmWhFbSZ6gr3sz5EpxjmxhPCfj4JYH43y4p6o1gNzSMzow".parse()?;
    let block = ipfs.fetch(&cid_leaf_cbor, ipfs.peers()).await?;
    println!("got single block. len = {}", block.data().len());

    // let block = ipfs.get(&cid_simple_dag).unwrap();
    // println!("got single block. len = {}", block.data().len());

    // let mut updates = ipfs.sync(&cid_simple_dag, ipfs.peers()).await?;
    // println!("starting sync of large file");
    // while let Some(update) = updates.next().await {
    //     println!("{:?}", update);
    // }
    let topic: String = "fruits".to_string();
    let mut tt = ipfs.subscribe(topic.clone()).await?;

    ipfs.publish(topic.clone(), b"test hi1".to_vec()).await?;

    println!("{}", ipfs.local_peer_id());
    println!("{}", ipfs.listeners().into_iter().next().unwrap());

    while let Some(value) = tt.next().await {
        match value {
            GossipEvent::Message(_, data) => {
                println!("Got {}", String::from_utf8_lossy(&data));
            }
            GossipEvent::Subscribed(id) => {
                println!("Sub: {}", id);
            }
            GossipEvent::Unsubscribed(id) => {
                println!("UnSub: {}", id);
            }
        }
    }
    Ok(())
}
