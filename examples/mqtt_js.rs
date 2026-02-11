use argh::FromArgs;

use esp_now_server::mqtt_task::register_mqtt;
use esp_now_server::rquickjs_util::{
    call_fn, get_script, json_to_value, register_fns, register_oneshot, repl_rustyline, run_module,
    run_script, value_to_json,
};

use rquickjs::{async_with, AsyncContext, AsyncRuntime};

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

/// Basic CLI test
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: CliArgs = argh::from_env();

    // Check that we have something to do
    if args.script.is_empty() && args.module.is_empty() && args.call.is_empty() && !args.repl {
        let name = std::env::args().next().unwrap_or("-".into());
        CliArgs::from_args(&[&name], &["--help"]).map_err(|exit| anyhow::anyhow!(exit.output))?;
    }

    let rt = AsyncRuntime::new()?;
    let ctx = AsyncContext::full(&rt).await?;

    let (oneshot_tx, oneshot_rx) = tokio::sync::oneshot::channel::<String>();

    tokio::spawn(async move {
        match oneshot_rx.await {
            Ok(msg) => {
                println!("[+] Oneshot Resolved -> {msg}");
            }
            Err(_) => eprintln!("[-] Oneshot Channel Closed"),
        }
    });

    async_with!(ctx => |ctx| {
        register_fns(&ctx)?;
        register_mqtt(&ctx)?;
        register_oneshot(ctx.clone(), oneshot_tx, "resolve")?;

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
            repl_rustyline(ctx.clone()).await?;
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

    rt.idle().await;

    Ok(())
}
