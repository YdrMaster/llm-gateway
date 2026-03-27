mod logger;

use llm_gateway::{StatisticsConfig, StatsStoreManager, build, serve};
use llm_gateway_config::GatewayConfig;
use log::{LevelFilter, info, warn};
use std::{env, fs, sync::Arc};
use tokio::task::JoinSet;

/// 主入口函数
///
/// 读取配置文件，初始化日志和统计模块，启动 HTTP 服务器
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 读取配置文件路径（默认为 config.toml）
    let config = env::args().nth(1);
    let config = fs::read_to_string(config.as_deref().unwrap_or("config.toml"))?;
    let config: GatewayConfig = config.parse()?;

    // 初始化日志系统
    logger::init(LevelFilter::Debug);
    info!("{config:?}");

    // 构建节点图
    let inputs = build(&config);
    if inputs.is_empty() {
        warn!("No input node in config");
        return Ok(());
    }

    // 创建 Tokio 运行时并启动异步任务
    tokio::runtime::Runtime::new()
        .expect("Failed to create Tokio runtime")
        .block_on(async move {
            // 在 runtime 内初始化统计模块
            let stats = init_statistics(&config.statistics).await?;

            // 为每个输入节点启动 HTTP 服务器
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
///
/// 根据配置创建统计存储管理器，若禁用则返回 None
async fn init_statistics(
    config: &Option<StatisticsConfig>,
) -> Result<Option<Arc<StatsStoreManager>>, Box<dyn std::error::Error>> {
    match config {
        Some(config) => {
            if !config.enabled {
                info!("Statistics disabled");
                return Ok(None);
            }
            let store = StatsStoreManager::new(config).await?;
            info!("Statistics enabled, db_path={}", config.db_path);
            Ok(Some(Arc::new(store)))
        }
        None => {
            info!("Statistics not configured, using defaults (disabled)");
            Ok(None)
        }
    }
}
