use esp_now_protocol::{format_mac::from_mac, BroadcastData, Msg, TxData, MAX_DATA_LEN};
use esp_now_server::serial_task::{next_id, SerialTask};

use tokio::signal::ctrl_c;
use tokio::time::{sleep, Duration};

#[allow(unused_imports)]
use tracing::{debug, error, info, warn};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    // Start Serial task
    let serial = SerialTask::new("/dev/tty.usbmodem3101".into(), 115200, 6);

    let (command_tx, mut event_rx) = serial.start().await?;

    // Spawn a task to handle events
    tokio::spawn(async move {
        while let Some(msg) = event_rx.recv().await {
            info!(">> RX Event: {msg}");
        }
    });

    let tx0 = command_tx.clone();
    let tx1 = command_tx.clone();

    // Spawn a task to send msgs
    tokio::spawn(async move {
        loop {
            let data: heapless::Vec<u8, MAX_DATA_LEN> = heapless::Vec::from_slice(b"TEST").unwrap();
            let msg = Msg::Send(TxData {
                id: next_id(),
                dst_addr: from_mac("98:a3:16:8e:ff:c0").unwrap(),
                data,
                defer: false,
            });
            let msg_s = msg.to_string();
            match tx0.send(msg) {
                Ok(_) => info!(">> SENDING COMMAND: {msg_s}"),
                Err(e) => error!(">> ERROR SENDING COMMAND: {e}"),
            }
            sleep(Duration::from_secs(5)).await;
        }
    });

    // Delay before sending broadcast
    sleep(Duration::from_secs(1)).await;

    // Spawn a task to send broadcast
    tokio::spawn(async move {
        loop {
            let data: heapless::Vec<u8, MAX_DATA_LEN> =
                heapless::Vec::from_slice(b"BROADCAST").unwrap();
            let msg = Msg::Broadcast(BroadcastData {
                id: next_id(),
                data,
                interval: None,
            });
            let msg_s = msg.to_string();
            match tx1.send(msg) {
                Ok(_) => info!(">> SENDING COMMAND: {msg_s}"),
                Err(e) => error!(">> ERROR SENDING COMMAND: {e}"),
            }
            sleep(Duration::from_secs(5)).await;
        }
    });

    ctrl_c().await.expect("Error listening for Ctrl-C");
    info!("Received Ctrl-C: Exiting");

    Ok(())
}
