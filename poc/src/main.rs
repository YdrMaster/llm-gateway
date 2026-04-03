//! LLM Gateway POC - 基于 Pingora 的代理验证
//!
//! 验证目标：
//! 1. SSE 流式协议转换
//! 2. 并发限制保持
//! 3. Failover 机制

mod backend;
mod middleware;
mod proxy;
mod strategy;

use backend::BackendManager;
use middleware::concurrency::ConcurrencyLimitLayer;
use middleware::protocol::ProtocolConversionMiddleware;
use proxy::LlmGatewayProxy;
use strategy::chain::ChainExecutor;
use strategy::failover::FailoverStrategy;

use log::{error, info, LevelFilter};
use pingora_core::server::Server;
use pingora_core::server::configuration::ServerConf;
use pingora_proxy::http_proxy_service;
use std::sync::Arc;

/// 端口分配
const DEFAULT_PROXY_PORT: u16 = 18080;
const MOCK_OPENAI_A_PORT: u16 = 18001;
const MOCK_OPENAI_B_PORT: u16 = 18002;
const MOCK_ANTHROPIC_PORT: u16 = 18003;

fn get_proxy_port() -> u16 {
    std::env::var("POC_PROXY_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_PROXY_PORT)
}

fn main() {
    // 初始化日志
    env_logger::Builder::new()
        .filter_level(LevelFilter::Info)
        .format_timestamp(Some(env_logger::TimestampPrecision::Millis))
        .init();

    info!("Starting LLM Gateway POC");

    // 创建运行时
    let rt = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");

    rt.block_on(async {
        // 启动 Mock 后端
        info!("Starting mock backends...");
        tokio::spawn(mock_openai_backend_a(MOCK_OPENAI_A_PORT));
        tokio::spawn(mock_openai_backend_b(MOCK_OPENAI_B_PORT));
        tokio::spawn(mock_anthropic_backend(MOCK_ANTHROPIC_PORT));

        // 等待 Mock 后端启动
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        info!(
            "Mock backends started: OpenAI-A:{}, OpenAI-B:{}, Anthropic:{}",
            MOCK_OPENAI_A_PORT, MOCK_OPENAI_B_PORT, MOCK_ANTHROPIC_PORT
        );
    });

    // 创建后端管理器
    let backend_manager = BackendManager::new(vec![
        format!("127.0.0.1:{MOCK_OPENAI_A_PORT}"),
        format!("127.0.0.1:{MOCK_OPENAI_B_PORT}"),
    ]);

    // 创建并发限制层
    let concurrency_layer = ConcurrencyLimitLayer::new(5);

    // 创建 Failover 策略
    let failover_strategy = FailoverStrategy::new(vec![
        format!("127.0.0.1:{MOCK_OPENAI_A_PORT}"),
        format!("127.0.0.1:{MOCK_OPENAI_B_PORT}"),
    ]);

    // 创建策略链执行器
    let strategy_chain = ChainExecutor::new(vec![Arc::new(failover_strategy)]);

    // 创建协议转换中间件
    let protocol_middleware = ProtocolConversionMiddleware::new();

    // 创建代理
    let proxy = LlmGatewayProxy::new(
        backend_manager,
        concurrency_layer,
        strategy_chain,
        protocol_middleware,
    );

    let proxy_port = get_proxy_port();

    // 启动 Pingora 服务器
    info!("Starting Pingora proxy server on port {proxy_port}...");

    // 创建服务器配置
    let mut server_conf = ServerConf::default();
    server_conf.daemon = false;

    let server_conf_arc = Arc::new(server_conf);

    // 创建 HTTP 代理服务并添加监听器
    let mut proxy_service = http_proxy_service(&server_conf_arc, proxy);
    proxy_service.add_tcp(&format!("0.0.0.0:{proxy_port}"));

    // 创建服务器
    let mut server = Server::new(None).expect("Failed to create server");
    server.bootstrap();

    // 添加服务
    server.add_service(proxy_service);

    info!("Server running. Press Ctrl+C to stop.");
    info!("Test endpoint: http://127.0.0.1:{proxy_port}/v1/chat/completions");

    // 运行服务器（阻塞）
    server.run_forever();
}

/// 模拟 OpenAI 后端 A - 会随机失败
async fn mock_openai_backend_a(port: u16) {
    use tokio::net::TcpListener;

    let listener = TcpListener::bind(format!("127.0.0.1:{port}"))
        .await
        .expect("Failed to bind mock OpenAI backend A");

    info!("Mock OpenAI backend A listening on {port}");

    loop {
        match listener.accept().await {
            Ok((socket, addr)) => {
                info!("Mock OpenAI-A: Accepted connection from {addr}");
                tokio::spawn(async move {
                    let mut buf = [0u8; 1024];
                    let _ = socket.readable().await;
                    let _ = socket.try_read(&mut buf).ok();

                    if rand::random::<bool>() {
                        let response = "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n";
                        let _ = socket.writable().await;
                        let _ = socket.try_write(response.as_bytes()).ok();
                        info!("Mock OpenAI-A: Returning 500 error");
                    } else {
                        let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
                        let response = format!(
                            "HTTP/1.1 200 OK\r\n\
                             Content-Type: text/event-stream\r\n\
                             \r\n\
                             data: {{\"id\":\"test-a\",\"object\":\"chat.completion.chunk\",\"created\":{ts},\"model\":\"gpt-4\",\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"Hello from A\"}},\"finish_reason\":\"stop\"}}]}}\n\n\
                             data: [DONE]\n\n"
                        );
                        let _ = socket.writable().await;
                        let _ = socket.try_write(response.as_bytes()).ok();
                    }
                });
            }
            Err(e) => error!("Mock OpenAI-A: Accept error: {e}"),
        }
    }
}

/// 模拟 OpenAI 后端 B — 以 OpenAI SSE 帧流式返回，带延迟
async fn mock_openai_backend_b(port: u16) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind(format!("127.0.0.1:{port}"))
        .await
        .expect("Failed to bind mock OpenAI backend B");

    info!("Mock OpenAI backend B listening on {port}");

    loop {
        match listener.accept().await {
            Ok((mut socket, addr)) => {
                info!("Mock OpenAI-B: Accepted connection from {addr}");
                tokio::spawn(async move {
                    // 读取请求
                    let mut buf = [0u8; 4096];
                    let _ = socket.read(&mut buf).await;

                    // 写入 HTTP 头
                    let headers = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n";
                    if socket.write_all(headers.as_bytes()).await.is_err() {
                        return;
                    }

                    // 逐帧流式发送 SSE，带延迟
                    let frames = vec![
                        format!("data: {{\"id\":\"test-b\",\"object\":\"chat.completion.chunk\",\"created\":{},\"model\":\"gpt-4\",\"choices\":[{{\"index\":0,\"delta\":{{\"role\":\"assistant\"}},\"finish_reason\":null}}]}}\n\n",
                            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()),
                        format!("data: {{\"id\":\"test-b\",\"object\":\"chat.completion.chunk\",\"created\":{},\"model\":\"gpt-4\",\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"Hello\"}},\"finish_reason\":null}}]}}\n\n",
                            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()),
                        format!("data: {{\"id\":\"test-b\",\"object\":\"chat.completion.chunk\",\"created\":{},\"model\":\"gpt-4\",\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\" from B\"}},\"finish_reason\":null}}]}}\n\n",
                            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()),
                        "data: {\"id\":\"test-b\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n".to_string(),
                        "data: [DONE]\n\n".to_string(),
                    ];

                    for frame in &frames {
                        if socket.write_all(frame.as_bytes()).await.is_err() {
                            break;
                        }
                        if socket.flush().await.is_err() {
                            break;
                        }
                        // 帧间 50ms 延迟（模拟真实 LLM 延迟）
                        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
                    }
                });
            }
            Err(e) => error!("Mock OpenAI-B: Accept error: {e}"),
        }
    }
}

