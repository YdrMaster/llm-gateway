mod logger;

use llm_gateway::{build, serve};
use llm_gateway_config::GatewayConfig;
use log::warn;
use std::{env, fs};
use tokio::task::JoinSet;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = env::args().nth(1);
    let config = fs::read_to_string(config.as_deref().unwrap_or("config.toml"))?;
    let config: GatewayConfig = config.parse()?;

    logger::init(log::LevelFilter::Debug);
    log::info!("{config:#?}");

    let inputs = build(&config);
    if inputs.is_empty() {
        log::warn!("No input node in config");
        return Ok(());
    }

    tokio::runtime::Runtime::new()
        .expect("Failed to create Tokio runtime")
        .block_on(async move {
            let mut set = JoinSet::new();
            for input in inputs {
                set.spawn(async move {
                    if let Err(e) = serve(&input).await {
                        warn!("input node stopped: {e}")
                    }
                });
            }
            set.join_all().await
        });

    Ok(())
}
