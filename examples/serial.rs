use esp_now_protocol::{format_mac::from_mac, Msg, TxData, MAX_DATA_LEN};
use esp_now_server::serial_task::{next_id, SerialTask};

use tokio::signal::ctrl_c;
use tokio::time::{sleep, Duration};

#[allow(unused_imports)]
use tracing::{debug, error, info, warn};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    // Start Serial task
    let serial = SerialTask::new("/dev/tty.usbmodem3101".into(), 115200);

    let (command_tx, mut event_rx) = serial.start().await?;

    // Spawn a task to handle events
    tokio::spawn(async move {
        while let Some(msg) = event_rx.recv().await {
            info!(">> RX Msg: {msg}");
        }
    });

    // Spawn a task to send msgs
    tokio::spawn(async move {
        loop {
            let data: heapless::Vec<u8, MAX_DATA_LEN> = heapless::Vec::from_slice(b"TEST").unwrap();
            let msg = Msg::Send(TxData {
                id: next_id(),
                dst_addr: from_mac("12:34:56:78:9a:bc").unwrap(),
                data,
                defer: false,
            });
            match command_tx.send(msg) {
                Ok(_) => info!(">> Sent TxData"),
                Err(e) => error!(">> ERROR: {e}"),
            }
            sleep(Duration::from_secs(5)).await;
        }
    });

    ctrl_c().await.expect("Error listening for Ctrl-C");
    info!("Received Ctrl-C: Exiting");

    Ok(())
}
