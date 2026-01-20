use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};

use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::{timeout, Duration};
use tokio_serial::{SerialPortBuilderExt, SerialStream};
use tokio_stream::StreamExt;
use tokio_util::codec::Framed;

use futures_util::sink::SinkExt;

#[allow(unused_imports)]
use tracing::{debug, error, info, warn};

use esp_now_protocol::{Monitor, Msg};

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
    pub tty_path: String,
    pub baud_rate: u32,
    pub timeout_secs: u64,
}

impl SerialTask {
    pub fn new(tty_path: String, baud_rate: u32, timeout_secs: u64) -> Self {
        Self {
            tty_path,
            baud_rate,
            timeout_secs,
        }
    }
    pub async fn start(
        &self,
    ) -> Result<
        (
            mpsc::UnboundedSender<Msg>,
            mpsc::UnboundedReceiver<Msg>,
            watch::Receiver<Option<Monitor>>,
        ),
        // We return a watch channel for monitoring in addition to the mpsc channels
        Box<dyn std::error::Error>,
    > {
        // Create channels
        let (command_tx, mut command_rx) = mpsc::unbounded_channel::<Msg>();
        let (event_tx, event_rx) = mpsc::unbounded_channel::<Msg>();
        let (monitor_tx, monitor_rx) = watch::channel::<Option<Monitor>>(None);

        let mut serial =
            tokio_serial::new(self.tty_path.clone(), self.baud_rate).open_native_async()?;

        #[cfg(unix)]
        serial.set_exclusive(false)?;

        let mut framed = Framed::new(serial, FramedBinaryCodec::new());
        let timeout_secs = self.timeout_secs;

        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(10));
            let mut waiting_ack: HashMap<u32, oneshot::Sender<bool>> = HashMap::new();

            loop {
                tokio::select! {
                    serial_rx = framed.try_next() => {
                        debug!("SELECT --> FRAMED");
                        match serial_rx {
                            Ok(Some(Ok(pkt))) => {
                                if let Ok(msg) = Msg::from_slice(&pkt) {

                                    info!("SERIAL RX ->> {msg}");
                                    // Send monitor message to channel
                                    let _ = monitor_tx.send(Some(Monitor::new_rx(&msg)));

                                    match msg {
                                        Msg::Ack(ack) => {
                                            // Return ACK status
                                            if let Some(tx) = waiting_ack.remove(&ack.rx_id) {
                                                match tx.send(true) {
                                                    Ok(_) => info!("  -> ACK OK: {} <{}>", ack.rx_id, if ack.status { "SUCCESS" } else { "ERROR" }),
                                                    Err(_) => error!("  -> ACK Timeout: {}", ack.rx_id),
                                                }
                                            } else {
                                                error!("  -> ACK Id Not Found: {}", ack.rx_id);
                                            }
                                        },
                                        _ => {
                                            // Send to event queue
                                            let _msg_id = msg.get_id();
                                            let _status = event_tx.send(msg).is_ok();

                                            /*
                                            // Dont Send ACK for esp -> server events
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
                                            */
                                        }
                                    }
                                } else {
                                    error!("SERIAL RX ->> Invalid Msg {pkt:?}");
                                    // Send monitor message to channel
                                    let _ = monitor_tx.send(Some(Monitor::RxError));
                                }
                            }
                            Ok(Some(Err(e))) => error!("SERIAL RX ->> RX Error: {e:?}"),
                            Ok(None) => {}
                            Err(e) => {
                                error!("SERIAL RX ->> Device Error: {e}");
                                break
                            }
                        }
                    }
                    command = command_rx.recv() => {
                        debug!("SELECT --> COMMAND");
                        match command {
                            Some(msg) => {
                                info!("COMMAND RX ->> {msg}");
                                match send_msg(&mut framed, msg.clone(), timeout_secs).await {
                                    Ok((id,tx)) => {
                                        // Save channel in ack hashmap
                                        waiting_ack.insert(id,tx);
                                        info!("SERIAL TX ->> {msg}");
                                        // Send monitor message to channel
                                        let _ = monitor_tx.send(Some(Monitor::new_tx(&msg)));
                                    },
                                    Err(e) => {
                                        error!("SERIAL TX ->> Error: {e}");
                                        // Send monitor message to channel
                                        let _ = monitor_tx.send(Some(Monitor::new_txerror()));
                                    }
                                }
                            }
                            None=> {
                                // Channel closed
                                error!("COMMAND RX ->> Channel Closed");
                                break;
                            }
                        }
                    }
                    _ = ticker.tick() => {
                        debug!("SELECT --> TICK");
                        // Prune waiting_ack
                        let keys: Vec<u32> = waiting_ack.keys().copied().collect();
                            info!("TICK ->> Pruning Ack Map: {}", keys.len());
                        keys.into_iter().for_each(|k| {
                            let v = waiting_ack.remove(&k).unwrap(); // Safe
                            if v.is_closed() {
                                info!("  >> Pruning expired ack: id={k}");
                            } else {
                                waiting_ack.insert(k, v);
                            }
                        });
                    }
                }
            }
        });

        Ok((command_tx, event_rx, monitor_rx))
    }
}

fn strerror<'a>(e: &'a str) -> Box<dyn std::error::Error> {
    e.to_string().into()
}

async fn send_msg(
    framed: &mut Framed<SerialStream, FramedBinaryCodec>,
    msg: Msg,
    timeout_secs: u64,
) -> Result<(u32, oneshot::Sender<bool>), Box<dyn std::error::Error>> {
    let id = msg.get_id();
    let encoded_msg = encode_msg(&msg).map_err(|_| strerror("Msg Encoding Error"))?;
    framed.send(&encoded_msg).await?;

    // Spawn a task to wait for the ACK (with timeout)
    let (tx, rx) = oneshot::channel::<bool>();
    tokio::spawn(async move {
        // Wait for ACK
        match timeout(Duration::from_secs(timeout_secs), rx).await {
            Ok(Ok(status)) => {
                // RX ACK
                debug!("ACK ->> {id} : {status} [{msg}]");
            }
            Ok(Err(_)) => {
                // Channel closed: The sender was dropped without sending
                debug!("ACK ->> RX channel closed: {} [{msg}]", id);
            }
            Err(_) => {
                // Timeout
                debug!("ACK ->> Timeout: {id} [{msg}]");
            }
        }
    });

    Ok((id, tx))
}
