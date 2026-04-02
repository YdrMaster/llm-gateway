//! 策略链执行器
//!
//! 按顺序执行多个策略插件

use super::failover::{BackendSelectionResult, RoutingError};
use log::debug;
use std::sync::Arc;

/// 策略插件 trait
#[async_trait::async_trait]
pub trait RoutingPlugin: Send + Sync {
    /// 选择后端
    async fn select_backend(&self, context: &RoutingContext) -> BackendSelectionResult;
}

/// 路由上下文
#[derive(Clone, Debug)]
pub struct RoutingContext {
    /// 模型名称
    pub model: String,
    /// 请求路径
    pub path: String,
    /// 目标协议
    pub target_protocol: String,
}

impl RoutingContext {
    pub fn new(model: String, path: String, target_protocol: String) -> Self {
        Self {
            model,
            path,
            target_protocol,
        }
    }
}

/// 策略链执行器
pub struct ChainExecutor {
    /// 策略列表
    plugins: Vec<Arc<dyn RoutingPlugin>>,
}

impl ChainExecutor {
    /// 创建新的策略链执行器
    pub fn new(plugins: Vec<Arc<dyn RoutingPlugin>>) -> Self {
        Self { plugins }
    }

    /// 执行策略链
    pub async fn execute(&self, context: &RoutingContext) -> BackendSelectionResult {
        debug!(
            "Executing strategy chain with {} plugins",
            self.plugins.len()
        );

        for (idx, plugin) in self.plugins.iter().enumerate() {
            debug!("Executing plugin {idx}");
            match plugin.select_backend(context).await {
                Ok(backend) => {
                    debug!("Plugin {idx} selected backend: {}", backend.addr);
                    return Ok(backend);
                }
                Err(e) => {
                    debug!("Plugin {idx} failed: {e:?}");
                    // 继续尝试下一个插件
                    continue;
                }
            }
        }

        Err(RoutingError::NoAvailableBackend)
    }

    /// 添加策略插件
    pub fn add_plugin(&mut self, plugin: Arc<dyn RoutingPlugin>) {
        self.plugins.push(plugin);
    }
}
