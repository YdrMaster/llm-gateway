//! Failover 策略
//!
//! 后端失败时自动切换到备选后端

use super::chain::{RoutingContext, RoutingPlugin};
use crate::backend::Backend;
use log::{debug, info, warn};
use std::sync::Arc;
use tokio::sync::RwLock;

/// 后端选择结果
pub type BackendSelectionResult = Result<Backend, RoutingError>;

/// 路由错误
#[derive(Debug, Clone)]
pub enum RoutingError {
    /// 没有可用后端
    NoAvailableBackend,
    /// 后端连接失败
    BackendConnectionFailed(String),
    /// 健康检查失败
    HealthCheckFailed(String),
}

impl std::fmt::Display for RoutingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoAvailableBackend => write!(f, "No available backend"),
            Self::BackendConnectionFailed(addr) => {
                write!(f, "Backend connection failed: {addr}")
            }
            Self::HealthCheckFailed(addr) => write!(f, "Backend health check failed: {addr}"),
        }
    }
}

impl std::error::Error for RoutingError {}

/// 健康检查器
pub struct HealthChecker {
    /// 后端健康状态
    health_status: Arc<RwLock<std::collections::HashMap<String, bool>>>,
}

impl HealthChecker {
    pub fn new() -> Self {
        Self {
            health_status: Arc::new(RwLock::new(std::collections::HashMap::new())),
        }
    }

    /// 检查后端是否健康
    pub async fn is_healthy(&self, addr: &str) -> bool {
        let status = self.health_status.read().await;
        status.get(addr).copied().unwrap_or(true)
    }

    /// 记录后端失败
    pub async fn record_failure(&self, addr: &str) {
        let mut status = self.health_status.write().await;
        status.insert(addr.to_string(), false);
        warn!("Recorded failure for backend: {addr}");
    }

    /// 记录后端成功
    pub async fn record_success(&self, addr: &str) {
        let mut status = self.health_status.write().await;
        status.insert(addr.to_string(), true);
        debug!("Recorded success for backend: {addr}");
    }
}

impl Default for HealthChecker {
    fn default() -> Self {
        Self::new()
    }
}

/// Failover 策略
pub struct FailoverStrategy {
    /// 后端地址列表
    backends: Vec<String>,
    /// 健康检查器
    health_checker: HealthChecker,
}

impl FailoverStrategy {
    /// 创建新的 Failover 策略
    pub fn new(backends: Vec<String>) -> Self {
        Self {
            backends,
            health_checker: HealthChecker::new(),
        }
    }

    /// 尝试连接后端
    async fn try_connect(&self, addr: &str) -> Result<Backend, RoutingError> {
        // 简单的 TCP 连接检查
        match tokio::time::timeout(
            std::time::Duration::from_secs(2),
            tokio::net::TcpStream::connect(addr),
        )
        .await
        {
            Ok(Ok(_stream)) => {
                debug!("Successfully connected to backend: {addr}");
                Ok(Backend {
                    addr: addr.to_string(),
                    healthy: true,
                    failure_count: 0,
                })
            }
            Ok(Err(e)) => {
                debug!("Failed to connect to backend {addr}: {e}");
                Err(RoutingError::BackendConnectionFailed(addr.to_string()))
            }
            Err(_) => {
                debug!("Connection timeout to backend: {addr}");
                Err(RoutingError::BackendConnectionFailed(addr.to_string()))
            }
        }
    }
}

#[async_trait::async_trait]
impl RoutingPlugin for FailoverStrategy {
    async fn select_backend(&self, _context: &RoutingContext) -> BackendSelectionResult {
        debug!(
            "FailoverStrategy: Selecting backend from {} options",
            self.backends.len()
        );

        for backend_addr in &self.backends {
            // 检查健康状态
            if !self.health_checker.is_healthy(backend_addr).await {
                debug!("Skipping unhealthy backend: {backend_addr}");
                continue;
            }

            // 尝试连接
            match self.try_connect(backend_addr).await {
                Ok(backend) => {
                    self.health_checker.record_success(backend_addr).await;
                    info!("Selected backend: {backend_addr}");
                    return Ok(backend);
                }
                Err(e) => {
                    self.health_checker.record_failure(backend_addr).await;
                    warn!("Backend {backend_addr} failed: {e}");
                    // 继续尝试下一个
                    continue;
                }
            }
        }

        Err(RoutingError::NoAvailableBackend)
    }
}
