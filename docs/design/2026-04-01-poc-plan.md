# POC 阶段详细开发计划

**日期**: 2026-04-01
**阶段**: PoC (3-4 天)
**目标**: 验证 Pingora 能支持现有核心功能

## 1. 验证目标

| 序号 | 验证项           | 风险等级 | 说明                                    |
| ---- | ---------------- | -------- | --------------------------------------- |
| 1    | SSE 流式协议转换 | 高       | 复用现有协议转换代码，验证流式处理      |
| 2    | 并发限制保持     | 高       | 确保流结束时才释放并发许可              |
| 3    | Failover 机制    | 中       | 后端失败时自动切换到备选                |

## 2. 项目结构

```plaintext
poc/
├── Cargo.toml
├── src/
│   ├── main.rs              # 入口
│   ├── proxy.rs             # ProxyHttp 实现
│   ├── middleware/
│   │   ├── mod.rs
│   │   ├── concurrency.rs   # 并发限制中间件
│   │   └── protocol.rs      # 协议转换中间件
│   ├── strategy/
│   │   ├── mod.rs
│   │   ├── chain.rs         # 策略链执行器
│   │   └── failover.rs      # Failover 策略插件
│   └── backend.rs           # 后端管理
├── tests/
│   ├── test_streaming.rs    # 流式测试
│   ├── test_concurrency.rs  # 并发测试
│   └── test_failover.rs     # Failover 测试
└── README.md
```

## 3. 环境需求

### 3.1 完全自包含，无需额外环境

POC 阶段**不需要**准备任何外部基础设施：

| 工具        | 是否需要 | 原因                         | POC 替代方案                      |
| ----------- | -------- | ---------------------------- | --------------------------------- |
| Docker      | 否       | 无需容器化                   | 直接 `cargo run` 运行             |
| K8s         | 否       | 无需服务发现                 | 写死 `127.0.0.1` 本地端口         |
| Redis       | 否       | 无需统计存储                 | 跳过统计功能                      |
| Prometheus  | 否       | 无需指标采集                 | 日志输出验证                      |
| 真实 LLM    | 否       | 无需真实模型                 | 代码内嵌 Mock 后端                |
| SQLite      | 否       | 无需持久化                   | 内存存储或不存储                  |

**仅需**：Rust 工具链（已具备）

### 3.2 Mock 后端实现

POC 中所有后端均通过代码内嵌 Mock 实现：

```rust
/// 启动模拟 OpenAI 流式后端
async fn mock_openai_backend(port: u16) {
    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}"))
        .await
        .unwrap();
    
    tokio::spawn(async move {
        loop {
            let (socket, _) = listener.accept().await.unwrap();
            tokio::spawn(handle_mock_request(socket));
        }
    });
}

/// 手写 HTTP SSE 响应
async fn handle_mock_request(socket: tokio::net::TcpStream) {
    let response = "HTTP/1.1 200 OK\r\n\
        Content-Type: text/event-stream\r\n\
        \r\n\
        data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n\
        data: [DONE]\n\n";
    
    let (mut read, mut write) = socket.into_split();
    write.write_all(response.as_bytes()).await.unwrap();
}
```

更简单的方案：使用 `axum` 或 `warp` 快速搭建（dev-dependencies 中可选）。

### 3.3 端口分配

所有服务运行在本地不同端口：

| 服务                    | 地址                  | 说明                    |
| ----------------------- | --------------------- | ----------------------- |
| POC Proxy               | `127.0.0.1:18080`     | 待验证的 Pingora 代理   |
| Mock OpenAI Backend A   | `127.0.0.1:18001`     | 模拟故障后端            |
| Mock OpenAI Backend B   | `127.0.0.1:18002`     | 模拟正常后端            |
| Mock Anthropic Backend  | `127.0.0.1:18003`     | 用于协议转换测试        |

### 3.4 测试执行流程

```bash
# 1. 进入 POC 目录
cd poc

# 2. 编译并运行（会自动启动所有 mock 后端）
cargo run

# 3. 另一个终端执行测试
curl -N http://127.0.0.1:18080/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model":"test","messages":[],"stream":true}'

# 4. 观察输出
# - SSE 流是否正确
# - 协议转换是否正确（Anthropic → OpenAI）
# - 并发限制是否生效
# - Failover 是否触发

# 5. 运行自动化测试
cargo test
```

