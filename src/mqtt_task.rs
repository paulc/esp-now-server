use rumqttc::{AsyncClient, Event, EventLoop, MqttOptions};
use serde::{Deserialize, Serialize};
use std::result::Result;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

#[cfg(feature = "js")]
use rquickjs::{class::Trace, ArrayBuffer, Ctx, Exception, JsLifetime, Value};

// We need to define a local QoS enum which is serialisable
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "js", derive(Trace, JsLifetime), rquickjs::class())]
pub enum QoS {
    AtMostOnce,
    AtLeastOnce,
    ExactlyOnce,
}

impl std::fmt::Display for QoS {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QoS::AtMostOnce => write!(f, "AtMostOnce"),
            QoS::AtLeastOnce => write!(f, "AtLeastOnce"),
            QoS::ExactlyOnce => write!(f, "ExactlyOnce"),
        }
    }
}

impl TryFrom<&str> for QoS {
    type Error = ();
    fn try_from(s: &str) -> Result<Self, ()> {
        match s.to_lowercase().as_str() {
            "qos0" | "atmostonce" => Ok(QoS::AtMostOnce),
            "qos1" | "atleastonce" => Ok(QoS::AtLeastOnce),
            "qos2" | "exactlyonce" => Ok(QoS::ExactlyOnce),
            _ => Err(()),
        }
    }
}

// Convert into rumqqtc::QoS
impl From<QoS> for rumqttc::QoS {
    fn from(qos: QoS) -> Self {
        match qos {
            QoS::AtMostOnce => rumqttc::QoS::AtMostOnce,
            QoS::AtLeastOnce => rumqttc::QoS::AtLeastOnce,
            QoS::ExactlyOnce => rumqttc::QoS::ExactlyOnce,
        }
    }
}

#[cfg(feature = "js")]
#[rquickjs::methods]
impl QoS {
    #[qjs(constructor)]
    pub fn new(ctx: Ctx<'_>, qos: String) -> rquickjs::Result<Self> {
        Self::try_from(qos.as_str()).map_err(|_| Exception::throw_message(&ctx, "Invalid QoS"))
    }
    pub fn debug(&self) -> String {
        format!("QoS: {:?}", self)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "js", derive(Trace, JsLifetime), rquickjs::class())]
pub enum MqttCommand {
    /// Publish a message to a topic
    Publish {
        topic: String,
        payload: Vec<u8>,
        qos: QoS,
    },
    /// Subscribe to a topic
    Subscribe { topic: String, qos: QoS },
    /// Unsubscribe from a topic
    Unsubscribe { topic: String },
    /// Disconnect from the MQTT broker
    Disconnect,
}

#[cfg(feature = "js")]
/// JS Value to Rust String
fn v_to_s(ctx: &Ctx<'_>, v: &Value<'_>) -> rquickjs::Result<String> {
    v.as_string()
        .ok_or(Exception::throw_message(ctx, "Invalid String"))?
        .to_string()
}

#[cfg(feature = "js")]
/// JS Value to Buffer (Vec<u8>)
fn v_to_b(ctx: &Ctx<'_>, v: &Value<'_>) -> rquickjs::Result<Vec<u8>> {
    Ok(rquickjs::ArrayBuffer::from_value(v.clone())
        .ok_or(Exception::throw_message(ctx, "Invalid ArrayBuffer"))?
        .as_bytes()
        .unwrap_or(&[])
        .to_vec())
}

#[cfg(feature = "js")]
#[rquickjs::methods]
impl MqttCommand {
    #[qjs(constructor)]
    pub fn new(
        ctx: Ctx<'_>,
        msg_type: String,
        args: rquickjs::function::Rest<Value<'_>>,
    ) -> rquickjs::Result<Self> {
        let err = |s| Exception::throw_message(&ctx, s);
        match msg_type.as_str() {
            "publish" => Ok(MqttCommand::Publish {
                topic: v_to_s(&ctx, args.get(0).ok_or(err("Missing Topic"))?)?,
                payload: v_to_b(&ctx, args.get(1).ok_or(err("Missing Payload"))?)?,
                qos: QoS::try_from(v_to_s(&ctx, args.get(2).ok_or(err("Missing QoS"))?)?.as_str())
                    .map_err(|_| err("Invalid QoS"))?,
            }),
            "subscribe" => Ok(MqttCommand::Subscribe {
                topic: v_to_s(&ctx, args.get(0).ok_or(err("Missing Topic"))?)?,
                qos: QoS::try_from(v_to_s(&ctx, args.get(1).ok_or(err("Missing QoS"))?)?.as_str())
                    .map_err(|_| err("Invalid QoS"))?,
            }),
            "unsubscribe" => Ok(MqttCommand::Unsubscribe {
                topic: v_to_s(&ctx, args.get(0).ok_or(err("Missing Topic"))?)?,
            }),
            "disconnect" => Ok(MqttCommand::Disconnect),
            _ => Err(Exception::throw_message(&ctx, "Invalid Command")),
        }
    }

    #[qjs(get, rename = "type")]
    pub fn get_type(&self) -> String {
        match self {
            MqttCommand::Publish { .. } => "publish".to_string(),
            MqttCommand::Subscribe { .. } => "subscribe".to_string(),
            MqttCommand::Unsubscribe { .. } => "unsubscribe".to_string(),
            MqttCommand::Disconnect => "disconnect".to_string(),
        }
    }

