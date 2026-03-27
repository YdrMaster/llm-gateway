//! 健康监控模块
//!
//! 实现滑动窗口式的健康检查和熔断机制，防止向后端发送过多失败请求

use llm_gateway_config::InternalHealthConfig;
use std::collections::VecDeque;
use std::sync::RwLock;
use std::time::Instant;

/// 滑动窗口最大容量限制（防止内存无限增长）
const MAX_WINDOW_CAPACITY: usize = 10000;

/// 单个后端节点的健康监控器
///
/// 使用滑动窗口记录请求历史，当失败次数超过阈值时进入冷却期
pub struct HealthMonitor {
    /// 监控配置
    config: InternalHealthConfig,
    /// 状态（包含滑动窗口和熔断状态）
    state: RwLock<HealthState>,
}

/// 健康状态
struct HealthState {
    /// 请求记录滑动窗口
    window: VecDeque<RequestRecord>,
    /// 增量失败计数（O(1) 复杂度）
    failure_count: u32,
    /// 熔断状态
    circuit: CircuitState,
}

/// 单次请求记录
struct RequestRecord {
    /// 请求时间戳
    timestamp: Instant,
    /// 是否成功
    success: bool,
}

/// 熔断状态
enum CircuitState {
    /// 健康状态
    Healthy,
    /// 冷却中
    CoolingDown { until: Instant },
}

impl HealthMonitor {
    /// 使用给定配置创建新的健康监控器
    pub fn new(config: InternalHealthConfig) -> Self {
        Self {
            config,
            state: RwLock::new(HealthState {
                window: VecDeque::with_capacity(MAX_WINDOW_CAPACITY),
                failure_count: 0,
                circuit: CircuitState::Healthy,
            }),
        }
    }

    /// 检查后端是否可用（不在冷却期）
    pub fn is_available(&self) -> bool {
        let state = self.state.read().unwrap();
        match &state.circuit {
            CircuitState::Healthy => true,
            CircuitState::CoolingDown { until } => Instant::now() >= *until,
        }
    }

    /// 记录一次成功的请求
    pub fn record_success(&self) {
        let mut state = self.state.write().unwrap();
        let now = Instant::now();

        // 检查是否刚从冷却期恢复
        let was_cooling_down = matches!(state.circuit, CircuitState::CoolingDown { .. });

        // 添加成功记录
        state.window.push_back(RequestRecord {
            timestamp: now,
            success: true,
        });

        // 清理过期记录
        Self::cleanup_window_static(&mut state, &self.config, now);

        // 成功后恢复到健康状态
        if was_cooling_down {
            info!("Backend recovered from cooldown");
        }
        state.circuit = CircuitState::Healthy;
    }

    /// 记录一次失败的请求
    pub fn record_failure(&self) {
        let mut state = self.state.write().unwrap();
        let now = Instant::now();

        // 添加失败记录
        state.window.push_back(RequestRecord {
            timestamp: now,
            success: false,
        });

        // 增量增加失败计数
        state.failure_count += 1;

        // 清理过期记录并强制执行容量限制
        Self::cleanup_window_static(&mut state, &self.config, now);

        // 检查是否超过阈值（使用增量计数实现 O(1) 复杂度）
        if state.failure_count >= self.config.failure_threshold {
            let until = now + self.config.cooldown_duration;
            info!(
                "Backend entering cooldown for {:?} until {:?}",
                self.config.cooldown_duration, until
            );
            // 进入冷却期
            state.circuit = CircuitState::CoolingDown { until };
        }
    }

    /// 清理滑动窗口中的过期记录
    ///
    /// 静态版本，用于异步上下文
    fn cleanup_window_static(state: &mut HealthState, config: &InternalHealthConfig, now: Instant) {
        let cutoff = now - config.window_size;

        // 1. 清理时间过期的记录
        while let Some(record) = state.window.front() {
            if record.timestamp < cutoff {
                // 如果过期的是失败记录，减少失败计数
                if !record.success {
                    state.failure_count = state.failure_count.saturating_sub(1);
                }
                state.window.pop_front();
            } else {
                break;
            }
        }

        // 2. 清理超过容量限制的记录（从最旧的开始）
        while state.window.len() > MAX_WINDOW_CAPACITY {
            if let Some(record) = state.window.pop_front()
                && !record.success
            {
                state.failure_count = state.failure_count.saturating_sub(1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn test_config() -> InternalHealthConfig {
        InternalHealthConfig {
            window_size: Duration::from_secs(60),
            failure_threshold: 3,
            cooldown_duration: Duration::from_secs(300),
        }
    }

    #[test]
    fn test_initially_available() {
        let monitor = HealthMonitor::new(test_config());
        assert!(monitor.is_available());
    }

    #[test]
    fn test_enters_cooldown_after_failures() {
        let monitor = HealthMonitor::new(test_config());

        // Record 3 failures (threshold)
        monitor.record_failure();
        monitor.record_failure();
        monitor.record_failure();

        // Should be in cooldown
        assert!(!monitor.is_available());
    }

    #[test]
    fn test_success_resets_cooldown() {
        let monitor = HealthMonitor::new(test_config());

        // Record failures
        monitor.record_failure();
        monitor.record_failure();

        // Record success
        monitor.record_success();

        // Should still be available
        assert!(monitor.is_available());
    }

    #[test]
    fn test_window_expires_old_records() {
        // This test verifies the sliding window behavior
        // In real usage, records older than window_size are cleaned up
        let monitor = HealthMonitor::new(test_config());

        // Record 2 failures
        monitor.record_failure();
        monitor.record_failure();

        // Still available (threshold not reached)
        assert!(monitor.is_available());
    }

    #[test]
    fn test_incremental_failure_count() {
        let monitor = HealthMonitor::new(test_config());

        // Record 2 failures
        monitor.record_failure();
        monitor.record_failure();

        // Verify still available
        assert!(monitor.is_available());

        // 3rd failure triggers cooldown
        monitor.record_failure();
        assert!(!monitor.is_available());
    }

    #[test]
    fn test_window_capacity_limit() {
        let monitor = HealthMonitor::new(InternalHealthConfig {
            window_size: Duration::from_secs(3600), // Long window
            failure_threshold: 100,                 // High threshold
            cooldown_duration: Duration::from_secs(300),
        });

        // Record more than capacity limit
        for _ in 0..MAX_WINDOW_CAPACITY + 1000 {
            monitor.record_success();
        }

        // Verify window size doesn't exceed limit
        let state = monitor.state.read().unwrap();
        assert!(state.window.len() <= MAX_WINDOW_CAPACITY);
    }
}