## 4. 任务分解

### Day 1: 基础框架搭建

#### 任务 1.1: 创建项目骨架 (2h)

**内容**:

- 创建 `poc/` 目录
- 初始化 Cargo 项目
- 添加依赖：pingora-core, pingora-proxy, pingora-server
- 复用 `llm-gateway-protocols` crate

**依赖配置**:

```toml
[dependencies]
pingora-core = "0.4"
pingora-proxy = "0.4"
pingora-server = "0.4"
pingora-http = "0.4"

llm-gateway-protocols = { path = "../crates/protocols" }

tokio = { version = "1", features = ["rt-multi-thread"] }
serde_json = "1"
log = "0.4"
env_logger = "0.11"
```

**验收标准**:

- `cargo check` 通过
- 能编译出可执行文件

#### 任务 1.2: 实现基础 Proxy (3h)

**内容**:
实现最简单的 `ProxyHttp`，只负责转发请求到固定后端。

```rust
use pingora_proxy::{ProxyHttp, Session};
use pingora_core::upstreams::peer::HttpPeer;
use async_trait::async_trait;

pub struct SimpleProxy {
    upstream_addr: String,
}

#[async_trait]
impl ProxyHttp for SimpleProxy {
    type CTX = ();

    fn new_ctx(&self) -> Self::CTX {}

    async fn upstream_peer(
        &self,
        _session: &mut Session,
        _ctx: &mut Self::CTX,
    ) -> Result<Box<HttpPeer>, Box<pingora_core::Error>> {
        let peer = HttpPeer::new(&self.upstream_addr, false, "".to_string());
        Ok(Box::new(peer))
    }
}
```

**测试方式**:

```bash
# 启动一个简单的后端（如 Python http.server）
python3 -m http.server 8080 &

# 启动代理
cargo run -- --port 8000 --upstream http://127.0.0.1:8080

# 测试
curl http://127.0.0.1:8000/
```

**验收标准**:

- 代理能正常转发 HTTP 请求
- 响应正确返回

#### 任务 1.3: 复用协议转换代码 (2h)

**内容**:
验证现有的 `SseCollector` 和 `StreamingCollector` 能否在 Pingora 中使用。

关键问题验证：

- Pingora 的响应 Body 类型能否转换为流
- 能否在流上逐帧处理 SSE 数据

```rust
use pingora_http::ResponseHeader;
use pingora_core::http_proxy::write_response_header;
use bytes::Bytes;
use http::StatusCode;

/// 流式响应转换处理
async fn transform_streaming_response(
    session: &mut Session,
    converter: Box<dyn StreamingCollector>,
) -> Result<(), Box<pingora_core::Error>> {
    let mut collector = SseCollector::new();
    let converter = std::sync::Mutex::new(converter);

    // 获取响应体流
    // 注意：这里需要验证 Pingora 的 Body 处理方式

    Ok(())
}
```

**验收标准**:

- 能调用 `SseCollector::collect()` 解析 SSE 数据
- 能通过 `StreamingCollector::process()` 转换协议

### Day 2: 流式协议转换验证

#### 任务 2.1: 模拟 LLM 后端 (2h)

**内容**:
创建一个模拟的 OpenAI/Anthropic 后端，用于测试流式响应。

```rust
/// 模拟 OpenAI 流式后端
async fn mock_openai_backend() {
    // 返回 SSE 格式的流式响应
    // data: {"id": "...", "choices": [{"delta": {"content": "Hello"}}]}
    // data: {"id": "...", "choices": [{"delta": {"content": " World"}}]}
    // data: [DONE]
}
```

**验收标准**:

- 后端能返回 SSE 流式响应
- 能模拟 OpenAI 和 Anthropic 两种格式

#### 任务 2.2: 实现 ProtocolConversion 中间件 (4h)

**内容**:
在 Pingora 中实现协议转换，复用 `llm-gateway-protocols`。

核心代码框架：

