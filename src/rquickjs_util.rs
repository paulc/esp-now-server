use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use anyhow::anyhow;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;
use tokio::time::Duration;

use rustyline::{error::ReadlineError, DefaultEditor};

use rquickjs::{
    function::Rest,
    function::{Async, Func},
    prelude::IntoArgs,
    CatchResultExt, Ctx, Exception, Function, Module, Object, Value,
};

/// Register TX channel
pub fn register_oneshot<'js, T>(
    ctx: Ctx<'js>,
    tx: oneshot::Sender<T>,
    f: &str,
) -> anyhow::Result<()>
where
    T: rquickjs::IntoJs<'js> + rquickjs::FromJs<'js> + Clone + Send + 'static,
{
    let tx = Arc::new(Mutex::new(Some(tx)));
    ctx.globals().set(
        f,
        Func::new(move |ctx, msg: T| match tx.lock() {
            Ok(mut guard) => match guard.take() {
                Some(tx) => match tx.send(msg) {
                    Ok(_) => Ok::<(), rquickjs::Error>(()),
                    Err(_) => Err::<(), rquickjs::Error>(Exception::throw_message(
                        &ctx,
                        "TX Channel Closed",
                    )),
                },
                None => {
                    Err::<(), rquickjs::Error>(Exception::throw_message(&ctx, "Already Resolved"))
                }
            },
            Err(_) => Err::<(), rquickjs::Error>(Exception::throw_message(&ctx, "Mutex Error")),
        }),
    )?;
    Ok(())
}

/// Register TX channel
pub fn register_tx_channel<'js, T>(
    ctx: Ctx<'js>,
    tx: UnboundedSender<T>,
    f: &str,
) -> anyhow::Result<()>
where
    T: rquickjs::IntoJs<'js> + rquickjs::FromJs<'js> + Clone + Send + 'static,
{
    let tx = Arc::new(Mutex::new(tx));
    ctx.globals().set(
        f,
        Func::new(Async(move |ctx, msg: T| {
            let tx = tx.clone();
            async move {
                match tx
                    .lock()
                    .map_err(|_| Exception::throw_message(&ctx, "Mutex Error"))?
                    .send(msg)
                {
                    Ok(_) => Ok::<(), rquickjs::Error>(()),
                    Err(_) => Err::<(), rquickjs::Error>(Exception::throw_message(
                        &ctx,
                        "TX Channel Closed",
                    )),
                }
            }
        })),
    )?;
    Ok(())
}

/// Register RX channel
pub fn register_rx_channel<'js, T>(
    ctx: Ctx<'js>,
    rx: UnboundedReceiver<T>,
    f: &str,
) -> anyhow::Result<()>
where
    T: rquickjs::IntoJs<'js> + rquickjs::FromJs<'js> + Clone + Send + 'static,
{
    let rx = Arc::new(Mutex::new(rx));
    ctx.globals().set(
        f,
        Func::new(Async(move |ctx| {
            // Pass closure to JS engine
            let rx = rx.clone();
            async move {
                // Returns future when called
                if let Some(msg) = {
                    rx.lock()
                        .map_err(|_e| Exception::throw_message(&ctx, "Mutex Error"))?
                        .recv()
                        .await
                } {
                    Ok::<T, rquickjs::Error>(msg)
                } else {
                    Err::<T, rquickjs::Error>(Exception::throw_message(&ctx, "RX Channel Closed"))
                }
            }
        })),
    )?;
    Ok(())
}

/// Register useful QJS functions
pub fn register_fns(ctx: &Ctx<'_>) -> anyhow::Result<()> {
    ctx.globals().set("__print", js_print)?;
    ctx.globals().set("__print_v", js_print_v)?;
    ctx.globals().set("__sleep", js_sleep)?;
    ctx.globals().set("__globals", js_globals)?;
    ctx.globals().set("__to_buffer", js_to_buffer)?;
    ctx.globals().set("__to_utf8", js_to_utf8)?;
    ctx.globals().set("setTimeout", js_set_timeout)?;
    // Add console.log function
    let console = Object::new(ctx.clone())?;
    console.set("log", js_log)?;
    ctx.globals().set("console", console)?;
    // Add to_utf8 / to_buffer prototype methods
    ctx.eval::<(),_>(r#"
        Object.defineProperty(String.prototype, "to_buffer", { value: function () { return __to_buffer(this) }});
        Object.defineProperty(ArrayBuffer.prototype, "to_utf8", { value: function() { return __to_utf8(this) }});
    "#)?;
    Ok(())
}

/// Print JS String
#[rquickjs::function]
fn print(s: String) {
    println!("{}", s);
}

/// Convert Value to JSON String
pub fn value_to_json<'js>(ctx: Ctx<'js>, v: Value<'js>) -> anyhow::Result<String> {
    if v.is_undefined() {
        Ok("null".into())
    } else {
        ctx.json_stringify(v)?
            .and_then(|s| s.as_string().map(|s| s.to_string().ok()).flatten())
            .ok_or(anyhow::anyhow!("JSON Error"))
    }
}

