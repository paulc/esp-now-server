use serde::{Deserialize, Serialize};
use std::result::Result;

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
    pub fn get_payload<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<ArrayBuffer<'js>> {
        match self {
            MqttCommand::Publish { payload, .. } => {
                Ok(ArrayBuffer::new_copy(ctx.clone(), payload)?)
            }
            _ => Err(Exception::throw_message(&ctx, "Invalid Type")),
        }
    }

    #[qjs(get, rename = "payload_utf8")]
    pub fn get_payload_utf8<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<String> {
        match self {
            MqttCommand::Publish { payload, .. } => Ok(String::from_utf8(payload.clone())?),
            _ => Err(Exception::throw_message(&ctx, "Invalid Type")),
        }
    }

    #[qjs(get, rename = "topic")]
    pub fn get_topic<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<String> {
        match self {
            MqttCommand::Publish { topic, .. }
            | MqttCommand::Subscribe { topic, .. }
            | MqttCommand::Unsubscribe { topic, .. } => Ok(topic.clone()),
            _ => Err(Exception::throw_message(&ctx, "Invalid Type")),
        }
    }

    #[qjs(get, rename = "qos")]
    pub fn get_qos<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<String> {
        match self {
            MqttCommand::Publish { qos, .. } | MqttCommand::Subscribe { qos, .. } => {
                Ok(qos.to_string())
            }
            _ => Err(Exception::throw_message(&ctx, "Invalid Type")),
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
