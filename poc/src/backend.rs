//! 后端管理模块
//!
//! 提供后端服务器的管理和健康检查功能

use log::{debug, info, warn};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

/// 后端服务器信息
#[derive(Clone, Debug)]
pub struct Backend {
    /// 后端地址
    pub addr: String,
    /// 是否健康
    pub healthy: bool,
    /// 失败次数
    pub failure_count: u32,
}

/// 后端管理器
pub struct BackendManager {
    /// 后端列表
    backends: Arc<RwLock<Vec<Backend>>>,
    /// 健康检查配置
    health_check_config: HealthCheckConfig,
}

/// 健康检查配置
#[derive(Clone, Debug)]
pub struct HealthCheckConfig {
    /// 失败阈值，超过此值标记为不健康
    pub failure_threshold: u32,
    /// 恢复阈值，连续成功次数后恢复健康
    pub recovery_threshold: u32,
    /// 健康检查超时
    pub check_timeout: Duration,
}

impl Default for HealthCheckConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 3,
            recovery_threshold: 2,
            check_timeout: Duration::from_secs(5),
        }
    }
}

impl BackendManager {
    /// 创建新的后端管理器
    pub fn new(addrs: Vec<String>) -> Self {
        let backends: Vec<Backend> = addrs
            .into_iter()
            .map(|addr| Backend {
                addr,
                healthy: true,
                failure_count: 0,
            })
            .collect();

        Self {
            backends: Arc::new(RwLock::new(backends)),
            health_check_config: HealthCheckConfig::default(),
        }
    }

    /// 获取所有后端
    pub async fn get_all_backends(&self) -> Vec<Backend> {
        self.backends.read().await.clone()
    }

    /// 获取健康的后端列表
    pub async fn get_healthy_backends(&self) -> Vec<Backend> {
        self.backends
            .read()
            .await
            .iter()
            .filter(|b| b.healthy)
            .cloned()
            .collect()
    }

    /// 记录后端失败
    pub async fn record_failure(&self, addr: &str) {
        let mut backends = self.backends.write().await;
        if let Some(backend) = backends.iter_mut().find(|b| b.addr == addr) {
            backend.failure_count += 1;
            if backend.failure_count >= self.health_check_config.failure_threshold {
                if backend.healthy {
                    backend.healthy = false;
                    warn!(
                        "Backend {addr} marked as unhealthy after {} failures",
                        backend.failure_count
                    );
                }
            }
            debug!("Backend {addr} failure count: {}", backend.failure_count);
        }
    }

    /// 记录后端成功
    pub async fn record_success(&self, addr: &str) {
        let mut backends = self.backends.write().await;
        if let Some(backend) = backends.iter_mut().find(|b| b.addr == addr) {
            if backend.failure_count > 0 {
                backend.failure_count = backend.failure_count.saturating_sub(1);
            }
            if !backend.healthy && backend.failure_count == 0 {
                backend.healthy = true;
                info!("Backend {addr} marked as healthy");
            }
            debug!(
                "Backend {addr} success, failure count: {}",
                backend.failure_count
            );
        }
    }

    /// 检查后端是否健康
    pub async fn is_healthy(&self, addr: &str) -> bool {
        self.backends
            .read()
            .await
            .iter()
            .find(|b| b.addr == addr)
            .map(|b| b.healthy)
            .unwrap_or(false)
    }

    /// 获取后端失败次数
    pub async fn get_failure_count(&self, addr: &str) -> u32 {
        self.backends
            .read()
            .await
            .iter()
            .find(|b| b.addr == addr)
            .map(|b| b.failure_count)
            .unwrap_or(0)
    }
}