```rust
use llm_gateway_protocols::{Protocol, SseCollector, SseMessage};
use llm_gateway_protocols::streaming::{StreamingCollector, AnthropicToOpenai, OpenaiToAnthropic};
use std::sync::Mutex;
use futures::StreamExt;

pub struct ProtocolConversionMiddleware {
    target_protocol: Protocol,
}

impl ProtocolConversionMiddleware {
    /// 转换流式响应
    pub async fn transform_body<B>(
        &self,
        body: B,
        converter: Box<dyn StreamingCollector>,
    ) -> impl Body
    where
        B: Body<Data = Bytes>,
    {
        let collector = Mutex::new(SseCollector::new());
        let converter = Mutex::new(converter);

        // 将 Body 转换为 Stream，逐帧处理
        body.into_data_stream().map(move |result| {
            match result {
                Ok(bytes) => {
                    let mut coll = collector.lock().unwrap();
                    let msgs = coll.collect(&bytes).unwrap();

                    let mut conv = converter.lock().unwrap();
                    let mut output = Vec::new();

                    for msg in msgs {
                        let converted = conv.process(msg).unwrap();
                        for out_msg in converted {
                            output.push(out_msg.to_string());
                        }
                    }

                    Ok(Bytes::from(output.concat()))
                }
                Err(e) => Err(e),
            }
        })
    }
}
```

**关键验证点**:

1. Pingora 的 Body 能否转换为 Stream
2. 转换后的 Stream 能否作为响应返回
3. SSE 帧边界是否正确处理

**验收标准**:

- 发送 Anthropic 格式的流式请求
- 代理返回 OpenAI 格式的流式响应
- 客户端能正确接收转换后的 SSE 数据

#### 任务 2.3: 流式测试 (2h)

**测试用例**:

```rust
#[tokio::test]
async fn test_streaming_conversion() {
    // 启动模拟后端（Anthropic 格式）
    let backend = start_mock_anthropic_backend().await;

    // 启动代理（转换为 OpenAI 格式）
    let proxy = start_proxy(&backend).await;

    // 发送请求
    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/v1/chat/completions", proxy))
        .json(&json!({
            "model": "test-model",
            "messages": [{"role": "user", "content": "Hello"}],
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    // 验证响应格式为 OpenAI SSE
    let body = response.text().await.unwrap();
    assert!(body.contains("data: {"));
    assert!(body.contains("chat.completion.chunk"));
}
```

**验收标准**:

- 测试通过
- 流式数据实时转换（非缓冲后一次性返回）

### Day 3: 并发限制验证

#### 任务 3.1: 理解 Pingora 上下文生命周期 (2h)

**内容**:
研究 Pingora 的 `CTX` 生命周期，确认能否保持状态到流结束。

关键问题：

- `CTX` 在何时创建和销毁？
- 能否在 `CTX` 中存储 Drop 类型来实现 RAII？

```rust
pub struct ConcurrencyContext {
    permit: Option<OwnedSemaphorePermit>,
}

impl Drop for ConcurrencyContext {
    fn drop(&mut self) {
        // permit 在这里释放
        println!("Concurrency permit released");
    }
}
```

**验证方法**:

1. 在 `new_ctx()` 中打印日志
2. 在 `CTX` 的 Drop 中打印日志
3. 观察流式请求的生命周期

**验收标准**:

- 确认 `CTX` 在流结束后才 Drop

#### 任务 3.2: 实现 ConcurrencyLimit 中间件 (3h)

**内容**:
实现并发限制，使用 Semaphore 控制并发数。

```rust
use tokio::sync::{Semaphore, OwnedSemaphorePermit};
use std::sync::Arc;
use dashmap::DashMap;

pub struct ConcurrencyLimitLayer {
    semaphores: DashMap<String, Arc<Semaphore>>,
    default_limit: usize,
}

impl ConcurrencyLimitLayer {
    pub async fn acquire(&self, model: &str) -> Result<OwnedSemaphorePermit, ()> {
        let sem = self.semaphores
            .entry(model.to_string())
            .or_insert_with(|| Arc::new(Semaphore::new(self.default_limit)))
            .clone();

        sem.try_acquire_owned().map_err(|_| ())
    }
}

/// 在 ProxyHttp 中使用
#[async_trait]
impl ProxyHttp for ProxyWithConcurrency {
    type CTX = ConcurrencyContext;

    fn new_ctx(&self) -> Self::CTX {
        ConcurrencyContext { permit: None }
    }

    async fn upstream_peer(
        &self,
        session: &mut Session,
        ctx: &mut Self::CTX,
    ) -> Result<Box<HttpPeer>, Box<pingora_core::Error>> {
        // 获取模型名
        let model = extract_model(session).await?;

        // 尝试获取许可
        match self.concurrency.acquire(&model).await {
            Ok(permit) => {
                ctx.permit = Some(permit);
                // 继续处理
            }
            Err(_) => {
                // 返回 503
                return Err(...);
            }
        }

        // 选择后端...
    }
}
```