/// Convert JSON String to Value
pub fn json_to_value<'js>(ctx: Ctx<'js>, json: &str) -> anyhow::Result<Value<'js>> {
    match ctx.json_parse(json.as_bytes()) {
        Ok(v) => Ok(v),
        Err(e) => {
            if let Ok(ex) = rquickjs::Exception::from_value(ctx.catch()) {
                Err(anyhow::anyhow!(
                    "JSON Error: {}\n{}",
                    ex.message().unwrap_or("-".into()),
                    ex.stack().unwrap_or("-".into())
                ))
            } else {
                Err(anyhow::anyhow!("JSON Error: {e}"))
            }
        }
    }
}

/// Print JS Value as JSON
#[rquickjs::function]
pub fn print_v<'js>(ctx: Ctx<'js>, v: Value<'js>) -> rquickjs::Result<()> {
    let output = ctx
        .json_stringify(v)?
        .and_then(|s| s.as_string().map(|s| s.to_string().ok()).flatten())
        .unwrap_or_else(|| "<ERR>".to_string());
    println!("{}", output);
    Ok(())
}

/// Print globals
#[rquickjs::function]
fn globals<'js>(ctx: Ctx<'js>) -> rquickjs::Result<()> {
    let mut i = ctx.globals().props::<String, rquickjs::Value>();
    while let Some(Ok((k, v))) = i.next() {
        println!("{} v = {:?}", k, v);
    }
    Ok(())
}

/// String to ArrayBuffer
/// JS: Object.defineProperty(String.prototype, "to_buffer", { value: function () { return __to_buffer(this) }})
#[rquickjs::function]
fn to_buffer<'js>(ctx: Ctx<'js>, s: String) -> rquickjs::Result<rquickjs::ArrayBuffer<'js>> {
    rquickjs::ArrayBuffer::new_copy(ctx.clone(), s.as_bytes())
}

/// ArrayBuffer to UTF8
/// JS: Object.defineProperty(ArrayBuffer.prototype, "to_utf8", { value: function() { return __to_utf8(this) }})
#[rquickjs::function]
fn to_utf8<'js>(ctx: Ctx<'js>, a: rquickjs::ArrayBuffer<'js>) -> rquickjs::Result<String> {
    let bytes = a
        .as_bytes()
        .ok_or(rquickjs::Exception::throw_message(
            &ctx,
            "Invalid ArrayBuffer",
        ))?
        .to_vec();
    Ok(String::from_utf8(bytes)?)
}

/// console.log
#[rquickjs::function]
fn log<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
    println!(
        "{}",
        args.iter()
            .map(|a| -> rquickjs::Result<String> {
                Ok(ctx
                    .json_stringify(a)?
                    .and_then(|s| s.as_string().map(|s| s.to_string().ok()).flatten())
                    .unwrap_or_else(|| "<ERR>".to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?
            .join(", ")
    );
    Ok(())
}

#[rquickjs::function]
async fn sleep(n: u64) -> rquickjs::Result<()> {
    tokio::time::sleep(Duration::from_secs(n)).await;
    Ok(())
}

#[rquickjs::function]
async fn set_timeout<'js>(
    ctx: Ctx<'js>,
    n: u64,
    f: Function<'js>,
    args: Rest<Value<'js>>,
) -> rquickjs::Result<()> {
    tokio::time::sleep(Duration::from_secs(n)).await;
    let mut arg = rquickjs::function::Args::new(ctx.clone(), args.len());
    arg.push_args(args.iter())?;
    f.call_arg(arg)
}

/// Expand script arg to handle literal script, @file or stdin (-)
pub fn get_script(script: &str) -> anyhow::Result<String> {
    Ok(if script == "-" {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s)?;
        s
    } else if script.starts_with("@") {
        std::fs::read_to_string(&script[1..])?
    } else {
        script.to_string()
    })
}

/// Run as script
pub async fn run_script<'js>(ctx: Ctx<'js>, script: String) -> anyhow::Result<Value<'js>> {
    match ctx.eval::<rquickjs::Value, _>(script) {
        Ok(v) => Ok(v),
        Err(e) => {
            if let Ok(ex) = rquickjs::Exception::from_value(ctx.catch()) {
                Err(anyhow!(
                    "{}\n{}",
                    ex.message().unwrap_or("-".into()),
                    ex.stack().unwrap_or("-".into())
                ))
            } else {
                Err(anyhow!("JS Error: {e}"))
            }
        }
    }
}

