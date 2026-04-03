//! 集成测试：Failover 机制
//!
//! 验证：
//! 1. 当第一个后端返回错误时，代理尝试下一个
//! 2. 成功的 Failover 向客户端返回 200

mod common;

use common::{http_client, start_proxy, stop_proxy};

#[tokio::test]
async fn test_failover_on_backend_failure() {
    let (child, port) = start_proxy();
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    let client = http_client();

    // 发送多个请求 — Mock A 50% 失败，
    // Failover 到 Mock B 应确保大部分请求成功
    let mut success_count = 0;
    let total_requests = 10;

    for _ in 0..total_requests {
        let response = client
            .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
            .header("Content-Type", "application/json")
            .body(r#"{"model":"test","messages":[{"role":"user","content":"hi"}]}"#)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => success_count += 1,
            _ => {
                // 请求完全失败（两个后端都失败或 Failover 未生效）
            }
        }
    }

    // A 的失败率 50%，B 始终成功，
    // 有 Failover 时应接近 100% 成功。无 Failover 则约 50%。
    // 使用 70% 阈值来考虑时序边界情况。
    let success_rate = success_count as f64 / total_requests as f64;
    assert!(
        success_rate >= 0.7,
        "Failover should achieve >= 70% success rate, got {success_rate:.1} ({success_count}/{total_requests})"
    );

    stop_proxy(child);
}