    #[qjs(get, rename = "payload")]
    pub fn get_payload<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        match self {
            MqttCommand::Publish { payload, .. } => {
                Ok(ArrayBuffer::new_copy(ctx.clone(), payload)?
                    .as_value()
                    .clone())
            }
            _ => Ok(rquickjs::Undefined {}.into_value(ctx.clone())),
        }
    }

    #[qjs(get, rename = "payload_utf8")]
    pub fn get_payload_utf8<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        match self {
            MqttCommand::Publish { payload, .. } => {
                let s = String::from_utf8(payload.clone())?;
                Ok(rquickjs::String::from_str(ctx.clone(), s.as_str())?
                    .as_value()
                    .clone())
            }
            _ => Ok(rquickjs::Undefined {}.into_value(ctx.clone())),
        }
    }

    #[qjs(get, rename = "topic")]
    pub fn get_topic<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        match self {
            MqttCommand::Publish { topic, .. }
            | MqttCommand::Subscribe { topic, .. }
            | MqttCommand::Unsubscribe { topic, .. } => {
                Ok(rquickjs::String::from_str(ctx.clone(), topic)?
                    .as_value()
                    .clone())
            }
            MqttCommand::Disconnect => Ok(rquickjs::Undefined {}.into_value(ctx.clone())),
        }
    }

    #[qjs(get, rename = "qos")]
    pub fn get_qos<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        match self {
            MqttCommand::Publish { qos, .. } | MqttCommand::Subscribe { qos, .. } => {
                Ok(rquickjs::String::from_str(ctx.clone(), &qos.to_string())?
                    .as_value()
                    .clone())
            }
            _ => Ok(rquickjs::Undefined {}.into_value(ctx.clone())),
        }
    }

    pub fn debug(&self) -> String {
        format!("MqttCommand: {:?}", self)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "js", derive(Trace, JsLifetime), rquickjs::class())]
pub enum MqttEvent {
    /// A message was received on a subscribed topic
    MessageReceived { topic: String, payload: Vec<u8> },
    /// Successfully connected to the MQTT broker
    Connected,
    /// Disconnected from the MQTT broker
    Disconnected,
    /// An error occurred
    Error(String),
}

/// JS Helpers
#[cfg(feature = "js")]
#[rquickjs::methods]
impl MqttEvent {
    // Only needed to ensure class visible from JS
    #[qjs(constructor)]
    pub fn new() -> Self {
        MqttEvent::Error("INVALID".into())
    }

    #[qjs(get, rename = "type")]
    pub fn get_type(&self) -> String {
        match self {
            MqttEvent::MessageReceived { .. } => "MessageReceived".to_string(),
            MqttEvent::Connected { .. } => "Connected".to_string(),
            MqttEvent::Disconnected { .. } => "Disconnected".to_string(),
            MqttEvent::Error { .. } => "Error".to_string(),
        }
    }

    #[qjs(get, rename = "topic")]
    pub fn get_topic<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        match self {
            MqttEvent::MessageReceived { topic, .. } => {
                let s: rquickjs::String = rquickjs::String::from_str(ctx.clone(), &topic)?;
                Ok(rquickjs::Value::from_string(s))
            }
            _ => Ok(rquickjs::Undefined {}.into_value(ctx.clone())),
        }
    }

    #[qjs(get, rename = "error")]
    pub fn get_error<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        match self {
            MqttEvent::Error(err) => {
                let s: rquickjs::String = rquickjs::String::from_str(ctx.clone(), &err)?;
                Ok(rquickjs::Value::from_string(s))
            }
            _ => Ok(rquickjs::Undefined {}.into_value(ctx.clone())),
        }
    }

    #[qjs(get, rename = "payload")]
    pub fn get_payload<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        match self {
            MqttEvent::MessageReceived { payload, .. } => {
                Ok(ArrayBuffer::new_copy(ctx.clone(), payload)?
                    .as_value()
                    .clone())
            }
            _ => Ok(rquickjs::Undefined {}.into_value(ctx.clone())),
        }
    }

    #[qjs(get, rename = "payload_utf8")]
    pub fn get_payload_utf8<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        match self {
            MqttEvent::MessageReceived { payload, .. } => {
                let s: rquickjs::String =
                    rquickjs::String::from_str(ctx.clone(), &String::from_utf8(payload.clone())?)?;
                Ok(rquickjs::Value::from_string(s))
            }
            _ => Ok(rquickjs::Undefined {}.into_value(ctx.clone())),
        }
    }

    pub fn debug(&self) -> String {
        format!("MqttEvent: {:?}", self)
    }
}

/// Register JS functions/classes
#[cfg(feature = "js")]
pub fn register_mqtt(ctx: &Ctx<'_>) -> anyhow::Result<()> {
    // Create mqtt object
    let mqtt = rquickjs::Object::new(ctx.clone())?;
    // Register classes
    rquickjs::Class::<QoS>::define(&mqtt).unwrap();
    rquickjs::Class::<MqttCommand>::define(&mqtt).unwrap();
    rquickjs::Class::<MqttEvent>::define(&mqtt).unwrap();
    ctx.globals().set("mqtt", mqtt)?;
    Ok(())
}

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
