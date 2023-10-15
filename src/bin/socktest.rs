use chrono::Utc;
use rand::{rngs::StdRng, Rng, SeedableRng};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::fs::{File, OpenOptions};
use tokio::io::AsyncWriteExt;
use tokio::net::UdpSocket;
use tokio::signal;
use tokio::sync::{mpsc, Notify};
use tokio::time::Duration;
use tracing_subscriber::fmt::format;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let listener = UdpSocket::bind("0.0.0.0:0".parse::<SocketAddr>().unwrap()).await?;
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let noti = Arc::new(Notify::new());
    let (tx_done, mut rx_done) = mpsc::unbounded_channel::<String>();

    tokio::spawn({
        let mut path: String = format!(
            "./shared/test-{}.txt",
            Utc::now().format("%Y_%d_%m-%H_%M_%S")
        );
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .open(path.clone())
            .await?;

        let cloned_tx = tx_done.clone();
        let cloned_noti = noti.clone();
        async move {
            loop {
                tokio::select! {
                    Some(_data) = rx.recv() =>{
                        file.write_all(_data.as_bytes()).await.unwrap();
                    }
                    _=cloned_noti.notified()=>{
                        // let _path=path.clone();
                        let _=cloned_tx.send(path.clone());
                        path=format!("./shared/test-{}.txt", Utc::now().format("%Y_%d_%m-%H_%M_%S"));
                        file=OpenOptions::new()
                        .write(true)
                        .create(true)
                        .open(path.clone())
                        .await.unwrap();
                        // break
                    }
                }
            }
        }
    });
    tokio::spawn({
        let cloned_noti = noti.clone();
        async move {
            loop {
                tokio::time::sleep(Duration::from_secs(3)).await;

                cloned_noti.notify_one();
            }
        }
    });
    tokio::spawn({
        let cloned_tx = tx.clone();
        // let cloned_tx2 = tx.clone();
        // let mut rng = rand::thread_rng();

        let mut rng = {
            let rng = rand::thread_rng();
            StdRng::from_rng(rng).unwrap()
        };

        async move {
            let mut i = 0;
            let sleep_duration = 1000 / 200;
            loop {
                let _ = cloned_tx.send(format!(
                    "{}  {}\n",
                    rng.gen::<f64>().to_string(),
                    Utc::now().timestamp_millis()
                ));
                i = i + 1;
                tokio::time::sleep(Duration::from_millis(sleep_duration)).await;
            }
        }
    });

    tokio::spawn({
        async move {
            loop {
                tokio::select! {
                Some(_path) = rx_done.recv()=>{
                    // println!("{}",_path);
                    let _ = listener.send_to(
                            &_path.as_bytes(),
                            "127.0.0.1:3132".parse::<SocketAddr>().unwrap(),
                        ).await.unwrap();
                    }
                }
            }
        }
    });
    // let mut buf: [u8; 100] = [0; 100];
    tokio::task::yield_now().await;

    loop {
        tokio::select! {
            _ = signal::ctrl_c() => {
                println!("SHUTDOWN");
                // cancel_token.notify_one();
                break
            }
        }
        // tokio::time::sleep(Duration::from_secs(3)).await;

        // // buf[0] = "asd".as_bytes();
        // let len = listener
        //     .send_to(
        //         &"./test2".as_bytes(),
        //         "127.0.0.1:3132".parse::<SocketAddr>().unwrap(),
        //     )
        //     .await?;
        // println!("{:?} bytes send to", len);
        // break;
        // // tx.send((buf[..len].to_vec(), addr)).await.unwrap();
    }
    Ok(())
}