/// Run as module
pub async fn run_module(ctx: Ctx<'_>, module: String) -> anyhow::Result<()> {
    // Declare module
    let module = Module::declare(ctx.clone(), "main.mjs", module)
        .catch(&ctx)
        .map_err(|e| anyhow!("JS error [declare]: {}", e))?;

    // Evaluate module
    let (_module, promise) = module
        .eval()
        .catch(&ctx)
        .map_err(|e| anyhow!("JS error [eval]: {}", e))?;

    // Complete promise as future
    promise
        .into_future::<()>()
        .await
        .catch(&ctx)
        .map_err(|e| anyhow!("JS error [await]: {}", e))?;

    Ok(())
}

/// REPL
pub async fn repl(ctx: Ctx<'_>) -> anyhow::Result<()> {
    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);
    loop {
        let script = read_multiline_input(&mut reader).await?;
        if !script.is_empty() {
            match run_script(ctx.clone(), script).await {
                Ok(v) => {
                    if !v.is_undefined() {
                        ctx.globals().set("_", v.clone())?;
                        let _ = print_v(ctx.clone(), v);
                    }
                }
                Err(e) => eprintln!("{e}"),
            }
        }
    }
}

/// REPL
const PROMPT: &str = ">>> ";
const MULTILINE_PROMPT: &str = "... ";

pub async fn repl_rustyline(ctx: Ctx<'_>) -> anyhow::Result<()> {
    let mut rl = DefaultEditor::new()?;
    let mut lines = Vec::new();
    let mut prompt = PROMPT;
    loop {
        match rl.readline(prompt) {
            Ok(line) => {
                lines.push(line.to_string());
                let script = lines.join("\n");
                // Check if we need more input (unmatched braces/parens)
                if needs_more_input(&script) {
                    prompt = MULTILINE_PROMPT;
                } else {
                    if !script.is_empty() {
                        rl.add_history_entry(script.as_str())?;
                        match run_script(ctx.clone(), script).await {
                            Ok(v) => {
                                if !v.is_undefined() {
                                    ctx.globals().set("_", v.clone())?;
                                    let _ = print_v(ctx.clone(), v);
                                }
                            }
                            Err(e) => eprintln!("{e}"),
                        }
                        lines.clear();
                    }
                    prompt = PROMPT;
                };
            }
            Err(ReadlineError::Interrupted) => {
                eprintln!("<CTRL-C>");
                break;
            }
            Err(ReadlineError::Eof) => {
                eprintln!("<CTRL-D>");
                break;
            }
            Err(e) => {
                eprintln!("Error: {:?}", e);
                break;
            }
        }
    }
    Ok(())
}

/// Call JS fn
pub async fn call_fn<'js, A>(ctx: Ctx<'js>, path: &str, args: A) -> anyhow::Result<Value<'js>>
where
    A: IntoArgs<'js>,
{
    let mut obj = ctx.globals();
    for p in path.split(".") {
        obj = obj
            .get::<_, rquickjs::Object>(p)
            .map_err(|e| anyhow::anyhow!("Invalid Path: {p} [{e}]"))?;
    }
    Ok(obj
        .as_function()
        .ok_or(anyhow::anyhow!("{path} not a function"))?
        .call::<A, rquickjs::Value>(args)?)
}

async fn read_multiline_input(reader: &mut BufReader<tokio::io::Stdin>) -> anyhow::Result<String> {
    let mut lines = Vec::new();
    let mut buffer = String::new();

    loop {
        let prompt = if lines.is_empty() { ">>> " } else { "... " };
        print!("{}", prompt);
        std::io::stdout().flush()?;

        buffer.clear();
        reader.read_line(&mut buffer).await?;
        let line = buffer.trim_end();

        lines.push(line.to_string());

        let full_input = lines.join("\n");
        // Check if we need more input (unmatched braces/parens)
        if !needs_more_input(&full_input) {
            return Ok(full_input);
        }
    }
}

fn needs_more_input(input: &str) -> bool {
    let mut balance = 0i32;
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '{' | '(' | '[' => balance += 1,
            '}' | ')' | ']' => {
                balance -= 1;
                if balance < 0 {
                    return false;
                } // Syntax error
            }
            '"' | '\'' => {
                // Skip string literals
                let quote = ch;
                while let Some(c) = chars.next() {
                    if c == '\\' {
                        // Skip escaped chars
                        chars.next();
                    } else if c == quote {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    balance > 0
}
