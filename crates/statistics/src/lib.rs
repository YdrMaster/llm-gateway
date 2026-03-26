//! LLM Gateway 事件统计模块
//!
//! 提供请求路由事件的记录、查询和聚合统计功能。
//!
//! # 示例
//!
//! ```no_run
//! use llm_gateway_statistics::{StatisticsConfig, StatsStoreManager, RoutingEvent, EventFilter};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // 创建配置
//! let config = StatisticsConfig::in_memory();
//!
//! // 创建存储管理器
//! let store = StatsStoreManager::new(&config).await?;
//!
//! // 记录事件（使用 Builder）
//! let event = RoutingEvent::builder(1234567890000, 9000)
//!     .remote_addr("192.168.1.1:12345".parse().unwrap())
//!     .method("POST")
//!     .path("/v1/chat/completions")
//!     .model("qwen3.5-35b")
//!     .routing_path("input->qwen3.5-35b->sglang")
//!     .backend("sglang")
//!     .success(true)
//!     .duration_ms(150)
//!     .build();
//! store.record_event(event).await?;
//!
//! // 查询事件
//! let events = store.query_events(EventFilter {
//!     start_time: Some(1234567880000),
//!     end_time: Some(1234567900000),
//!     model: Some("qwen3.5-35b".to_string()),
//!     ..Default::default()
//! }).await?;
//!
//! # Ok(())
//! # }
//! ```

pub mod aggregator;
pub mod config;
pub mod event;
pub mod query;
pub mod sqlite;
pub mod store;

// 重新导出常用类型
pub use aggregator::Aggregator;
pub use config::{AggregationConfig, StatisticsConfig};
pub use event::{RoutingEvent, RoutingEventBuilder};
pub use query::{AggQuery, AggStats, EventFilter, StatsQueryBuilder, TimeGranularity};
pub use sqlite::SqliteStore;
pub use store::StatsStoreManager;

/// 错误类型
#[derive(Debug, thiserror::Error)]
pub enum StatisticsError {
    #[error("Database error: {0}")]
    DatabaseError(String),

    #[error("Configuration error: {0}")]
    ConfigurationError(String),

    #[error("Query error: {0}")]
    QueryError(String),
}

impl From<String> for StatisticsError {
    fn from(s: String) -> Self {
        StatisticsError::ConfigurationError(s)
    }
}

/// 结果类型别名
pub type Result<T> = std::result::Result<T, StatisticsError>;
