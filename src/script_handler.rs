use esp_now_protocol::Msg;
use rhai::{Dynamic, Engine, EvalAltResult, Scope, AST};

pub struct ScriptHandler {
    engine: Engine,
    ast: AST,
}

impl ScriptHandler {
    pub fn new(script: String) -> Result<Self, Box<EvalAltResult>> {
        let mut engine = Engine::new();
        engine.register_fn("parse", parse_msg);
        let ast = if script.starts_with("@") {
            engine.compile_file(script[1..].into())?
        } else {
            engine.compile(script)?
        };
        Ok(Self { engine, ast })
    }
    pub fn on_init(&self, msg: Dynamic) -> Result<(), Box<EvalAltResult>> {
        if self.ast.iter_functions().any(|m| m.name == "on_init") {
            let mut scope = Scope::new();
            self.engine
                .call_fn::<()>(&mut scope, &self.ast, "on_init", (msg,))?;
        }
        Ok(())
    }
    pub fn on_event(&self, msg: Dynamic) -> Result<(), Box<EvalAltResult>> {
        if self.ast.iter_functions().any(|m| m.name == "on_event") {
            let mut scope = Scope::new();
            self.engine
                .call_fn::<()>(&mut scope, &self.ast, "on_event", (msg,))?;
        }
        Ok(())
    }
}

fn parse_msg(m: &mut Msg) -> Result<Dynamic, Box<EvalAltResult>> {
    rhai::serde::to_dynamic(m)
}
