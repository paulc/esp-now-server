use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use esp_now_server::mqtt_task::{MqttCommand, MqttConfig, MqttTask};
use hook::MqttHook;

use tokio::signal::ctrl_c;
use tokio::time::{sleep, Duration};

use argh::FromArgs;
use rhai::{Dynamic, INT};

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
    tick: Option<u64>,
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

static USER_EXIT: AtomicBool = AtomicBool::new(false);

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

    // Start task waiting for Ctrl-C
    tokio::spawn(async move {
        ctrl_c().await.expect("Error listening for Ctrl-C");
        USER_EXIT.store(true, Ordering::Relaxed);
    });

    // Start the MQTT task
    let (command_tx, mut event_rx) = MqttTask::new(config).start().await?;

    // Create Hook Handler
    let hook = Arc::new(MqttHook::new(args.script.clone(), command_tx.clone())?);

    // Call mqtt_init hook
    match hook.mqtt_init() {
        Ok(v) => info!("Called mqtt_init hook: {v:?}"),
        Err(e) => error!("Error calling mqtt_init hook: {e:?}"),
    }

    // Spawn event task
    let hook_clone = Arc::clone(&hook);

    let event_handle = tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            match hook_clone.mqtt_event(Dynamic::from(event)) {
                Ok(v) => info!("Called mqtt_event hook: {v:?}"),
                Err(e) => error!("Error calling mqtt_event hook: {e:?}"),
            }
        }
        /*
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
        */
    });

    let hook_clone = Arc::clone(&hook);
    let timer_handle = if let Some(t) = args.tick {
        Some(tokio::spawn(async move {
            let mut counter = 0_usize;
            loop {
                sleep(Duration::from_secs(t)).await;
                match hook_clone.mqtt_tick(INT::from(counter as i64)) {
                    Ok(v) => info!("Called mqtt_tick hook: {v:?}"),
                    Err(e) => error!("Error calling mqtt_tick hook: {e:?}"),
                }
                counter += 1;
            }
        }))
    } else {
        None
    };

    // Wait for Ctrl-C
    loop {
        if USER_EXIT.load(Ordering::Relaxed) {
            info!("Received Ctrl-C: Exiting");
            // Disconnect
            if command_tx.send(MqttCommand::Disconnect).is_ok() {
                info!("Sent disconnect command");
            }
            // Shut down tasks
            event_handle.abort();
            timer_handle.map(|h| h.abort());
            break;
        }
        if command_tx.is_closed() {
            info!("Commmand Channel Closed: Exiting");
            // Shut down tasks
            event_handle.abort();
            timer_handle.map(|h| h.abort());
            break;
        }
        sleep(Duration::from_secs(1)).await;
    }

    Ok(())
}

mod hook {
    use esp_now_server::mqtt_task::{MqttCommand, MqttEvent, QoS};

    use rhai::{Blob, Dynamic, Engine, EvalAltResult, ImmutableString, Scope, AST, INT};
    use tokio::sync::mpsc::UnboundedSender;

    #[allow(unused_imports)]
    use tracing::{debug, error, info, warn};
    pub struct MqttHook {
        engine: Engine,
        ast: AST,
    }

    impl MqttHook {
        pub fn new(
            script: String,
            command_tx: UnboundedSender<MqttCommand>,
        ) -> Result<Self, Box<EvalAltResult>> {
            let mut engine = Engine::new();
            // Move command_tx into fn
            let _c1 = command_tx.clone();
            engine.register_fn(
                "mqtt_cmd",
                move |cmd: &mut MqttCommand| -> Result<(), Box<EvalAltResult>> {
                    match command_tx.send(cmd.clone()) {
                        Ok(_) => debug!("[HOOK] MQTT Command Sent OK"),
                        Err(e) => error!("[HOOK]  Error sending MQTT Command: {e:?}"),
                    }
                    Ok(())
                },
            );
            engine.register_fn("parse", parse_event);
            engine.register_fn("subscribe_msg", subscribe_msg);
            engine.register_fn("unsubscribe_msg", unsubscribe_msg);
            engine.register_fn("publish_msg", publish_msg);
            engine.register_fn("disconnect_msg", disconnect_msg);
            let ast = if script.starts_with("@") {
                engine.compile_file(script[1..].into())?
            } else {
                engine.compile(script)?
            };
            Ok(Self { engine, ast })
        }
        pub fn mqtt_init(&self) -> Result<Dynamic, Box<EvalAltResult>> {
            if self.ast.iter_functions().any(|m| m.name == "mqtt_init") {
                let mut scope = Scope::new();
                self.engine
                    .call_fn::<Dynamic>(&mut scope, &self.ast, "mqtt_init", ())
            } else {
                Ok(().into())
            }
        }
        pub fn mqtt_event(&self, event: Dynamic) -> Result<(), Box<EvalAltResult>> {
            if self.ast.iter_functions().any(|m| m.name == "mqtt_event") {
                let mut scope = Scope::new();
                self.engine
                    .call_fn::<()>(&mut scope, &self.ast, "mqtt_event", (event,))?;
            }
            Ok(())
        }
        pub fn mqtt_tick(&self, counter: INT) -> Result<(), Box<EvalAltResult>> {
            if self.ast.iter_functions().any(|m| m.name == "mqtt_tick") {
                let mut scope = Scope::new();
                self.engine
                    .call_fn::<()>(&mut scope, &self.ast, "mqtt_tick", (counter,))?;
            }
            Ok(())
        }
    }

    // Parse MqttEvent to OnjectMap
    fn parse_event(m: &mut MqttEvent) -> Result<Dynamic, Box<EvalAltResult>> {
        rhai::serde::to_dynamic(m)
    }

    fn subscribe_msg(
        topic: ImmutableString,
        qos: ImmutableString,
    ) -> Result<MqttCommand, Box<EvalAltResult>> {
        let msg = MqttCommand::Subscribe {
            topic: topic.into(),
            qos: QoS::try_from(qos.as_str()).map_err(|_| runtime_error("Invalid QoS"))?,
        };
        Ok(msg)
    }

    fn unsubscribe_msg(topic: ImmutableString) -> Result<MqttCommand, Box<EvalAltResult>> {
        let msg = MqttCommand::Unsubscribe {
            topic: topic.into(),
        };
        Ok(msg)
    }

    fn disconnect_msg() -> Result<MqttCommand, Box<EvalAltResult>> {
        Ok(MqttCommand::Disconnect)
    }

    fn publish_msg(
        topic: ImmutableString,
        payload: Blob,
        qos: ImmutableString,
    ) -> Result<MqttCommand, Box<EvalAltResult>> {
        let msg = MqttCommand::Publish {
            topic: topic.into(),
            payload,
            qos: QoS::try_from(qos.as_str()).map_err(|_| runtime_error("Invalid QoS"))?,
        };
        Ok(msg)
    }

    fn runtime_error<'a>(e: &'a str) -> Box<EvalAltResult> {
        Box::new(EvalAltResult::ErrorRuntime(e.into(), rhai::Position::NONE))
    }
}
