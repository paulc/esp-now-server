use esp_now_protocol::{
    format_mac::from_mac, BroadcastData, InitConfig, Msg, TxData, MAX_DATA_LEN,
};

use esp_now_server::hook::Hook;
use esp_now_server::serial_task::{next_id, SerialTask};

use tokio::signal::ctrl_c;
use tokio::time::{sleep, Duration};
use tokio_stream::wrappers::WatchStream;
use tokio_stream::StreamExt;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use rhai::{Dynamic, INT};

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
    #[argh(option)]
    /// tick event interval
    tick: Option<u64>,
    #[argh(option, default = "default_baud()")]
    /// baud rate (default: 115200)
    baud: u32,
    #[argh(option, default = "default_ack_timeout()")]
    /// ack timeout (default 2s)
    ack_timeout: u64,
}

fn default_baud() -> u32 {
    115200
}

fn default_ack_timeout() -> u64 {
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

                // Create Hook Handler
                let hook = Arc::new(Hook::new(args.script.clone(), command_tx.clone())?);

                // Call init method
                match hook.on_init() {
                    Ok(v) => info!("ON_INIT: {v:?}"),
                    Err(e) => error!("ON_INIT: {e:?}"),
                }

                // Spawn a task to handle serial rx events
                let hook_clone = Arc::clone(&hook);
                let event_handle = tokio::spawn(async move {
                    while let Some(msg) = event_rx.recv().await {
                        info!("RECEIVED EVENT ->> {msg}");
                        match hook_clone.on_event(Dynamic::from(msg)) {
                            Ok(v) => info!("ON_EVENT: {v:?}"),
                            Err(e) => error!("ON_EVENT: {e:?}"),
                        }
                    }
                });

                let hook_clone = Arc::clone(&hook);
                let timer_handle = if let Some(t) = args.tick {
                    Some(tokio::spawn(async move {
                        let mut counter = 0_usize;
                        loop {
                            sleep(Duration::from_secs(t)).await;
                            match hook_clone.on_tick(INT::from(counter as i64)) {
                                Ok(v) => info!("ON_TICK: {v:?}"),
                                Err(e) => error!("ON_TICK: {e:?}"),
                            }
                            counter += 1;
                        }
                    }))
                } else {
                    None
                };

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
                        timer_handle.map(|h| h.abort());
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