/// 模拟 Anthropic 后端 — 以 Anthropic SSE 帧流式返回，带延迟
async fn mock_anthropic_backend(port: u16) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind(format!("127.0.0.1:{port}"))
        .await
        .expect("Failed to bind mock Anthropic backend");

    info!("Mock Anthropic backend listening on {port}");

    loop {
        match listener.accept().await {
            Ok((mut socket, addr)) => {
                info!("Mock Anthropic: Accepted connection from {addr}");
                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    let _ = socket.read(&mut buf).await;

                    let headers = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n";
                    if socket.write_all(headers.as_bytes()).await.is_err() {
                        return;
                    }

                    let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis();
                    let frames = vec![
                        format!("event: message_start\ndata: {{\"type\":\"message_start\",\"message\":{{\"id\":\"msg-{ts}\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-3\",\"content\":[]}}}}\n\n"),
                        format!("event: content_block_start\ndata: {{\"type\":\"content_block_start\",\"index\":0,\"content_block\":{{\"type\":\"text\",\"text\":\"\"}}}}\n\n"),
                        format!("event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":\"Hello\"}}}}\n\n"),
                        format!("event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":\" from Claude\"}}}}\n\n"),
                        format!("event: content_block_stop\ndata: {{\"type\":\"content_block_stop\",\"index\":0}}\n\n"),
                        format!("event: message_delta\ndata: {{\"type\":\"message_delta\",\"delta\":{{\"stop_reason\":\"end_turn\"}},\"usage\":{{\"output_tokens\":4}}}}\n\n"),
                        "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n".to_string(),
                    ];

                    for frame in &frames {
                        if socket.write_all(frame.as_bytes()).await.is_err() {
                            break;
                        }
                        if socket.flush().await.is_err() {
                            break;
                        }
                        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
                    }
                });
            }
            Err(e) => error!("Mock Anthropic: Accept error: {e}"),
        }
    }
}
