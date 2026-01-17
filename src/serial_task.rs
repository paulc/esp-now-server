use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};

use tokio::sync::{mpsc, oneshot};
use tokio::time::{timeout, Duration};
use tokio_serial::{SerialPortBuilderExt, SerialStream};
use tokio_stream::StreamExt;
use tokio_util::codec::Framed;

use futures_util::sink::SinkExt;

#[allow(unused_imports)]
use tracing::{debug, error, info, warn};

use esp_now_protocol::{Ack, Msg};

use crate::framed_codec::{FramedBinaryCodec, MAX_PAYLOAD_LEN};

static MSG_ID: AtomicU32 = AtomicU32::new(0);

fn encode_msg(msg: &Msg) -> Result<heapless::Vec<u8, MAX_PAYLOAD_LEN>, ()> {
    let src: heapless::Vec<u8, MAX_PAYLOAD_LEN> = msg.to_heapless().map_err(|_| ())?;
    heapless::Vec::from_slice(&src).map_err(|_| ())
}

pub fn next_id() -> u32 {
    MSG_ID.fetch_add(1, Ordering::Relaxed)
}

pub struct SerialTask {
    tty_path: String,
    baud_rate: u32,
}

impl SerialTask {
    pub fn new(tty_path: String, baud_rate: u32) -> Self {
        Self {
            tty_path,
            baud_rate,
        }
    }
    pub async fn start(
        &self,
    ) -> Result<
        (mpsc::UnboundedSender<Msg>, mpsc::UnboundedReceiver<Msg>),
        Box<dyn std::error::Error>,
    > {
        // Create channels
        let (command_tx, mut command_rx) = mpsc::unbounded_channel::<Msg>();
        let (event_tx, event_rx) = mpsc::unbounded_channel::<Msg>();

        let mut serial =
            tokio_serial::new(self.tty_path.clone(), self.baud_rate).open_native_async()?;

        #[cfg(unix)]
        serial
            .set_exclusive(false)
            .expect("Unable to set serial port exclusive to false");

        let mut framed = Framed::new(serial, FramedBinaryCodec::new());

        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(10));
            let mut waiting_ack: HashMap<u32, oneshot::Sender<bool>> = HashMap::new();

            loop {
                tokio::select! {
                    serial_rx = framed.try_next() => {
                        info!("SELECT --> FRAMED");
                        match serial_rx {
                            Ok(Some(Ok(pkt))) => {
                                if let Ok(msg) = Msg::from_slice(&pkt) {
                                    info!("[+] RX Msg: {msg}");
                                    match msg {
                                        Msg::Ack(ack) => {
                                            // Return ACK status
                                            if let Some(tx) = waiting_ack.remove(&ack.rx_id) {
                                                info!(">>> GOT ACK: {}", ack.rx_id);
                                                let _ = tx.send(true);
                                            } else {
                                                info!(">>> ACK NOT FOUND: {}", ack.rx_id);
                                            }
                                        },
                                        _ => {
                                            // Send to event queue
                                            let msg_id = msg.get_id();
                                            let status = event_tx.send(msg).is_ok();
                                            // Send ack
                                            let ack = Msg::Ack (
                                                Ack { id: next_id(), rx_id: msg_id, status }
                                            );
                                            let encoded_msg = encode_msg(&ack).expect("Error encoding msg: {ack}");
                                            match framed.send(&encoded_msg).await {
                                                Ok(_) => {
                                                    info!("[+] Ack OK: {ack}");
                                                },
                                                Err(e) => {
                                                    error!("[-] Ack Error: {e}");
                                                }
                                            }
                                        }
                                    }
                                } else {
                                    error!("[-] Invalid Msg {pkt:?}");
                                }
                            }
                            Ok(Some(Err(e))) => error!(">> RX Error: {e:?}"),
                            Ok(None) => {}
                            Err(e) => {
                                error!("ERR: {e}");
                                break
                            }
                        }
                    }
                    command = command_rx.recv() => {
                        info!("SELECT --> COMMAND");
                        match command {
                            Some(msg) => {
                                info!("[+] RX COMMAND: {msg}");
                                match send_msg(&mut framed, msg).await {
                                    Ok((id,tx)) => {
                                        // Save channel in ack hashmap
                                        waiting_ack.insert(id,tx);
                                    },
                                    Err(e) => error!("[-] TX Error: {e}")
                                }
                            }
                            None=> {
                                // Channel closed
                                break;
                            }
                        }
                    }
                    _ = ticker.tick() => {
                        info!("SELECT --> TICK");
                        // Prune waiting_ack
                        let keys: Vec<u32> = waiting_ack.keys().copied().collect();
                        keys.into_iter().for_each(|k| {
                            let v = waiting_ack.remove(&k).unwrap(); // Safe
                            info!("WAITING_ACK: >> {k} : {v:?}");
                            if v.is_closed() {
                                info!("[-] Pruning expired ack: id={k}");
                            } else {
                                waiting_ack.insert(k, v);
                            }
                        });
                    }
                }
            }
        });

        Ok((command_tx, event_rx))
    }
}

async fn send_msg(
    framed: &mut Framed<SerialStream, FramedBinaryCodec>,
    msg: Msg,
) -> Result<(u32, oneshot::Sender<bool>), Box<dyn std::error::Error>> {
    let id = msg.get_id();
    let encoded_msg = encode_msg(&msg).expect("Error encoding msg: {msg}");
    framed.send(&encoded_msg).await?;
    debug!("[+] TX Msg OK: {msg}");

    // Spawn a task to wait for the ACK (with timeout)
    let (tx, rx) = oneshot::channel::<bool>();
    tokio::spawn(async move {
        // Wait max 1 second for a ACK
        match timeout(Duration::from_secs(1), rx).await {
            Ok(Ok(status)) => {
                // RX ACK
                debug!("[+] RX Ack: {msg} -> {status}");
            }
            Ok(Err(_)) => {
                // Channel closed: The sender was dropped without sending
                error!("[-] RX channel closed: {}", id);
            }
            Err(_) => {
                // Timeout
                error!("[-] Timeout: {msg}");
            }
        }
    });

    Ok((id, tx))
}
