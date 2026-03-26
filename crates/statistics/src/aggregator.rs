use crate::event::RoutingEvent;
use crate::query::AggStats;
use std::collections::HashMap;
use std::num::NonZeroU64;

/// 聚合计算器
pub struct Aggregator;

impl Aggregator {
    /// 按指定窗口大小聚合事件
    pub fn aggregate(events: &[RoutingEvent], window_size: NonZeroU64) -> Vec<AggStats> {
        if events.is_empty() {
            return Vec::new();
        }

        let window_size = window_size.get() as i64;

        // 按 (窗口起始，model, backend) 分组
        let mut groups: HashMap<(i64, String, String), Vec<&RoutingEvent>> = HashMap::new();

        for event in events {
            // 计算窗口起始时间（毫秒）
            let window_start = (event.timestamp / 1000 / window_size) * window_size * 1000;
            let key = (window_start, event.model.clone(), event.backend.clone());
            groups.entry(key).or_default().push(event);
        }

        // 计算各组统计值
        groups
            .into_iter()
            .map(|((window_start, model, backend), events)| {
                let total_requests = events.len() as i64;
                let success_count = events.iter().filter(|e| e.success).count() as i64;
                let fail_count = events.iter().filter(|e| !e.success).count() as i64;

                // 计算延迟统计
                let mut durations: Vec<i64> = events.iter().map(|e| e.duration_ms).collect();
                durations.sort();

                let avg_duration_ms = if total_requests > 0 {
                    durations.iter().sum::<i64>() / total_requests
                } else {
                    0
                };

                let min_duration_ms = durations.first().copied().unwrap_or(0);
                let max_duration_ms = durations.last().copied().unwrap_or(0);

                AggStats {
                    window_start,
                    window_size: window_size * 1000, // 转换为毫秒
                    model,
                    backend,
                    total_requests,
                    success_count,
                    fail_count,
                    avg_duration_ms,
                    min_duration_ms,
                    max_duration_ms,
                    p50_duration_ms: Some(calculate_percentile(&durations, 0.50)),
                    p90_duration_ms: Some(calculate_percentile(&durations, 0.90)),
                    p99_duration_ms: Some(calculate_percentile(&durations, 0.99)),
                }
            })
            .collect()
    }
}