**验收标准**:

- 并发请求数被正确限制
- 超过限制时返回 503
- 流结束后并发计数减少

#### 任务 3.3: 并发测试 (3h)

**测试用例**:

```rust
#[tokio::test]
async fn test_concurrency_limit() {
    // 设置并发限制为 2
    let proxy = start_proxy_with_concurrency_limit(2).await;

    // 发送 3 个并发流式请求
    let mut handles = vec![];
    for i in 0..3 {
        let handle = tokio::spawn(async move {
            let client = reqwest::Client::new();
            let response = client
                .post(format!("http://{}/v1/chat/completions", proxy))
                .json(&json!({
                    "model": "test-model",
                    "messages": [],
                    "stream": true
                }))
                .send()
                .await
                .unwrap();

            (i, response.status())
        });
        handles.push(handle);
    }

    // 验证：2 个成功，1 个 503
    let results: Vec<_> = futures::future::join_all(handles)
        .await
        .into_iter()
        .map(|r| r.unwrap())
        .collect();

    let success_count = results.iter().filter(|(_, s)| s.is_success()).count();
    let limited_count = results.iter().filter(|(_, s)| s.as_u16() == 503).count();

    assert_eq!(success_count, 2);
    assert_eq!(limited_count, 1);
}

#[tokio::test]
async fn test_concurrency_release_after_stream() {
    // 验证流结束后并发数减少
    // 先发送 2 个长流式请求，等 1 个完成后，第 3 个应该能成功
}
```

**验收标准**:

- 并发限制生效
- 流结束后许可正确释放
- 后续请求能被处理

### Day 4: Failover 验证

#### 任务 4.1: 实现策略链框架 (3h)

**内容**:
实现简化的策略链，支持 Failover。

```rust
/// 策略插件 trait（简化版）
#[async_trait]
pub trait RoutingPlugin: Send + Sync {
    async fn select_backend(
        &self,
        ctx: &Context,
    ) -> Result<Option<Backend>, RoutingError>;
}

/// Failover 策略
pub struct FailoverStrategy {
    backends: Vec<String>,
    health_checker: HealthChecker,
}

#[async_trait]
impl RoutingPlugin for FailoverStrategy {
    async fn select_backend(&self, ctx: &Context) -> Result<Option<Backend>, RoutingError> {
        for backend_addr in &self.backends {
            // 健康检查
            if !self.health_checker.is_healthy(backend_addr).await {
                continue;
            }

            // 尝试连接
            match try_connect(backend_addr).await {
                Ok(backend) => return Ok(Some(backend)),
                Err(e) => {
                    self.health_checker.record_failure(backend_addr).await;
                    log::warn!("Backend {backend_addr} failed: {e}");
                    continue;
                }
            }
        }

        Err(RoutingError::NoAvailableBackend)
    }
}
```

**验收标准**:

- 策略链能按顺序执行
- 找到可用后端后停止

#### 任务 4.2: 集成到 Proxy (3h)

**内容**:
将策略链集成到 `ProxyHttp`。

```rust
#[async_trait]
impl ProxyHttp for ProxyWithStrategy {
    type CTX = StrategyContext;

    async fn upstream_peer(
        &self,
        session: &mut Session,
        ctx: &mut Self::CTX,
    ) -> Result<Box<HttpPeer>, Box<pingora_core::Error>> {
        // 获取模型名
        let model = extract_model(session).await?;

        // 执行策略链
        match self.strategy_chain.execute(&model).await {
            Ok(backend) => {
                ctx.selected_backend = Some(backend.clone());
                let peer = HttpPeer::new(&backend.addr, false, "".to_string());
                Ok(Box::new(peer))
            }
            Err(RoutingError::NoAvailableBackend) => {
                // 返回 503
                let mut response = ResponseHeader::build(StatusCode::SERVICE_UNAVAILABLE, None)?;
                session.write_response_header(Box::new(response), true).await?;
                Err(...)
            }
        }
    }

    async fn logging(
        &self,
        session: &mut Session,
        e: Option<&pingora_core::Error>,
        ctx: &mut Self::CTX,
    ) {
        // 记录结果到健康检查器
        if let Some(backend) = &ctx.selected_backend {
            if e.is_some() {
                self.health_checker.record_failure(&backend.addr).await;
            } else {
                self.health_checker.record_success(&backend.addr).await;
            }
        }
    }
}
```

