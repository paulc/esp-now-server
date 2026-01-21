use esp_now_protocol::{
    format_mac::from_mac, BroadcastData, InitConfig, Msg, TxData, MAX_DATA_LEN,
};

use esp_now_server::script_handler::ScriptHandler;
use esp_now_server::serial_task::{next_id, SerialTask};

use tokio::signal::ctrl_c;
use tokio::time::{sleep, Duration};
use tokio_stream::wrappers::WatchStream;
use tokio_stream::StreamExt;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use rhai::Dynamic;

use argh::FromArgs;

#[derive(FromArgs)]
/// ESP-NOW Bridge
struct CliArgs {
    #[argh(option, short = 't')]
    /// TTY path
    tty: String,
    #[argh(option, short = 's')]
    /// RHAI handler script [@filename to read from file]
    script: String,
    #[argh(option, default = "default_baud()")]
    /// baud rate (default: 115200)
    baud: u32,
    #[argh(option, default = "default_timeout()")]
    /// ack timeout (default 2s)
    ack_timeout: u64,
}

fn default_baud() -> u32 {
    115200
}

fn default_timeout() -> u64 {
    2
}

static USER_EXIT: AtomicBool = AtomicBool::new(false);

#[allow(unused_imports)]
use tracing::{debug, error, info, warn};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_file(true)
        .with_line_number(true)
        .init();

    // Get CLI Args
    let args: CliArgs = argh::from_env();

    // Start task waiting for Ctrl-C
    tokio::spawn(async move {
        ctrl_c().await.expect("Error listening for Ctrl-C");
        USER_EXIT.store(true, Ordering::Relaxed);
    });

    // Start Serial task
    let serial = SerialTask::new(args.tty, args.baud, 6);

    // Create Script Handler
    let script = Arc::new(ScriptHandler::new(args.script)?);

    loop {
        match serial.start().await {
            Err(e) => {
                error!(
                    "SERIAL ->> Error connecting to serial port: {} -> {e}",
                    serial.tty_path
                );
                sleep(Duration::from_secs(1)).await;
            }

            Ok((command_tx, mut event_rx, monitor_rx)) => {
                info!("SERIAL ->> Connected to serial port: {}", serial.tty_path);

                // XXX
                let msg = Msg::Init(InitConfig {
                    id: 99,
                    api_version: 1,
                    now_version: 2,
                    channel: 11,
                    address: from_mac("00:11:22:33:44:55").unwrap(),
                });

                let status = script.on_init(Dynamic::from(msg));
                info!(">>> ON_INIT: {status:?}");

                // Spawn monitor task
                let monitor_handle = tokio::spawn(async move {
                    let mut ws = WatchStream::new(monitor_rx);
                    loop {
                        match ws.next().await {
                            Some(Some(m)) => info!("MONITOR ->> {}", m),
                            Some(None) => info!("MONITOR ->> WAITING"),
                            None => break,
                        }
                    }
                });

                // Spawn a task to handle events
                let script_clone = Arc::clone(&script);
                let event_handle = tokio::spawn(async move {
                    while let Some(msg) = event_rx.recv().await {
                        info!("RECEIVED EVENT ->> {msg}");
                        let _ = script_clone.on_event(Dynamic::from(msg));
                    }
                });

                let tx0 = command_tx.clone();
                let tx1 = command_tx.clone();

                // Spawn a task to send msgs
                let msg_handle = tokio::spawn(async move {
                    loop {
                        let data: heapless::Vec<u8, MAX_DATA_LEN> =
                            heapless::Vec::from_slice(b"TEST").unwrap();
                        let msg = Msg::Send(TxData {
                            id: next_id(),
                            dst_addr: from_mac("98:a3:16:8e:ff:c0").unwrap(),
                            data,
                            defer: false,
                        });
                        let msg_s = msg.to_string();
                        match tx0.send(msg) {
                            Ok(_) => info!("SENDING COMMAND ->> {msg_s}"),
                            Err(e) => error!("ERROR SENDING COMMAND ->> {e}"),
                        }
                        sleep(Duration::from_secs(5)).await;
                    }
                });

                // Delay before sending broadcast
                sleep(Duration::from_secs(1)).await;

                // Spawn a task to send broadcast
                let broadcast_handle = tokio::spawn(async move {
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
                            Ok(_) => info!("SENDING COMMAND ->> {msg_s}"),
                            Err(e) => error!("ERROR SENDING COMMAND ->> {e}"),
                        }
                        sleep(Duration::from_secs(5)).await;
                    }
                });

                // Wait for Ctrl-C
                loop {
                    if USER_EXIT.load(Ordering::Relaxed) {
                        info!("Received Ctrl-C: Exiting");
                        break;
                    }
                    if command_tx.is_closed() {
                        info!("Commmand Channel Closed: Exiting");
                        // Shut down tasks
                        event_handle.abort();
                        msg_handle.abort();
                        broadcast_handle.abort();
                        monitor_handle.abort();
                        break;
                    }
                    sleep(Duration::from_secs(1)).await;
                }
            }
        }

        if USER_EXIT.load(Ordering::Relaxed) {
            info!("Received Ctrl-C: Exiting");
            break;
        }
    }

    Ok(())
}
