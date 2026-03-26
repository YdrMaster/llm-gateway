use serde::{Deserialize, Serialize};
use std::num::NonZeroU64;

/// 原始事件查询过滤器
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EventFilter {
    /// 起始时间（毫秒时间戳）
    pub start_time: Option<i64>,
    /// 结束时间（毫秒时间戳）
    pub end_time: Option<i64>,
    /// 模型名称
    pub model: Option<String>,
    /// 后端节点
    pub backend: Option<String>,
    /// 成功/失败
    pub success: Option<bool>,
    /// 限制返回数量
    pub limit: Option<usize>,
    /// 偏移量
    pub offset: Option<usize>,
}

/// 聚合查询参数
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggQuery {
    /// 起始时间（毫秒时间戳）
    pub start_time: i64,
    /// 结束时间（毫秒时间戳）
    pub end_time: i64,
    /// 窗口大小（秒）
    pub window_size: NonZeroU64,
    /// 模型名称
    pub model: Option<String>,
    /// 后端节点
    pub backend: Option<String>,
}

/// 聚合统计结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggStats {
    /// 窗口起始时间（毫秒时间戳）
    pub window_start: i64,
    /// 窗口大小（毫秒）
    pub window_size: i64,
    /// 模型名称
    pub model: String,
    /// 后端节点
    pub backend: String,
    /// 总请求数
    pub total_requests: i64,
    /// 成功数
    pub success_count: i64,
    /// 失败数
    pub fail_count: i64,
    /// 平均耗时（毫秒）
    pub avg_duration_ms: i64,
    /// 最小耗时（毫秒）
    pub min_duration_ms: i64,
    /// 最大耗时（毫秒）
    pub max_duration_ms: i64,
    /// P50 延迟（毫秒）
    pub p50_duration_ms: Option<i64>,
    /// P90 延迟（毫秒）
    pub p90_duration_ms: Option<i64>,
    /// P99 延迟（毫秒）
    pub p99_duration_ms: Option<i64>,
}

/// 时间粒度枚举
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum TimeGranularity {
    /// 5 分钟
    FiveMinutes,
    /// 15 分钟
    FifteenMinutes,
    /// 1 小时
    OneHour,
    /// 1 天
    OneDay,
    /// 自定义秒数
    Custom(i64),
}

impl TimeGranularity {
    /// 转换为秒数
    pub fn as_seconds(&self) -> i64 {
        match self {
            TimeGranularity::FiveMinutes => 300,
            TimeGranularity::FifteenMinutes => 900,
            TimeGranularity::OneHour => 3600,
            TimeGranularity::OneDay => 86400,
            TimeGranularity::Custom(secs) => *secs,
        }
    }

    /// 从秒数创建
    pub fn from_seconds(secs: i64) -> Self {
        match secs {
            300 => TimeGranularity::FiveMinutes,
            900 => TimeGranularity::FifteenMinutes,
            3600 => TimeGranularity::OneHour,
            86400 => TimeGranularity::OneDay,
            _ => TimeGranularity::Custom(secs),
        }
    }
}

/// 统计查询构建器
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatsQueryBuilder {
    start_time: i64,
    end_time: i64,
    granularity: TimeGranularity,
    model: Option<String>,
    backend: Option<String>,
}

impl StatsQueryBuilder {
    pub fn new(start_time: i64, end_time: i64, granularity: TimeGranularity) -> Self {
        Self {
            start_time,
            end_time,
            granularity,
            model: None,
            backend: None,
        }
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    pub fn with_backend(mut self, backend: impl Into<String>) -> Self {
        self.backend = Some(backend.into());
        self
    }

    pub fn build(self) -> AggQuery {
        let window_size = NonZeroU64::new(self.granularity.as_seconds() as u64)
            .unwrap_or_else(|| NonZeroU64::new(3600).unwrap());
        AggQuery {
            start_time: self.start_time,
            end_time: self.end_time,
            window_size,
            model: self.model,
            backend: self.backend,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_time_granularity_conversion() {
        assert_eq!(TimeGranularity::FiveMinutes.as_seconds(), 300);
        assert_eq!(TimeGranularity::FifteenMinutes.as_seconds(), 900);
        assert_eq!(TimeGranularity::OneHour.as_seconds(), 3600);
        assert_eq!(TimeGranularity::OneDay.as_seconds(), 86400);
        assert_eq!(TimeGranularity::Custom(123).as_seconds(), 123);
    }

    #[test]
    fn test_time_granularity_from_seconds() {
        assert!(matches!(
            TimeGranularity::from_seconds(300),
            TimeGranularity::FiveMinutes
        ));
        assert!(matches!(
            TimeGranularity::from_seconds(3600),
            TimeGranularity::OneHour
        ));
        assert!(matches!(
            TimeGranularity::from_seconds(999),
            TimeGranularity::Custom(999)
        ));
    }

    #[test]
    fn test_stats_query_builder() {
        let query = StatsQueryBuilder::new(1000, 2000, TimeGranularity::OneHour)
            .with_model("qwen3.5-35b")
            .with_backend("sglang")
            .build();

        assert_eq!(query.start_time, 1000);
        assert_eq!(query.end_time, 2000);
        assert_eq!(query.window_size, NonZeroU64::new(3600).unwrap());
        assert_eq!(query.model, Some("qwen3.5-35b".to_string()));
        assert_eq!(query.backend, Some("sglang".to_string()));
    }

    #[test]
    fn test_event_filter_default() {
        let filter = EventFilter::default();
        assert!(filter.start_time.is_none());
        assert!(filter.end_time.is_none());
        assert!(filter.model.is_none());
        assert!(filter.backend.is_none());
        assert!(filter.success.is_none());
        assert!(filter.limit.is_none());
        assert!(filter.offset.is_none());
    }
}