**验收标准**:

- 能按策略选择后端
- 后端失败时记录健康状态

#### 任务 4.3: Failover 测试 (2h)

**测试用例**:

```rust
#[tokio::test]
async fn test_failover_on_backend_failure() {
    // 启动两个后端：backend1（会失败），backend2（正常）
    let backend1 = start_failing_backend().await;  // 返回 500
    let backend2 = start_healthy_backend().await;  // 正常响应

    // 配置 Failover: [backend1, backend2]
    let proxy = start_proxy_with_failover(vec![&backend1, &backend2]).await;

    // 发送请求
    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/v1/chat/completions", proxy))
        .json(&json!({"model": "test", "messages": []}))
        .send()
        .await
        .unwrap();

    // 应该成功（通过 backend2）
    assert!(response.status().is_success());

    // 验证 backend1 被标记为不健康
    assert!(!proxy.is_healthy(&backend1).await);
}

#[tokio::test]
async fn test_failover_all_backends_down() {
    // 所有后端都失败时返回 503
}

#[tokio::test]
async fn test_failover_retry_mechanism() {
    // 验证重试次数和退避策略
}
```

**验收标准**:

- 第一个后端失败时自动切换到第二个
- 健康状态正确更新
- 所有后端失败时返回 503

## 5. 整合测试

### 5.1 端到端测试

```rust
#[tokio::test]
async fn test_end_to_end() {
    // 启动两个后端：
    // - backend1: OpenAI 协议，会随机失败
    // - backend2: Anthropic 协议，正常

    // 配置代理：
    // - 并发限制：5
    // - 协议转换：Anthropic -> OpenAI
    // - Failover: [backend1, backend2]

    // 发送 10 个并发流式请求
    // 验证：
    // 1. 并发数不超过 5
    // 2. 失败时自动切换到 backend2
    // 3. 协议转换正确
    // 4. 流式响应正常
}
```

### 5.2 性能基准

```rust
#[tokio::test]
async fn benchmark_throughput() {
    // 对比：
    // 1. 直接请求后端
    // 2. 通过 Pingora 代理
    //
    // 验证代理 overhead < 10%
}
```

## 6. 验收标准汇总

| 验证项              | 验收标准                               | 优先级 |
| ------------------- | -------------------------------------- | ------ |
| SSE 流式协议转换    | Anthropic → OpenAI 转换正确，实时流式  | P0     |
| 并发限制            | 超过限制返回 503，流结束释放许可       | P0     |
| Failover            | 后端失败自动切换，健康状态更新         | P1     |
| 端到端              | 三者能协同工作                         | P0     |

## 7. 风险与回退

| 风险                          | 影响 | 缓解措施                              |
| ----------------------------- | ---- | ------------------------------------- |
| Pingora Body 流式处理不兼容   | 高   | 尝试使用 Pingora 的 TransformBody API |
| CTX 生命周期不符合预期        | 高   | 研究 HttpProxy 的回调机制             |
| 协议转换性能问题              | 中   | 优化锁使用，考虑无锁方案              |

## 8. 输出物

1. **代码**: `poc/` 目录下的完整代码
2. **测试报告**: 所有测试用例的执行结果
3. **技术结论**:
   - Pingora 是否适合本项目
   - 需要解决的技术难点
   - 正式开发的估算调整

## 9. 决策点

PoC 完成后，团队需要根据以下标准决策是否继续：

| 检查项                  | Go 标准                  | No-Go 标准               |
| ----------------------- | ------------------------ | ------------------------ |
| SSE 流式转换            | 功能正确，延迟可接受     | 无法流式处理或延迟过高   |
| 并发控制                | 能正确保持到流结束       | 流中途释放或无法追踪     |
| Failover                | 能自动切换后端           | 失败检测不准确           |
| 开发复杂度              | 代码清晰，维护性好       | 过于复杂或 hack 过多     |

**决策会议**: Day 4 下午进行 PoC 评审会议，决定是否进入 Phase 1。
