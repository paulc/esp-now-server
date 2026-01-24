use std::sync::Arc;

use esp_now_server::hook::MqttHook;
use esp_now_server::mqtt_task::{MqttCommand, MqttConfig, MqttEvent, MqttTask, QoS};

use tokio::time::Duration;

use argh::FromArgs;

#[allow(unused_imports)]
use tracing::{debug, error, info, warn};

#[derive(FromArgs)]
/// MQTT Test
struct CliArgs {
    #[argh(option)]
    /// RHAI script
    script: String,
    #[argh(option, default = "default_addr()")]
    /// mqtt broker address (default: 127.0.0.1)
    address: String,
    #[argh(option, default = "default_port()")]
    /// mqtt broker port (default: 1883)
    port: u16,
    #[argh(option)]
    /// tick event interval
    _tick: Option<u64>,
    #[argh(option)]
    /// mqtt username
    username: Option<String>,
    #[argh(option)]
    /// mqtt password
    password: Option<String>,
    #[argh(option)]
    /// mqtt client-id
    client_id: Option<String>,
}

fn default_addr() -> String {
    "127.0.0.1".into()
}

fn default_port() -> u16 {
    1883
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    // Get CLI Args
    let args: CliArgs = argh::from_env();

    // Create MQTT configuration
    let config = MqttConfig {
        broker_addr: args.address,
        broker_port: args.port,
        client_id: args
            .client_id
            .unwrap_or(format!("mqtt_client_{}", uuid::Uuid::new_v4())),
        username: args.username,
        password: args.password,
        ..Default::default()
    };

    // Start the MQTT task
    let (command_tx, mut event_rx) = MqttTask::new(config).start().await?;

    // Create Hook Handler
    let hook = Arc::new(MqttHook::new(args.script.clone(), command_tx.clone())?);

    // Call init method
    match hook.on_init() {
        Ok(v) => info!("ON_INIT: {v:?}"),
        Err(e) => error!("ON_INIT: {e:?}"),
    }

    let command_tx_task = command_tx.clone();

    // Spawn a task to handle events
    tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            match event {
                MqttEvent::Connected => {
                    info!("MQTT client connected");

                    // Subscribe to a test topic after connecting
                    if command_tx_task
                        .send(MqttCommand::Subscribe {
                            topic: "test/topic".to_string(),
                            qos: QoS::AtMostOnce,
                        })
                        .is_ok()
                    {
                        info!("Sent subscription to test/topic");
                    }
                }
                MqttEvent::Disconnected => {
                    info!("MQTT client disconnected");
                }
                MqttEvent::MessageReceived { topic, payload } => {
                    info!(
                        "Received message on topic '{}': {:?}",
                        topic,
                        String::from_utf8_lossy(&payload)
                    );
                }
                MqttEvent::Error(error) => {
                    error!("MQTT client error: {}", error);
                }
            }
        }
    });

    // Publish a test message after a delay
    tokio::time::sleep(Duration::from_secs(2)).await;

    if command_tx
        .send(MqttCommand::Publish {
            topic: "test/topic".to_string(),
            payload: b"Hello from MQTT client!".to_vec(),
            qos: QoS::AtMostOnce,
        })
        .is_ok()
    {
        info!("Sent test message to test/topic");
    }

    // Keep the program running for a while to see the results
    tokio::time::sleep(Duration::from_secs(10)).await;

    // Disconnect
    if command_tx.send(MqttCommand::Disconnect).is_ok() {
        info!("Sent disconnect command");
    }

    Ok(())
}
