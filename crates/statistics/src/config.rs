use serde::{Deserialize, Serialize};
use std::num::NonZeroU64;

/// 统计配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatisticsConfig {
    /// 是否启用统计
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// 数据库文件路径
    #[serde(default = "default_db_path")]
    pub db_path: String,
    /// 数据保留天数（自动清理）
    #[serde(default = "default_retention_days")]
    pub retention_days: u64,
    /// 写入缓冲区大小（事件数量）
    #[serde(default = "default_write_buffer_size")]
    pub write_buffer_size: usize,
    /// 聚合配置
    #[serde(default)]
    pub aggregation: AggregationConfig,
}

impl Default for StatisticsConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            db_path: default_db_path(),
            retention_days: default_retention_days(),
            write_buffer_size: default_write_buffer_size(),
            aggregation: AggregationConfig::default(),
        }
    }
}

fn default_enabled() -> bool {
    true
}

fn default_db_path() -> String {
    "stats.db".to_string()
}

fn default_retention_days() -> u64 {
    30
}

fn default_write_buffer_size() -> usize {
    1000
}

/// 聚合配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregationConfig {
    /// 窗口大小列表（秒）
    #[serde(default = "default_window_sizes")]
    pub window_sizes: Vec<NonZeroU64>,
    /// 默认聚合粒度（秒）
    #[serde(default = "default_window")]
    pub default_window: NonZeroU64,
    /// 是否自动预计算
    #[serde(default = "default_auto_aggregate")]
    pub auto_aggregate: bool,
}

impl Default for AggregationConfig {
    fn default() -> Self {
        Self {
            window_sizes: default_window_sizes(),
            default_window: default_window(),
            auto_aggregate: default_auto_aggregate(),
        }
    }
}

fn default_window_sizes() -> Vec<NonZeroU64> {
    vec![
        NonZeroU64::new(300).unwrap(),   // 5min
        NonZeroU64::new(900).unwrap(),   // 15min
        NonZeroU64::new(3600).unwrap(),  // 1hour
        NonZeroU64::new(86400).unwrap(), // 1day
    ]
}

fn default_window() -> NonZeroU64 {
    NonZeroU64::new(3600).unwrap() // 1hour
}

fn default_auto_aggregate() -> bool {
    true
}

impl StatisticsConfig {
    /// 创建内存配置（用于测试）
    pub fn in_memory() -> Self {
        Self {
            enabled: true,
            db_path: ":memory:".to_string(),
            retention_days: 7,
            write_buffer_size: 100,
            aggregation: AggregationConfig::default(),
        }
    }

    /// 验证配置
    pub fn validate(&self) -> Result<(), String> {
        if self.retention_days == 0 {
            return Err("retention_days must be greater than 0".to_string());
        }

        if self.aggregation.window_sizes.is_empty() {
            return Err("window_sizes cannot be empty".to_string());
        }

        if !self
            .aggregation
            .window_sizes
            .contains(&self.aggregation.default_window)
        {
            return Err("default_window must be in window_sizes".to_string());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = StatisticsConfig::default();
        assert!(config.enabled);
        assert_eq!(config.db_path, "stats.db");
        assert_eq!(config.retention_days, 30);

        // 验证 window_sizes
        let expected_sizes: Vec<NonZeroU64> = vec![
            NonZeroU64::new(300).unwrap(),
            NonZeroU64::new(900).unwrap(),
            NonZeroU64::new(3600).unwrap(),
            NonZeroU64::new(86400).unwrap(),
        ];
        assert_eq!(config.aggregation.window_sizes, expected_sizes);
        assert_eq!(
            config.aggregation.default_window,
            NonZeroU64::new(3600).unwrap()
        );
        assert!(config.aggregation.auto_aggregate);
    }

    #[test]
    fn test_in_memory_config() {
        let config = StatisticsConfig::in_memory();
        assert!(config.enabled);
        assert_eq!(config.db_path, ":memory:");
        assert_eq!(config.retention_days, 7);
    }

    #[test]
    fn test_validate_valid_config() {
        let config = StatisticsConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_zero_retention() {
        let mut config = StatisticsConfig::default();
        config.retention_days = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_empty_window_sizes() {
        let mut config = StatisticsConfig::default();
        config.aggregation.window_sizes = vec![];
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_invalid_default_window() {
        let mut config = StatisticsConfig::default();
        config.aggregation.default_window = NonZeroU64::new(600).unwrap(); // 不在 window_sizes 中
        assert!(config.validate().is_err());
    }
}
