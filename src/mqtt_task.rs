use rumqttc::{AsyncClient, Event, EventLoop, MqttOptions};
use std::result::Result;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::mqtt_types::{MqttCommand, MqttEvent};

/// Configuration for the MQTT client
#[derive(Debug, Clone)]
pub struct MqttConfig {
    pub broker_addr: String,
    pub broker_port: u16,
    pub client_id: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub keep_alive: Duration,
    pub reconnect_initial_delay: Duration,
    pub reconnect_max_delay: Duration,
    pub reconnect_multiplier: f32,
}

impl Default for MqttConfig {
    fn default() -> Self {
        Self {
            broker_addr: "localhost".to_string(),
            broker_port: 1883,
            client_id: format!("mqtt_client_{}", uuid::Uuid::new_v4()),
            username: None,
            password: None,
            keep_alive: Duration::from_secs(5),
            reconnect_initial_delay: Duration::from_secs(1),
            reconnect_max_delay: Duration::from_secs(60),
            reconnect_multiplier: 2.0,
        }
    }
}

pub struct MqttTask {
    config: MqttConfig,
}

impl MqttTask {
    pub fn new(config: MqttConfig) -> Self {
        Self { config }
    }

    pub async fn start(
        &self,
    ) -> Result<
        (
            mpsc::UnboundedSender<MqttCommand>,
            mpsc::UnboundedReceiver<MqttEvent>,
        ),
        Box<dyn std::error::Error>,
    > {
        // Create channels for command and event communication
        let (command_tx, command_rx) = mpsc::unbounded_channel::<MqttCommand>();
        let (event_tx, event_rx) = mpsc::unbounded_channel::<MqttEvent>();

        // Clone config for the task
        let config = self.config.clone();

        // Spawn the main task
        tokio::spawn(async move {
            let mut command_rx = command_rx;
            let event_tx_clone = event_tx.clone();

            // Connection loop with reconnection capability
            loop {
                // Configure MQTT options
                let mut mqtt_options =
                    MqttOptions::new(&config.client_id, &config.broker_addr, config.broker_port);

                if let Some(ref username) = config.username {
                    mqtt_options
                        .set_credentials(username, config.password.as_deref().unwrap_or(""));
                }

                mqtt_options.set_keep_alive(config.keep_alive);

                // Create the async client
                let (client, event_loop) = AsyncClient::new(mqtt_options, 10);

                // Run the connection until an error occurs
                let should_reconnect = run_connection(
                    client,
                    event_loop,
                    &mut command_rx,
                    &event_tx_clone,
                    &config,
                )
                .await;

                if !should_reconnect {
                    break;
                }

                // Small delay before attempting to reconnect
                tokio::time::sleep(Duration::from_millis(100)).await;
            }

            info!("MQTT task shutting down");
        });

        Ok((command_tx, event_rx))
    }
}

// Separate function to handle a single connection
async fn run_connection(
    client: AsyncClient,
    mut event_loop: EventLoop,
    command_rx: &mut mpsc::UnboundedReceiver<MqttCommand>,
    event_tx: &mpsc::UnboundedSender<MqttEvent>,
    _config: &MqttConfig,
) -> bool {
    let mut connected = false;
    let local_command_rx = command_rx;

    loop {
        tokio::select! {
            // Handle incoming commands
            command = local_command_rx.recv() => {
                match command {
                    Some(MqttCommand::Publish { topic, payload, qos }) => {
                        if connected {
                            if let Err(e) = client.publish(&topic, qos.into(), false, payload).await {
                                let _ = event_tx.send(MqttEvent::Error(format!("Failed to publish: {}", e)));
                            } else {
                                debug!("Published message to topic: {}", topic);
                            }
                        } else {
                            let _ = event_tx.send(MqttEvent::Error("Not connected to broker".to_string()));
                        }
                    }
                    Some(MqttCommand::Subscribe { topic, qos }) => {
                        if connected {
                            if let Err(e) = client.subscribe(&topic, qos.into()).await {
                                let _ = event_tx.send(MqttEvent::Error(format!("Failed to subscribe: {}", e)));
                            } else {
                                debug!("Subscribed to topic: {}", topic);
                            }
                        } else {
                            let _ = event_tx.send(MqttEvent::Error("Not connected to broker".to_string()));
                        }
                    }
                    Some(MqttCommand::Unsubscribe { topic }) => {
                        if connected {
                            if let Err(e) = client.unsubscribe(&topic).await {
                                let _ = event_tx.send(MqttEvent::Error(format!("Failed to unsubscribe: {}", e)));
                            } else {
                                debug!("Unsubscribed from topic: {}", topic);
                            }
                        } else {
                            let _ = event_tx.send(MqttEvent::Error("Not connected to broker".to_string()));
                        }
                    }
                    Some(MqttCommand::Disconnect) => {
                        info!("Disconnect command received");
                        return false; // Don't reconnect
                    }
                    None => {
                        // Command channel closed, exit the loop
                        return false;
                    }
                }
            }
            // Handle MQTT events
            event = event_loop.poll() => {
                match event {
                    Ok(Event::Incoming(packet)) => {
                        match packet {
                            rumqttc::Packet::ConnAck(_) => {
                                connected = true;
                                info!("Connected to MQTT broker");
                                let _ = event_tx.send(MqttEvent::Connected);
                            }
                            rumqttc::Packet::Publish(publish) => {
                                let _ = event_tx.send(MqttEvent::MessageReceived {
                                    topic: publish.topic,
                                    payload: publish.payload.to_vec(),
                                });
                            }
                            rumqttc::Packet::Disconnect => {
                                connected = false;
                                warn!("Disconnected from MQTT broker");
                                let _ = event_tx.send(MqttEvent::Disconnected);
                            }
                            _ => {
                                // Handle other incoming packets as needed
                            }
                        }
                    }
                    Ok(Event::Outgoing(_)) => {
                        // Handle outgoing packets if needed
                    }
                    Err(e) => {
                        error!("MQTT connection error: {}", e);
                        let _ = event_tx.send(MqttEvent::Error(format!("MQTT connection error: {}", e)));

                        // Exit and return true to indicate we should attempt to reconnect
                        return true;
                    }
                }
            }
        }
    }
}
