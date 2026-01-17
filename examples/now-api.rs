use std::env;

use futures_util::sink::SinkExt;

use tokio::sync::mpsc;
// use tokio::time::{Duration, Timeout};
use tokio_serial::SerialPortBuilderExt;
use tokio_stream::StreamExt;
use tokio_util::codec::Framed;

use esp_now_protocol::format_mac::from_mac;
use esp_now_protocol::{BroadcastData, Msg, TxData};
use esp_now_server::framed_codec::{FramedBinaryCodec, MAX_PAYLOAD_LEN};

use std::sync::atomic::{AtomicU32, Ordering};

static MSG_ID: AtomicU32 = AtomicU32::new(0);

fn encode_msg(msg: &Msg) -> Result<heapless::Vec<u8, MAX_PAYLOAD_LEN>, ()> {
    let src: heapless::Vec<u8, MAX_PAYLOAD_LEN> = msg.to_heapless().map_err(|_| ())?;
    heapless::Vec::from_slice(&src).map_err(|_| ())
}

fn next_id() -> u32 {
    MSG_ID.fetch_add(1, Ordering::Relaxed)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Get the current executable name
    let exe_path = env::current_exe().expect("Failed to get current executable path");
    let exe_name = exe_path.file_name().expect("Failed to get executable name");

    let mut args = env::args();
    let tty_path = args
        .nth(1)
        .expect(&format!("Usage: {} <tty_path>", exe_name.to_string_lossy()));

    let mut serial = tokio_serial::new(tty_path, 115200).open_native_async()?;

    #[cfg(unix)]
    serial
        .set_exclusive(false)
        .expect("Unable to set serial port exclusive to false");

    let mut framed = Framed::new(serial, FramedBinaryCodec::new());

    let (tx_queue, mut rx_queue) = mpsc::channel::<Vec<u8>>(10);

    // Stdin reader task
    tokio::spawn(async move {
        use tokio::io::{stdin, AsyncBufReadExt, BufReader};
        let mut stdin = BufReader::new(stdin());
        let mut buffer = String::new();
        loop {
            buffer.clear();
            match stdin.read_line(&mut buffer).await {
                Ok(0) => break,
                Ok(_) => {
                    let msg = buffer.trim().as_bytes().to_vec();
                    if !msg.is_empty() && tx_queue.send(msg).await.is_err() {
                        break;
                    }
                }
                Err(e) => {
                    eprintln!("STDIN error: {}", e);
                    break;
                }
            }
        }
    });

    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(10));

    let mut counter = 0_u32;

    loop {
        tokio::select! {
            rx = framed.try_next() => {
                match rx {
                    Ok(Some(Ok(pkt))) => {
                        if let Ok(msg) = Msg::from_slice(&pkt) {
                            println!("[+] RX Msg: {msg}")
                            // XXX Send reply
                        } else {
                            eprintln!("[-] Invalid Msg {pkt:?}");
                        }
                    }
                    Ok(Some(Err(e))) => eprintln!(">> RX Error: {e:?}"),
                    Ok(None) => {}
                    Err(e) => {
                        eprintln!("ERR: {e}");
                        break
                    }
                }
                counter += 1;
            }
            msg = rx_queue.recv() => {
                if let Some(bcast) = msg {
                    // println!(">> TX from stdin: {:?}", String::from_utf8_lossy(&bcast));
                    // Send Broadcast
                    let mut data = heapless::Vec::<u8, 250>::new();
                    data.extend_from_slice(&bcast).unwrap();
                    let msg = Msg::Broadcast(BroadcastData {
                        id: next_id(),
                        interval: Some(5),
                        data,
                    });
                    if let Err(e) = framed.send(&encode_msg(&msg).unwrap()).await {
                        eprintln!(">> Error Sending BROADCAST message: {e:?}");
                    } else {
                        println!("Sent BROADCAST Message: OK {msg}");
                        println!("[+] BROADCAST Msg: {msg}")
                    }
                } else {
                    break; // stdin task dropped tx
                }

            }
            _ = ticker.tick() => {
                // Send TX message
                let mut data = heapless::Vec::<u8, 250>::new();
                data.extend_from_slice("HELLO".as_bytes()).unwrap();
                let msg = Msg::Send(TxData {
                    id: next_id(),
                    dst_addr: from_mac("98:a3:16:8e:ff:c0").unwrap(),
                    data,
                    defer: false,
                });
                if let Err(e) = framed.send(&encode_msg(&msg).unwrap()).await {
                    eprintln!(">> Error Sending TX message: {e:?}");
                } else {
                    println!("[+] TX Msg: {msg}")
                }
            }
            _ = tokio::signal::ctrl_c() => {
                println!("\nReceived Ctrl-C! Shutting down...\n");
                break;
            }
        }
    }
    println!("BYE");
    Ok(())
}