/// 计算百分位数
fn calculate_percentile(sorted_durations: &[i64], percentile: f64) -> i64 {
    if sorted_durations.is_empty() {
        return 0;
    }

    let index = ((sorted_durations.len() as f64 - 1.0) * percentile).round() as usize;
    sorted_durations.get(index).copied().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_event(
        timestamp: i64,
        model: &str,
        backend: &str,
        success: bool,
        duration: i64,
    ) -> RoutingEvent {
        RoutingEvent::builder(timestamp, 9000)
            .remote_addr_raw(0xC0A80101, 12345)
            .method("POST")
            .path("/v1/chat/completions")
            .model(model)
            .routing_path("input->node")
            .backend(backend)
            .success(success)
            .duration_ms(duration)
            .build()
    }

    #[test]
    fn test_aggregate_empty_events() {
        let events: Vec<RoutingEvent> = Vec::new();
        let result = Aggregator::aggregate(&events, NonZeroU64::new(3600).unwrap());
        assert!(result.is_empty());
    }

    #[test]
    fn test_aggregate_single_event() {
        let timestamp = 1234567890000;
        let events = vec![create_event(timestamp, "model-a", "backend-1", true, 100)];

        let result = Aggregator::aggregate(&events, NonZeroU64::new(3600).unwrap());

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].total_requests, 1);
        assert_eq!(result[0].success_count, 1);
        assert_eq!(result[0].fail_count, 0);
        assert_eq!(result[0].avg_duration_ms, 100);
        assert_eq!(result[0].min_duration_ms, 100);
        assert_eq!(result[0].max_duration_ms, 100);
    }

    #[test]
    fn test_aggregate_multiple_events_same_window() {
        // 同一窗口内的多个事件
        let base_timestamp = 1234567200000; // 某个整点
        let events = vec![
            create_event(base_timestamp, "model-a", "backend-1", true, 100),
            create_event(base_timestamp + 1000, "model-a", "backend-1", true, 200),
            create_event(base_timestamp + 2000, "model-a", "backend-1", false, 300),
        ];

        let result = Aggregator::aggregate(&events, NonZeroU64::new(3600).unwrap());

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].total_requests, 3);
        assert_eq!(result[0].success_count, 2);
        assert_eq!(result[0].fail_count, 1);
        assert_eq!(result[0].avg_duration_ms, 200); // (100+200+300)/3
        assert_eq!(result[0].min_duration_ms, 100);
        assert_eq!(result[0].max_duration_ms, 300);
    }

    #[test]
    fn test_aggregate_different_windows() {
        // 不同窗口的事件
        let events = vec![
            create_event(3600000, "model-a", "backend-1", true, 100), // 窗口 1
            create_event(3600001, "model-a", "backend-1", true, 150), // 窗口 1
            create_event(7200000, "model-a", "backend-1", true, 200), // 窗口 2
        ];

        let result = Aggregator::aggregate(&events, NonZeroU64::new(3600).unwrap());

        assert_eq!(result.len(), 2);

        let window1: Vec<_> = result
            .iter()
            .filter(|s| s.window_start == 3600000)
            .collect();
        let window2: Vec<_> = result
            .iter()
            .filter(|s| s.window_start == 7200000)
            .collect();

        assert_eq!(window1.len(), 1);
        assert_eq!(window1[0].total_requests, 2);
        assert_eq!(window1[0].avg_duration_ms, 125);

        assert_eq!(window2.len(), 1);
        assert_eq!(window2[0].total_requests, 1);
        assert_eq!(window2[0].avg_duration_ms, 200);
    }

    #[test]
    fn test_aggregate_different_models() {
        let base_timestamp = 3600000;
        let events = vec![
            create_event(base_timestamp, "model-a", "backend-1", true, 100),
            create_event(base_timestamp + 1000, "model-b", "backend-1", true, 200),
        ];

        let result = Aggregator::aggregate(&events, NonZeroU64::new(3600).unwrap());

        assert_eq!(result.len(), 2);

        let model_a: Vec<_> = result.iter().filter(|s| s.model == "model-a").collect();
        let model_b: Vec<_> = result.iter().filter(|s| s.model == "model-b").collect();

        assert_eq!(model_a.len(), 1);
        assert_eq!(model_a[0].avg_duration_ms, 100);

        assert_eq!(model_b.len(), 1);
        assert_eq!(model_b[0].avg_duration_ms, 200);
    }

    #[test]
    fn test_percentile_calculation() {
        let durations = vec![100, 200, 300, 400, 500, 600, 700, 800, 900, 1000];

        let p50 = calculate_percentile(&durations, 0.50);
        let p90 = calculate_percentile(&durations, 0.90);
        let p99 = calculate_percentile(&durations, 0.99);

        // 百分位数计算使用近似值
        assert_eq!(p50, 600); // 中位数附近
        assert_eq!(p90, 900);
        assert_eq!(p99, 1000);
    }

    #[test]
    fn test_aggregate_with_different_window_sizes() {
        let events = vec![
            create_event(0, "model-a", "backend-1", true, 100),
            create_event(300000, "model-a", "backend-1", true, 200), // 5 分钟后
            create_event(600000, "model-a", "backend-1", true, 300), // 10 分钟后
        ];

        // 使用 5 分钟窗口
        let result_5min = Aggregator::aggregate(&events, NonZeroU64::new(300).unwrap());
        assert_eq!(result_5min.len(), 3); // 3 个不同窗口

        // 使用 15 分钟窗口
        let result_15min = Aggregator::aggregate(&events, NonZeroU64::new(900).unwrap());
        assert_eq!(result_15min.len(), 1); // 同一窗口
        assert_eq!(result_15min[0].total_requests, 3);
    }
}
