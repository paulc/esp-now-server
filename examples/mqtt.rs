use esp_now_server::mqtt_task::{MqttCommand, MqttConfig, MqttEvent, MqttTask, QoS};
use tokio::time::Duration;

#[allow(unused_imports)]
use tracing::{debug, error, info, warn};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    // Create MQTT configuration for testing with our local server
    let config = MqttConfig {
        broker_addr: "127.0.0.1".to_string(),
        broker_port: 1883,
        client_id: "test_client".to_string(),
        username: Some("admin".to_string()),
        password: Some("admin123".to_string()),
        ..Default::default()
    };

    // Start the MQTT task
    let (command_tx, mut event_rx) = MqttTask::new(config).start().await?;

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
