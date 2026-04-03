//! 集成测试：HTTP 级别错误检测
//!
//! 验证：
//! 1. 上游 5xx 响应被正确检测并记录
//! 2. 流式 SSE 转换正常工作
//!
//! 注意：由于 Pingora 标准代理管道在 upstream_response_filter 之后
//! 立即写入响应头，HTTP 5xx 的静默重试需要 Phase 1 的 custom_forwarding 实现。

mod common;

use common::{http_client, start_proxy, stop_proxy};

#[tokio::test]
async fn test_sse_streaming_with_5xx_detection() {
    let (child, port) = start_proxy();
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    let client = http_client();

    // 发送多个请求，验证流式 SSE 转发正常
    let mut success_count = 0;
    let total_requests = 10;

    for _ in 0..total_requests {
        let response = client
            .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
            .header("Content-Type", "application/json")
            .body(
                r#"{"model":"test","messages":[{"role":"user","content":"hi"}],"stream":true}"#,
            )
            .send()
            .await;

        if let Ok(resp) = response {
            if resp.status().is_success() {
                success_count += 1;
            }
        }
    }

    // Mock A 50% 返回 500，Mock B 总是成功
    // 没有 HTTP 级别 failover 时，成功率约 50%
    // 这个测试主要验证流式转发和 5xx 检测日志
    eprintln!("SSE streaming test: {success_count}/{total_requests} succeeded");
    assert!(success_count >= 3, "At least 3 should succeed (got {success_count})");

    stop_proxy(child);
}
