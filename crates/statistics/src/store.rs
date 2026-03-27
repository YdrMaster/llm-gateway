//! 统计存储管理器模块

use crate::Result;
use crate::StatisticsError::DatabaseError;
use crate::config::StatisticsConfig;
use crate::event::RoutingEvent;
use crate::query::{AggQuery, AggStats, EventFilter};
use crate::sqlite::SqliteStore;
use log::warn;
use std::sync::Arc;
use tokio::sync::mpsc;

/// 统计存储管理器
pub struct StatsStoreManager {
    /// SQLite 存储实例
    store: Arc<SqliteStore>,
    /// 用于缓冲事件写入的通道发送端
    tx: mpsc::Sender<RoutingEvent>,
}

impl StatsStoreManager {
    /// 创建新的存储管理器
    pub async fn new(config: &StatisticsConfig) -> Result<Self> {
        config.validate()?;

        // 创建 SQLite 存储
        let store = if config.db_path == ":memory:" {
            Arc::new(SqliteStore::in_memory()?)
        } else {
            Arc::new(SqliteStore::new(&config.db_path)?)
        };

        // 创建有界通道用于缓冲事件写入
        let (tx, rx) = mpsc::channel::<RoutingEvent>(config.write_buffer_size);

        // 启动后台写入任务
        let store_clone = store.clone();
        tokio::spawn(async move {
            Self::background_writer(store_clone, rx).await;
        });

        Ok(Self { store, tx })
    }

    /// 后台写入任务 - 批量写入以减少锁竞争
    async fn background_writer(store: Arc<SqliteStore>, mut rx: mpsc::Receiver<RoutingEvent>) {
        const BATCH_SIZE: usize = 100;
        const FLUSH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

        let mut batch = Vec::with_capacity(BATCH_SIZE);
        let mut interval = tokio::time::interval(FLUSH_INTERVAL);

        loop {
            tokio::select! {
                Some(event) = rx.recv() => {
                    batch.push(event);
                    if batch.len() >= BATCH_SIZE {
                        Self::flush_batch(&store, &mut batch).await;
                    }
                }
                _ = interval.tick() => {
                    // 定时刷新
                    Self::flush_batch(&store, &mut batch).await;
                }
                else => {
                    // 通道关闭，刷新剩余事件
                    Self::flush_batch(&store, &mut batch).await;
                    break;
                }
            }
        }
    }

    /// 刷新批次中的事件到数据库
    async fn flush_batch(store: &Arc<SqliteStore>, batch: &mut Vec<RoutingEvent>) {
        if batch.is_empty() {
            return;
        }

        let store = store.clone();
        let events = std::mem::take(batch);

        // 在 spawn_blocking 中执行批量写入，避免阻塞异步运行时
        if let Err(e) = tokio::task::spawn_blocking(move || {
            for event in events {
                if let Err(e) = store.insert_event(&event) {
                    warn!("Failed to write event: {e}");
                }
            }
        })
        .await
        {
            warn!("Background writer task failed: {e}");
        }
    }

    /// 记录事件 - 通过通道异步写入
    pub async fn record_event(&self, event: RoutingEvent) -> Result<()> {
        // 使用 send().await 进行背压控制，避免数据丢失
        match self.tx.send(event).await {
            Ok(_) => Ok(()),
            Err(e) => Err(DatabaseError(format!("Channel closed: {e}"))),
        }
    }

    /// 查询原始事件
    pub async fn query_events(&self, filter: EventFilter) -> Result<Vec<RoutingEvent>> {
        // 在后台执行查询
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || store.query_events_internal(&filter))
            .await
            .map_err(|e| DatabaseError(format!("Query failed: {e}")))?
    }

    /// 获取聚合统计
    pub async fn get_aggregated_stats(&self, query: AggQuery) -> Result<Vec<AggStats>> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || -> Result<Vec<AggStats>> {
            let stats = store.query_aggregated_table(&query)?;
            if !stats.is_empty() {
                return Ok(stats);
            }
            store.compute_aggregation(&query)
        })
        .await
        .map_err(|e| DatabaseError(format!("Query failed: {e}")))?
    }

    /// 清理过期数据
    pub async fn cleanup_old(&self, before: i64) -> Result<usize> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || store.cleanup_old_internal(before))
            .await
            .map_err(|e| DatabaseError(format!("Cleanup failed: {e}")))?
    }

    /// 获取事件总数
    pub async fn count_events(&self) -> Result<i64> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || store.count_events_internal())
            .await
            .map_err(|e| DatabaseError(format!("Count failed: {e}")))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_validation() {
        let config = StatisticsConfig::in_memory();
        assert!(config.validate().is_ok());
    }
}
