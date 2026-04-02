//! 并发限制中间件
//!
//! 使用 Semaphore 控制并发请求数

use dashmap::DashMap;
use log::{debug, warn};
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// 并发限制层
pub struct ConcurrencyLimitLayer {
    /// 每个模型的信号量
    semaphores: DashMap<String, Arc<Semaphore>>,
    /// 默认并发限制
    default_limit: usize,
}

/// 并发上下文，持有许可
pub struct ConcurrencyContext {
    /// 持有的许可
    pub permit: Option<OwnedSemaphorePermit>,
}

impl ConcurrencyLimitLayer {
    /// 创建新的并发限制层
    pub fn new(default_limit: usize) -> Self {
        Self {
            semaphores: DashMap::new(),
            default_limit,
        }
    }

    /// 获取或创建信号量
    fn get_semaphore(&self, model: &str) -> Arc<Semaphore> {
        self.semaphores
            .entry(model.to_string())
            .or_insert_with(|| Arc::new(Semaphore::new(self.default_limit)))
            .clone()
    }

    /// 尝试获取并发许可
    pub async fn acquire(&self, model: &str) -> Result<OwnedSemaphorePermit, &'static str> {
        let sem = self.get_semaphore(model);
        let permits_before = sem.available_permits();

        match sem.try_acquire_owned() {
            Ok(permit) => {
                debug!(
                    "Acquired concurrency permit for model '{model}', available: {permits_before}"
                );
                Ok(permit)
            }
            Err(_) => {
                warn!("Concurrency limit reached for model '{model}'");
                Err("Concurrency limit exceeded")
            }
        }
    }

    /// 获取当前可用许可数
    pub fn available_permits(&self, model: &str) -> usize {
        self.get_semaphore(model).available_permits()
    }
}

impl Default for ConcurrencyLimitLayer {
    fn default() -> Self {
        Self::new(10)
    }
}
