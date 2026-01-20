use rhai::{Engine, EvalAltResult, ParseError, AST};

pub struct ScriptHandler {
    engine: Engine,
    init_handler: Option<AST>,
    timer_handler: Option<AST>,
    rx_handler: Option<AST>,
}

impl ScriptHandler {
    pub fn new(
        init_script: Option<String>,
        timer_script: Option<String>,
        rx_script: Option<String>,
    ) -> Result<Self, ParseError> {
        let engine = Engine::new();
        let init_handler = init_script.map(|s| engine.compile(s)).transpose()?;
        let timer_handler = timer_script.map(|s| engine.compile(s)).transpose()?;
        let rx_handler = rx_script.map(|s| engine.compile(s)).transpose()?;
        Ok(Self {
            engine,
            init_handler,
            timer_handler,
            rx_handler,
        })
    }
    pub fn call_init(&self) -> Result<(), Box<EvalAltResult>> {
        self.init_handler
            .as_ref()
            .map_or_else(|| Ok(()), |ast| self.engine.run_ast(ast))
    }
}
