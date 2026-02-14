use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use esp_now_server::rquickjs_util::{
    call_fn, get_script, json_to_value, register_fns, repl_rl, run_module, run_script,
    value_to_json,
};

use argh::FromArgs;

use rquickjs::{
    async_with, function::Rest, AsyncContext, AsyncRuntime, Exception, Function, Value,
};

use tokio::signal::ctrl_c;

#[derive(FromArgs)]
/// CLI Args
struct CliArgs {
    #[argh(option)]
    /// QJS script
    script: Vec<String>,
    #[argh(option)]
    /// QJS module
    module: Vec<String>,
    #[argh(switch)]
    /// JS REPL
    repl: bool,
    #[argh(option)]
    /// call JS
    call: Vec<String>,
    #[argh(option)]
    /// call args
    arg: Vec<String>,
}

static USER_EXIT: AtomicBool = AtomicBool::new(false);

#[rquickjs::function]
fn set_timeout<'js>(
    ctx: rquickjs::Ctx<'js>,
    f: rquickjs::Function<'js>,
    delay_ms: u64,
    args: Rest<Value<'js>>,
) -> rquickjs::Result<Function<'js>> {
    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();

    // Need to move cancel_tx into Arc<Mutex<Option>>> to ensure that cancel_f if Fn
    let cancel_tx = Arc::new(Mutex::new(Some(cancel_tx)));
    let cancel_f = Function::new(ctx.clone(), move |ctx| -> rquickjs::Result<()> {
        cancel_tx
            .lock()
            .map_err(|_| Exception::throw_message(&ctx, "Mutex Locked"))?
            .take()
            .ok_or_else(|| Exception::throw_message(&ctx, "Already Cancelled"))?
            .send(())
            .map_err(|_| Exception::throw_message(&ctx, "Oneshot Channel Closed"))
    })?;

    let mut arg = rquickjs::function::Args::new(ctx.clone(), args.len());
    arg.push_args(args.iter())?;

    let _handle = ctx.spawn({
        async move {
            tokio::select! {
                _ = tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)) => {
                    let _ = f.call_arg::<()>(arg);
                }
                Ok(()) = cancel_rx => {
                }
            }
        }
    });

    Ok(cancel_f)
}

#[rquickjs::function]
fn set_interval<'js>(
    ctx: rquickjs::Ctx<'js>,
    f: rquickjs::Function<'js>,
    delay_ms: u64,
    args: Rest<Value<'js>>,
) -> rquickjs::Result<Function<'js>> {
    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();

    // Need to move cancel_tx into Arc<Mutex<Option<_>>>> to ensure that cancel_f closure is Fn
    let cancel_tx = Arc::new(Mutex::new(Some(cancel_tx)));
    let cancel_f = Function::new(ctx.clone(), move |ctx| -> rquickjs::Result<()> {
        cancel_tx
            .lock()
            .map_err(|_| Exception::throw_message(&ctx, "Mutex Locked"))?
            .take()
            .ok_or_else(|| Exception::throw_message(&ctx, "Already Cancelled"))?
            .send(())
            .map_err(|_| Exception::throw_message(&ctx, "Oneshot Channel Closed"))
    })?;

    let _handle = ctx.spawn({
        let ctx = ctx.clone();
        async move {
            let mut cancel_rx = cancel_rx;
            loop {
                let mut arg = rquickjs::function::Args::new(ctx.clone(), args.len());
                let _ = arg.push_args(args.iter());
                tokio::select! {
                    _ = tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)) => {
                        let _ = f.call_arg::<()>(arg);
                    }
                    Ok(()) = &mut cancel_rx => {
                        break;
                    }
                }
            }
        }
    });

    Ok(cancel_f)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    // Get CLI args
    let args: CliArgs = argh::from_env();

    // Check that we have something to do
    if args.script.is_empty() && args.module.is_empty() && args.call.is_empty() && !args.repl {
        let name = std::env::args().next().unwrap_or("-".into());
        CliArgs::from_args(&[&name], &["--help"]).map_err(|exit| anyhow::anyhow!(exit.output))?;
    }

    // Start task waiting for Ctrl-C
    tokio::spawn(async move {
        ctrl_c().await.expect("Error listening for Ctrl-C");
        println!("[+] User Exit",);
        USER_EXIT.store(true, Ordering::Relaxed);
    });

    let rt = AsyncRuntime::new()?;
    let ctx = AsyncContext::full(&rt).await?;

    // Set interrupt handler - this only seems to be called on ctx.eval() so not actually useful
    rt.set_interrupt_handler(Some(Box::new(|| USER_EXIT.load(Ordering::Relaxed))))
        .await;

    // Create
    async_with!(ctx => |ctx| {
        register_fns(&ctx)?;
        ctx.globals().set("setTimeout", js_set_timeout)?;
        ctx.globals().set("setInterval", js_set_interval)?;

        // Run modules
        for module in args.module {
            run_module(ctx.clone(),get_script(&module)?).await?;
        }

        // Run scripts
        for script in args.script {
            run_script(ctx.clone(),get_script(&script)?).await?;
        }

        // Run REPL
        if args.repl {
            repl_rl(ctx.clone()).await?;
        }

        // Call JS
        for (f,a) in args.call.iter().zip(args.arg.iter().chain(std::iter::repeat(&("".to_string())))) {
            let r = if a.is_empty() {
                call_fn(ctx.clone(),&f,((),)).await?
            } else {
                call_fn(ctx.clone(),&f,(json_to_value(ctx.clone(),a)?,)).await?
            };
            println!("[+] Call: {f}({a}) => {}", value_to_json(ctx.clone(),r)?);
        }

        Ok::<(),anyhow::Error>(())
    })
    .await?;

    println!("[+] Tasks Pending: {:?}", rt.is_job_pending().await);

    // Complete pending tasks - use rt.execute_pending_job() rather than rt.idle() to allow
    // USER_EXIT to interrupt
    while rt.is_job_pending().await && !USER_EXIT.load(Ordering::Relaxed) {
        rt.execute_pending_job()
            .await
            .map_err(|_| anyhow::anyhow!("JS Runtime Error"))?;
        // Make sure we yield (possibly not necessary)
        tokio::task::yield_now().await;
    }

    Ok(())
}
