use crate::mqtt_task::MqttCommand;
use crate::serial_task::next_id;

use esp_now_protocol::{BroadcastData, Msg, TxData, MAX_DATA_LEN};

use rhai::{Array, Blob, Dynamic, Engine, EvalAltResult, ImmutableString, Scope, AST, INT};
use tokio::sync::mpsc::UnboundedSender;

#[allow(unused_imports)]
use tracing::{debug, error, info, warn};

pub struct Hook {
    engine: Engine,
    ast: AST,
}

impl Hook {
    pub fn new(
        script: String,
        command_tx: UnboundedSender<Msg>,
    ) -> Result<Self, Box<EvalAltResult>> {
        let mut engine = Engine::new();
        engine.register_fn("parse", parse_msg);
        engine.register_fn("to_msg", to_msg);
        engine.register_fn("to_blob", to_blob);
        engine.register_fn("format_mac", format_mac);
        engine.register_fn("from_mac", from_mac);
        engine.register_fn("next_id", next_id);
        engine.register_fn("new_txdata", new_txdata);
        engine.register_fn("new_broadcast", new_broadcast);
        engine.register_fn("new_broadcast", new_broadcast_with_interval);
        // Move command_tx into fn
        engine.register_fn(
            "command_tx",
            move |msg: &mut Msg| -> Result<(), Box<EvalAltResult>> {
                match command_tx.send(msg.clone()) {
                    Ok(_) => debug!("[HOOK] Msg Sent OK"),
                    Err(e) => error!("[HOOK]  Error sending Msg: {e:?}"),
                }
                Ok(())
            },
        );
        let ast = if script.starts_with("@") {
            engine.compile_file(script[1..].into())?
        } else {
            engine.compile(script)?
        };
        Ok(Self { engine, ast })
    }
    pub fn on_init(&self) -> Result<Dynamic, Box<EvalAltResult>> {
        if self.ast.iter_functions().any(|m| m.name == "on_init") {
            let mut scope = Scope::new();
            self.engine
                .call_fn::<Dynamic>(&mut scope, &self.ast, "on_init", ())
        } else {
            Ok(().into())
        }
    }
    pub fn on_event(&self, msg: Dynamic) -> Result<(), Box<EvalAltResult>> {
        if self.ast.iter_functions().any(|m| m.name == "on_event") {
            let mut scope = Scope::new();
            self.engine
                .call_fn::<()>(&mut scope, &self.ast, "on_event", (msg,))?;
        }
        Ok(())
    }
    pub fn on_tick(&self, counter: INT) -> Result<(), Box<EvalAltResult>> {
        if self.ast.iter_functions().any(|m| m.name == "on_tick") {
            let mut scope = Scope::new();
            self.engine
                .call_fn::<()>(&mut scope, &self.ast, "on_tick", (counter,))?;
        }
        Ok(())
    }
}

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
        let ast = if script.starts_with("@") {
            engine.compile_file(script[1..].into())?
        } else {
            engine.compile(script)?
        };
        Ok(Self { engine, ast })
    }
    pub fn on_init(&self) -> Result<Dynamic, Box<EvalAltResult>> {
        if self.ast.iter_functions().any(|m| m.name == "on_init") {
            let mut scope = Scope::new();
            self.engine
                .call_fn::<Dynamic>(&mut scope, &self.ast, "on_init", ())
        } else {
            Ok(().into())
        }
    }
    pub fn on_tick(&self, counter: INT) -> Result<(), Box<EvalAltResult>> {
        if self.ast.iter_functions().any(|m| m.name == "on_tick") {
            let mut scope = Scope::new();
            self.engine
                .call_fn::<()>(&mut scope, &self.ast, "on_tick", (counter,))?;
        }
        Ok(())
    }
}

// Msg from Object-Map
fn to_msg(m: &mut rhai::Dynamic) -> Result<Msg, Box<EvalAltResult>> {
    rhai::serde::from_dynamic(m)
}

// Object-Map from Msg
fn parse_msg(m: &mut Msg) -> Result<Dynamic, Box<EvalAltResult>> {
    rhai::serde::to_dynamic(m)
}

fn to_blob(arr: &mut Array) -> Result<Blob, Box<EvalAltResult>> {
    arr.iter()
        .map(|b| {
            b.as_int()
                .and_then(|i| u8::try_from(i).map_err(|_| "Invalid Byte"))
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| runtime_error(e))
}

fn format_mac(mac: &mut Array) -> Result<ImmutableString, Box<EvalAltResult>> {
    let hex = mac
        .iter()
        .take(6)
        .map(|b| b.as_int().and_then(|i| Ok(format!("{:02x}", i))))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ImmutableString::from(hex.join(":")))
}

fn from_mac(s: &mut ImmutableString) -> Result<Array, Box<EvalAltResult>> {
    s.split(":")
        .chain(std::iter::repeat("0"))
        .take(6)
        .map(|b| u8::from_str_radix(b, 16).and_then(|v| Ok(Dynamic::from(v))))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| runtime_error("Parse Error"))
}

fn new_txdata(dest: ImmutableString, data: Blob, defer: bool) -> Result<Msg, Box<EvalAltResult>> {
    Ok(Msg::Send(TxData {
        id: next_id(),
        dst_addr: esp_now_protocol::format_mac::from_mac(&dest)
            .map_err(|_| runtime_error("Invalid MAC address"))?,
        data: heapless::Vec::<u8, MAX_DATA_LEN>::from_slice(&data)
            .map_err(|e| runtime_error(&e.to_string()))?,
        defer,
    }))
}

fn new_broadcast(data: Blob) -> Result<Msg, Box<EvalAltResult>> {
    Ok(Msg::Broadcast(BroadcastData {
        id: next_id(),
        data: heapless::Vec::<u8, MAX_DATA_LEN>::from_slice(&data)
            .map_err(|e| runtime_error(&e.to_string()))?,
        interval: None,
    }))
}

fn new_broadcast_with_interval(data: Blob, interval: INT) -> Result<Msg, Box<EvalAltResult>> {
    Ok(Msg::Broadcast(BroadcastData {
        id: next_id(),
        data: heapless::Vec::<u8, MAX_DATA_LEN>::from_slice(&data)
            .map_err(|e| runtime_error(&e.to_string()))?,
        interval: Some(interval as u32),
    }))
}

fn runtime_error<'a>(e: &'a str) -> Box<EvalAltResult> {
    Box::new(EvalAltResult::ErrorRuntime(e.into(), rhai::Position::NONE))
}
