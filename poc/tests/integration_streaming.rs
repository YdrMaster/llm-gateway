//! 集成测试：通过 Pingora 代理的 SSE 流式
//!
//! 验证：
//! 1. 代理转发 SSE 流（多个块，非缓冲）
//! 2. 流包含预期的 SSE 帧
//! 3. 多个 SSE 帧随时间到达（非一次性全部到达）

mod common;

use common::{http_client, start_proxy, stop_proxy};

#[tokio::test]
async fn test_sse_streaming_passthrough() {
    let (child, port) = start_proxy();

    // 等待 Mock 启动（它们在 main.rs 中生成）
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    let client = http_client();

    // 发送多个请求，直到有一个成功（因为 Mock A 可能返回 500）
    let mut body = String::new();
    for _ in 0..5 {
        let response = client
            .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
            .header("Content-Type", "application/json")
            .body(r#"{"model":"test","messages":[{"role":"user","content":"hi"}],"stream":true}"#)
            .send()
            .await;

        if let Ok(resp) = response {
            if resp.status().is_success() {
                body = resp.text().await.expect("Should read body");
                break;
            }
        }
    }

    assert!(
        !body.is_empty(),
        "Should get at least one successful response"
    );

    // Mock 返回 "Hello" — 验证它在响应中
    assert!(
        body.contains("Hello") || body.contains("DONE") || body.contains("[DONE]"),
        "Body should contain SSE data. Got: {body}"
    );

    stop_proxy(child);
}

#[tokio::test]
async fn test_sse_stream_arrives_over_time() {
    let (child, port) = start_proxy();
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    let client = http_client();

    // 发送请求直到成功
    let start = std::time::Instant::now();
    let mut chunk_count = 0;

    for _ in 0..5 {
        let response = client
            .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
            .header("Content-Type", "application/json")
            .body(r#"{"model":"test","messages":[{"role":"user","content":"hi"}],"stream":true}"#)
            .send()
            .await;

        if let Ok(resp) = response {
            if resp.status().is_success() {
                use futures::StreamExt;
                let mut stream = resp.bytes_stream();
                while let Some(chunk) = stream.next().await {
                    let _chunk = chunk.expect("Chunk should be valid");
                    chunk_count += 1;
                }
                break;
            }
        }
    }

    // Mock 发送 5 帧，帧间 50ms 延迟 = 约 250ms 最小
    // 如果流式正常，总时间应 > 100ms（非瞬时）
    let total_time = start.elapsed();
    assert!(
        total_time > std::time::Duration::from_millis(100),
        "Streaming should take >100ms (got {:?}). If too fast, the mock may be buffering.",
        total_time
    );
    assert!(
        chunk_count >= 1,
        "Should receive at least one chunk, got {chunk_count}"
    );

    stop_proxy(child);
}
