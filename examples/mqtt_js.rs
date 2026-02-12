use std::sync::atomic::{AtomicBool, Ordering};

use esp_now_server::mqtt_task::{MqttConfig, MqttTask};
use esp_now_server::mqtt_types::register_mqtt;
use esp_now_server::rquickjs_util::{
    call_fn, get_script, json_to_value, register_fns, register_rx_channel, register_tx_channel,
    repl_rustyline, run_module, run_script, value_to_json,
};

use argh::FromArgs;

use rquickjs::{async_with, AsyncContext, AsyncRuntime};

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
    #[argh(option, default = "default_addr()")]
    /// mqtt broker address (default: 127.0.0.1)
    address: String,
    #[argh(option, default = "default_port()")]
    /// mqtt broker port (default: 1883)
    port: u16,
    #[argh(option)]
    /// tick event interval
    _tick: Option<u64>,
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
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    // Get CLI args
    let args: CliArgs = argh::from_env();

    // Check that we have something to do
    if args.script.is_empty() && args.module.is_empty() && args.call.is_empty() && !args.repl {
        let name = std::env::args().next().unwrap_or("-".into());
        CliArgs::from_args(&[&name], &["--help"]).map_err(|exit| anyhow::anyhow!(exit.output))?;
    }

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
    let (command_tx, event_rx) = MqttTask::new(config)
        .start()
        .await
        .map_err(|e| anyhow::anyhow!("MqttTask: {e}"))?;

    let rt = AsyncRuntime::new()?;
    let ctx = AsyncContext::full(&rt).await?;

    command_tx.send(esp_now_server::mqtt_types::MqttCommand::Subscribe {
        topic: "t1".into(),
        qos: "qos0".try_into().unwrap(),
    })?;

    // Create
    async_with!(ctx => |ctx| {
        register_fns(&ctx)?;
        register_mqtt(&ctx)?;
        register_tx_channel(ctx.clone(), command_tx, "mqtt_tx")?;
        register_rx_channel(ctx.clone(), event_rx, "mqtt_rx")?;

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
