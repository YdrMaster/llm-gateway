mod logger;

use llm_gateway::{StatisticsConfig, StatsStoreManager, build, serve};
use llm_gateway_config::GatewayConfig;
use log::warn;
use std::{env, fs, sync::Arc};
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
            // 在 runtime 内初始化统计模块
            let stats = init_statistics(&config.statistics).await?;

            let mut set = JoinSet::new();
            for input in inputs {
                let stats = stats.clone();
                set.spawn(async move {
                    if let Err(e) = serve(&input, stats).await {
                        warn!("input node stopped: {e}")
                    }
                });
            }
            set.join_all().await;
            Ok(())
        })
}

/// 初始化统计模块
async fn init_statistics(
    config: &Option<StatisticsConfig>,
) -> Result<Option<Arc<StatsStoreManager>>, Box<dyn std::error::Error>> {
    match config {
        Some(config) => {
            if !config.enabled {
                log::info!("Statistics disabled");
                return Ok(None);
            }
            let store = StatsStoreManager::new(config).await?;
            log::info!("Statistics enabled, db_path={}", config.db_path);
            Ok(Some(Arc::new(store)))
        }
        None => {
            log::info!("Statistics not configured, using defaults (disabled)");
            Ok(None)
        }
    }
}
