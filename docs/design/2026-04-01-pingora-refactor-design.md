# Pingora 重构设计文档

**日期**: 2026-04-01
**状态**: 草稿
**作者**: LLM Gateway Team

## 目录

1. [重构必要性与方案优势](#1-重构必要性与方案优势)
2. [架构设计](#2-架构设计)
3. [插件与中间件系统](#3-插件与中间件系统)
4. [部署指南](#4-部署指南)
5. [迁移计划](#5-迁移计划)

## 1. 重构必要性与方案优势

### 1.1 当前架构的局限性

当前 LLM Gateway 基于 Hyper 1.x 构建，虽然功能完整，但在以下方面存在局限：

| 维度           | 现状                         | 问题                             |
| -------------- | ---------------------------- | -------------------------------- |
| HTTP/2 支持    | 仅支持 HTTP/1.1              | 无法利用 HTTP/2 多路复用         |
| TLS 性能       | hyper-rustls 基础配置        | 无 TLS 会话复用                  |
| 连接池         | 手动管理                     | 缺乏智能连接复用                 |
| 重试机制       | 手动 loop 重试               | 缺乏指数退避策略                 |
| 可观测性       | 基础日志                     | 无指标暴露                       |
| 云原生集成     | 静态配置文件                 | 无法动态发现 K8s 服务            |
| 优雅关闭       | 无 SIGTERM 处理              | 容器重启时请求中断               |

### 1.2 为什么选择 Pingora

Pingora 是 Cloudflare 开源的 Rust 代理框架，具有以下优势：

| 特性                      | 价值                       |
| ------------------------- | -------------------------- |
| HTTP/1.1 + HTTP/2 双栈    | 自动协商                   |
| 内置连接池                | 智能复用                   |
| 可插拔 Service 架构       | 中间件模式                 |
| Prometheus 集成           | 内置指标暴露               |
| 优雅关闭                  | 内置 SIGTERM 处理          |
| Cloudflare 生产验证       | 大规模流量验证             |

### 1.3 重构后的能力对比

| 能力             | 当前 (Hyper)   | 重构后 (Pingora) |
| ---------------- | -------------- | ---------------- |
| HTTP/2 支持      | ❌             | ✅               |
| TLS 会话复用     | ❌             | ✅               |
| 智能连接池       | ⚠️ 手动        | ✅ 内置          |
| 重试策略         | ⚠️ 简单循环    | ✅ 指数退避      |
| Prometheus 指标  | ❌             | ✅               |
| OpenTelemetry    | ❌             | ✅               |
| K8s 服务发现     | ❌             | ✅               |
| 优雅关闭         | ❌             | ✅               |

## 2. 架构设计

### 2.1 整体架构图

```plaintext
┌─────────────────────────────────────────────────────────────────────┐
│                     Pingora HttpServer                              │
│                     (Port 8000/8001)                                │
└─────────────────────────────┬───────────────────────────────────────┘
                              │
┌─────────────────────────────▼───────────────────────────────────────┐
│                     Router Service                                  │
│  - 模型别名解析                                                     │
│  - /v1/models 端点处理                                              │
│  - 策略链组装                                                       │
└─────────────────────────────┬───────────────────────────────────────┘
                              │
        ┌─────────────────────┼─────────────────────┐
        │                     │                     │
┌───────▼───────┐    ┌────────▼────────┐   ┌──────▼──────┐
│  Middleware   │    │  Middleware     │   │ Middleware  │
│  Chain        │    │  Chain          │   │ Chain       │
│  (模型A)      │    │  (模型B)        │   │ (模型C)     │
│               │    │                 │   │             │
│ - Metrics     │    │ - Metrics       │   │ - Metrics   │
│ - Tracing     │    │ - Tracing       │   │ - Tracing   │
│ - Protocol    │    │ - Protocol      │   │ - Protocol  │
└───────┬───────┘    └────────┬────────┘   └──────┬──────┘
        │                     │                    │
        └─────────────────────┼────────────────────┘
                              │
┌─────────────────────────────▼───────────────────────────────────────┐
│                     RoutingStrategy (Plugin)                        │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │  Strategy Chain (按顺序执行)                                 │   │
│  │  ┌─────────────┐    ┌─────────────┐    ┌─────────────────┐  │   │
│  │  │ Concurrency │───▶│  Failover   │───▶│ LoadBalance     │  │   │
│  │  │   Limit     │    │  (Sequence) │    │ (RoundRobin/Random)│   │
│  │  └─────────────┘    └─────────────┘    └─────────────────┘  │   │
│  └─────────────────────────────────────────────────────────────┘   │
└─────────────────────────────┬───────────────────────────────────────┘
                              │
        ┌─────────────────────┼─────────────────────┐
        │                     │                     │
┌───────▼───────┐    ┌────────▼────────┐   ┌──────▼──────┐
│  Backend Pool │    │  Backend Pool   │   │ External    │
│  (sglang-35b) │    │  (sglang-122b)  │   │ API         │
│               │    │                 │   │ (Aliyun)    │
│ - 172.17...   │    │ - 172.17...     │   │             │
└───────────────┘    └─────────────────┘   └─────────────┘
```

### 2.2 核心组件职责

#### 2.2.1 HttpServerService

Pingora 的入口点，替代当前的 `serve.rs` 中的 `TcpListener` 循环。

**职责**：

- 监听指定端口（支持多端口）
- TLS 终止（可选）
- HTTP/1.1 和 HTTP/2 协议协商

#### 2.2.2 Router Service

替代当前的 `InputNode` 路由逻辑。

**职责**：

- 解析请求体中的 `model` 字段
- 处理模型别名映射
- 处理 `/v1/models` 端点
- 根据模型获取对应的策略链配置并组装执行

#### 2.2.3 中间件层（Middleware Chain）

**与插件的区别**：

- **中间件**：横切关注点，所有请求统一处理（Metrics、Tracing、Logging）
- **插件**：路由策略，决定请求如何流转到后端（Concurrency、Failover、LoadBalance）

**标准中间件接口**：

```rust
#[async_trait]
pub trait Middleware: Send + Sync + 'static {
    fn name(&self) -> &'static str;

    /// 返回 true 继续处理，false 中断
    async fn request_filter(&self, req: &mut RequestHeader, ctx: &mut Context) -> Result<bool>;

    async fn response_filter(&self, resp: &mut ResponseHeader, ctx: &Context) -> Result<()>;
}
```

#### 2.2.4 路由策略插件系统 (RoutingStrategy)

**核心创新**：将当前的"虚节点"概念抽象为可组合的策略插件。

```rust
/// 路由策略插件 trait
#[async_trait]
pub trait RoutingPlugin: Send + Sync + 'static {
    fn name(&self) -> &'static str;

    /// 执行路由决策
    ///
    /// # Returns
    /// - Ok(Some(backend)): 找到后端，停止链执行
    /// - Ok(None): 未找到，继续下一个插件
    /// - Err(e): 错误，根据配置决定是否重试/失败
    async fn select_backend(
        &self,
        ctx: &Context,
        upstreams: &[UpstreamPeer]
    ) -> Result<Option<SelectedBackend>>;

    /// 请求完成后的回调（用于状态更新）
    async fn on_request_complete(&self, result: &RequestResult, ctx: &Context);
}

/// 策略链执行器
pub struct StrategyChain {
    plugins: Vec<Arc<dyn RoutingPlugin>>,
}

impl StrategyChain {
    pub async fn execute(&self, ctx: &Context) -> Result<SelectedBackend> {
        for plugin in &self.plugins {
            match plugin.select_backend(ctx, &ctx.upstreams).await? {
                Some(backend) => return Ok(backend),
                None => continue,
            }
        }
        Err(RoutingError::NoAvailableBackend)
    }
}
```

#### 2.2.5 内置策略插件

| 插件名             | 职责                 | 对应现有节点                | 可配置参数                      |
| ------------------ | -------------------- | --------------------------- | ------------------------------- |
| ConcurrencyLimit   | 并发数限制           | ConcurrencyNode             | max_concurrent                  |
| TokenBucket        | 令牌桶限流           | 新增                        | tokens_per_sec, burst           |
| FailoverSequence   | 顺序尝试             | SequenceNode                | retries, backoff_strategy       |
| RoundRobin         | 轮询负载均衡         | 新增                        | weight-aware                    |
| WeightedRandom     | 加权随机             | 新增                        | weights                         |
| HealthAware        | 健康检查过滤         | HealthMonitor               | failure_threshold, cooldown     |

**组合示例**：

```rust
// 当前配置：
// [node."qwen3.5-35b-a3b"]
// sequence = ["limit-sglang-qwen-35b", "aliyun"]
//
// [node.limit-sglang-qwen-35b]
// concurrency = { max = 8, successor = "sglang-qwen3.5-35b-a3b" }

// 重构后策略链：
let strategy = StrategyChain::builder()
    .add(ConcurrencyLimit::new(8))
    .add(FailoverSequence::new()
        .add_backend("sglang-qwen-35b", health_check.clone())
        .add_backend("aliyun", health_check.clone())
        .with_retry(3, ExponentialBackoff::default()))
    .build();
```

#### 2.2.6 Upstream 管理

替代当前的静态后端配置。

**Upstream 发现策略**：

| 部署环境       | 发现方式                            |
| -------------- | ----------------------------------- |
| Kubernetes     | kube crate 监听 Endpoints           |
| VM/裸机        | 静态配置 + Consul 可选              |
| 混合           | 抽象 UpstreamDiscovery trait        |

### 2.3 数据流

#### 2.3.1 请求处理流程

1. Client 发送 POST /v1/chat/completions
2. HttpServerService 接收请求
3. Router Service 解析 model 字段，应用别名映射
4. **中间件链**依次处理（横切关注点）：
   - MetricsMiddleware：记录请求开始
   - TracingMiddleware：创建 Span
   - ProtocolDetectionMiddleware：检测协议类型
5. **策略插件链**执行（路由决策）：
   - ConcurrencyLimit：获取许可/等待/拒绝
   - FailoverSequence：尝试 backend A
     - 失败？记录失败，尝试 backend B
     - 成功？返回 SelectedBackend
6. Proxy 转发到选中的后端
7. 流式接收响应
8. **响应中间件链**（反向）处理
9. 策略插件的 `on_request_complete` 回调
10. 返回给 Client

#### 2.3.2 失败重试流程

```plaintext
StrategyChain 开始执行
  ↓
ConcurrencyLimit (获取许可成功)
  ↓
FailoverSequence 尝试 Backend A
  ↓
连接超时
  ↓
HealthMonitor 记录失败
  ↓
FailoverSequence 判断：还有重试次数？
  ↓ 是
等待指数退避时间 (backoff)
  ↓
尝试 Backend B
  ↓
成功
  ↓
返回 SelectedBackend
```

### 2.4 模块划分

重构后的 crate 结构：

```plaintext
llm-gateway/
├── crates/
│   ├── config/              # 配置解析（重写为 Pingora 风格）
│   ├── protocols/           # 保持不变
│   ├── statistics/          # 保持不变
│   └── strategy/            # 新增：路由策略插件框架
│       ├── src/
│       │   ├── lib.rs       # 插件 trait 定义
│       │   ├── chain.rs     # 策略链执行器
│       │   ├── plugins/
│       │   │   ├── concurrency.rs
│       │   │   ├── token_bucket.rs
│       │   │   ├── failover.rs
│       │   │   ├── round_robin.rs
│       │   │   └── health_aware.rs
│       │   └── context.rs   # 路由上下文
├── src/
│   ├── main.rs              # 入口，组装 Pingora
│   ├── router.rs            # Router Service
│   ├── proxy.rs             # ProxyHttp 实现
│   ├── middleware/          # 中间件实现
│   │   ├── metrics.rs
│   │   ├── tracing.rs
│   │   └── protocol.rs
│   └── upstream.rs          # Upstream 管理
└── Cargo.toml
```

## 3. 插件与中间件系统

### 3.1 双层架构

```plaintext
┌────────────────────────────────────────────┐
│              Request/Response               │
└──────────────────┬─────────────────────────┘
                   │
┌──────────────────▼─────────────────────────┐
│           Middleware Chain                 │  ← 横切关注点（所有请求）
│  - Metrics (Prometheus 指标)               │
│  - Tracing (OpenTelemetry)                 │
│  - Logging                                 │
│  - Protocol Detection                      │
└──────────────────┬─────────────────────────┘
                   │
┌──────────────────▼─────────────────────────┐
│         Routing Strategy Chain             │  ← 路由决策（模型特定）
│  - ConcurrencyLimit                        │
│  - TokenBucket                             │
│  - HealthAware                             │
│  - FailoverSequence / LoadBalance          │
└──────────────────┬─────────────────────────┘
                   │
┌──────────────────▼─────────────────────────┐
│            Upstream Peers                  │
└────────────────────────────────────────────┘
```

### 3.2 中间件设计

**核心 trait**：

```rust
#[async_trait]
pub trait Middleware: Send + Sync + 'static {
    fn name(&self) -> &'static str;

    async fn request_filter(
        &self,
        session: &mut Session,
        ctx: &mut RouterContext
    ) -> Result<bool>;

    async fn response_filter(
        &self,
        session: &mut Session,
        ctx: &RouterContext
    ) -> Result<()>;
}
```

**内置中间件**：

| 中间件             | 职责                    | 备注                     |
| ------------------ | ----------------------- | ------------------------ |
| MetricsMiddleware  | Prometheus 指标         | 请求计数、延迟直方图     |
| TracingMiddleware  | OpenTelemetry 追踪      | 创建 Span，注入上下文    |
| LoggingMiddleware  | 结构化日志              | 请求/响应日志            |
| ProtocolMiddleware | 协议检测与转换准备      | 标记需转换的协议         |

### 3.3 路由策略插件详解

#### 3.3.1 ConcurrencyLimit 插件

```rust
pub struct ConcurrencyLimit {
    semaphores: DashMap<String, Arc<Semaphore>>,
    limits: HashMap<String, usize>,
}

#[async_trait]
impl RoutingPlugin for ConcurrencyLimit {
    async fn select_backend(
        &self,
        ctx: &Context,
        _upstreams: &[UpstreamPeer]
    ) -> Result<Option<SelectedBackend>> {
        let model = ctx.model();
        let limit = self.limits.get(model).copied().unwrap_or(usize::MAX);
        let sem = self.semaphores
            .entry(model.to_string())
            .or_insert_with(|| Arc::new(Semaphore::new(limit)))
            .clone();

        match sem.try_acquire() {
            Ok(permit) => {
                ctx.extensions_mut().insert(permit);
                Ok(None) // 继续下一个插件
            }
            Err(_) => Err(RoutingError::OverConcurrency)
        }
    }

    async fn on_request_complete(&self, _result: &RequestResult, _ctx: &Context) {
        // permit 自动释放
    }
}
```

#### 3.3.2 FailoverSequence 插件

```rust
pub struct FailoverSequence {
    backends: Vec<String>,
    retry_policy: RetryPolicy,
    health_monitor: Arc<HealthMonitor>,
}

pub struct RetryPolicy {
    max_retries: usize,
    backoff: ExponentialBackoff,
    retryable_errors: Vec<ErrorKind>,
}

#[async_trait]
impl RoutingPlugin for FailoverSequence {
    async fn select_backend(
        &self,
        ctx: &Context,
        upstreams: &[UpstreamPeer]
    ) -> Result<Option<SelectedBackend>> {
        for backend_name in &self.backends {
            // 健康检查
            if !self.health_monitor.is_healthy(backend_name).await {
                continue;
            }

            let upstream = upstreams.iter()
                .find(|u| u.name() == backend_name)
                .ok_or_else(|| RoutingError::BackendNotFound)?;

            // 尝试发送（带重试）
            match self.try_with_retry(ctx, upstream).await {
                Ok(backend) => return Ok(Some(backend)),
                Err(e) => {
                    self.health_monitor.record_failure(backend_name).await;
                    warn!("Backend {backend_name} failed: {e}");
                    continue;
                }
            }
        }

        Err(RoutingError::NoAvailableBackend)
    }
}
```

#### 3.3.3 TokenBucket 插件（新增）

```rust
pub struct TokenBucketPlugin {
    buckets: DashMap<String, TokenBucket>,
}

struct TokenBucket {
    tokens: Mutex<f64>,
    capacity: f64,
    refill_rate: f64,
    last_refill: Mutex<Instant>,
}
```

### 3.4 自定义策略插件

用户可以实现自定义插件：

```rust
/// 示例：基于时间的路由插件
pub struct TimeBasedRouting;

#[async_trait]
impl RoutingPlugin for TimeBasedRouting {
    async fn select_backend(
        &self,
        ctx: &Context,
        upstreams: &[UpstreamPeer]
    ) -> Result<Option<SelectedBackend>> {
        let hour = chrono::Local::now().hour();

        // 工作时间使用本地后端，非工作时间使用云服务商
        let backend = if (9..18).contains(&hour) {
            upstreams.iter().find(|u| !u.is_external())
        } else {
            upstreams.iter().find(|u| u.is_external())
        };

        Ok(backend.map(|u| SelectedBackend::from(u)))
    }
}

// 注册插件
strategy_chain
    .add(ConcurrencyLimit::new(8))
    .add(TimeBasedRouting)  // 自定义插件
    .add(FailoverSequence::new(["local", "cloud"]))
```

### 3.5 配置格式

```toml
[server]
port = 8000

# 模型别名
[alias]
"data/Qwen3.5-35B-A3B" = "qwen3.5-35b-a3b"

# 模型路由配置
[model."qwen3.5-35b-a3b"]
# 策略链：按顺序执行
strategy = [
    { type = "concurrency_limit", max = 8 },
    { type = "health_aware" },
    { type = "failover", backends = ["sglang-35b", "aliyun"], retries = 3 },
]

[model."qwen3.5-122b-a10b"]
strategy = [
    { type = "concurrency_limit", max = 4 },
    { type = "token_bucket", tokens_per_second = 100, burst = 200 },
    { type = "round_robin", backends = ["sglang-122b-1", "sglang-122b-2"] },
]

# 后端定义
[backend."sglang-35b"]
url = "http://172.17.250.163:30001"
protocol = "openai"

[backend."sglang-122b-1"]
url = "http://172.17.250.163:30002"
protocol = "openai"

[backend."sglang-122b-2"]
url = "http://172.17.250.163:30003"
protocol = "openai"

[backend."aliyun"]
url = "https://dashscope.aliyuncs.com/apps/anthropic"
protocol = "anthropic"
api_key = "$ALIYUN_API_KEY"

# 健康检查
[health_check]
interval = 10
failure_threshold = 3
cooldown = 300

# 统计
[statistics]
enabled = true
db_path = "stats.db"
retention_days = 7

# 可观测性
[observability]
metrics_port = 9090
tracing_endpoint = "http://jaeger:4317"
```

### 3.6 可观测性

#### 3.6.1 Prometheus 指标

| 指标名                                        | 说明           |
| --------------------------------------------- | -------------- |
| llm_gateway_requests_total                    | 请求计数       |
| llm_gateway_request_duration_seconds          | 延迟直方图     |
| llm_gateway_active_connections                | 活跃连接数     |
| llm_gateway_concurrent_requests               | 当前并发数     |
| llm_gateway_upstream_health                   | 后端健康状态   |
| llm_gateway_rate_limit_hits                   | 限流触发次数   |

#### 3.6.2 OpenTelemetry 追踪

导出后端：

- Jaeger（开发环境）
- Tempo / Zipkin（生产环境）

## 4. 部署指南

### 4.1 Docker 镜像

```dockerfile
FROM rust:1.85 AS builder
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
COPY --from=builder /app/target/release/llm-gateway /usr/local/bin/
EXPOSE 8000 9090
ENTRYPOINT ["llm-gateway"]
```

### 4.2 Kubernetes 部署

#### 4.2.1 Deployment

关键配置：

- replicas: 3（高可用）
- prometheus.io/scrape: "true"（指标抓取）
- livenessProbe / readinessProbe（健康检查）
- resources（资源限制）

#### 4.2.2 ConfigMap

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: llm-gateway-config
data:
  config.toml: |
    [server]
    port = 8000

    [model."qwen3.5-35b-a3b"]
    strategy = [
        { type = "concurrency_limit", max = 8 },
        { type = "failover", backends = ["sglang-35b", "aliyun"] },
    ]
```

**热加载**：

```bash
kubectl exec <pod> -- kill -HUP 1
```

### 4.3 VM/裸机部署

#### 4.3.1 Systemd 服务

```ini
[Unit]
Description=LLM Gateway Service
After=network.target

[Service]
Type=notify
ExecStart=/usr/local/bin/llm-gateway /etc/llm-gateway/config.toml
Restart=on-failure
KillSignal=SIGTERM
TimeoutStopSec=30

[Install]
WantedBy=multi-user.target
```

### 4.4 混合部署配置

抽象 `UpstreamDiscovery` trait：

```rust
pub trait UpstreamDiscovery: Send + Sync {
    async fn discover(&self) -> Result<Vec<UpstreamPeer>>;
}

// K8s 实现
pub struct K8sDiscovery {
    client: kube::Client,
    service_name: String,
}

// 静态配置实现（VM）
pub struct StaticDiscovery {
    peers: Vec<UpstreamPeer>,
}
```

## 5. 迁移计划

### 5.1 核心设计原则

1. **配置重写**：不兼容旧配置格式，直接采用新设计
2. **插件优先**：路由策略全部抽象为可组合插件
3. **快速验证**：尽早通过 PoC 验证关键路径

### 5.2 阶段划分

| 阶段  | 时间       | 主要内容                           |
| ----- | ---------- | ---------------------------------- |
| PoC   | 3-4 天     | 关键技术验证                       |
| 1     | Week 1     | 基础框架 + 策略插件框架            |
| 2     | Week 2     | 核心插件实现                       |
| 3     | Week 3     | 协议转换集成 + 可观测性            |
| 4     | Week 4     | 管理 API + 测试完善                |

### 5.3 详细步骤

#### Phase PoC：关键技术验证

**目标**：验证 Pingora 能支持现有核心功能

**任务**：

- 用 Pingora 搭建最小化 SSE 代理
- 验证流式协议转换（OpenAI ↔ Anthropic）
- 验证 ConcurrencyLimit 实现（保持并发计数到流结束）
- 验证 Failover 重试机制

**验收标准**：

- SSE 流式响应能正确转换
- 并发限制在流式请求中生效（连接结束时才释放许可）
- 后端失败时能自动切换到备选

#### Phase 1：基础框架

**任务**：

- 创建 Pingora 项目骨架
- 实现 Router Service（模型路由）
- 实现策略插件框架（trait + chain 执行器）
- 集成基础中间件（Logging）
- 配置解析（新格式）

**验收标准**：

- 能启动 HTTP 服务器并监听端口
- 能根据 model 字段路由到不同策略链
- 配置正确解析并加载

#### Phase 2：核心插件

**任务**：

- ConcurrencyLimit 插件
- FailoverSequence 插件（SequenceNode 替代）
- HealthAware 插件（健康检查集成）
- TokenBucket 插件（新增）
- RoundRobin 插件（新增）

**验收标准**：

```rust
// 能正确执行策略链
let strategy = StrategyChain::builder()
    .add(ConcurrencyLimit::new(8))
    .add(FailoverSequence::new(["backend-a", "backend-b"]))
    .build();
```

- 并发限制生效
- 后端故障时自动 failover
- 健康检查正确标记不可用后端

#### Phase 3：协议转换与可观测性

**任务**：

- 集成 `llm-gateway-protocols` crate
- ProtocolMiddleware（协议检测与转换）
- MetricsMiddleware + Prometheus 端点
- TracingMiddleware + OpenTelemetry
- `/v1/models` 端点

**验收标准**：

- OpenAI ↔ Anthropic 协议转换正常
- SSE 流式转换正常
- `/metrics` 端点暴露指标
- 分布式追踪正常工作

#### Phase 4：管理 API 与测试

**任务**：

- Admin API 迁移（/v1/stats/*）
- 集成测试
- 压力测试
- 文档更新

**验收标准**：

- 所有 Admin API 功能对齐
- 单元测试覆盖率 > 80%
- 压力测试性能达标（HTTP/2 吞吐提升）

### 5.4 验证清单

| 验证项           | 方法                                        | 预期结果         |
| ---------------- | ------------------------------------------- | ---------------- |
| 基础转发         | 发送 OpenAI 请求                            | 后端正常响应     |
| 协议转换         | Anthropic → OpenAI                          | 响应格式正确     |
| SSE 流式         | stream: true 请求                           | 流式输出正常     |
| 并发限制         | 并发 10 请求（limit=8）                     | 2 个返回 503     |
| 并发释放         | 检查流结束后并发计数                        | 正确释放         |
| Failover         | 停止第一个后端                              | 自动切换到第二个 |
| 健康检查         | 模拟后端故障                                | 自动标记不健康   |
| 策略链组合       | 配置多种策略组合                            | 按顺序执行       |
| 自定义插件       | 实现 TimeBasedRouting 插件                  | 正常工作         |
| Prometheus       | 访问 /metrics                               | 指标正常暴露     |
| 优雅关闭         | kubectl delete pod                          | 无请求中断       |

### 5.5 风险与缓解

| 风险                     | 影响       | 缓解措施                               |
| ------------------------ | ---------- | -------------------------------------- |
| Pingora 流式 Body 处理   | 功能异常   | PoC 阶段重点验证                       |
| 策略插件性能             | 延迟增加   | 早期基准测试，必要时使用无锁结构       |
| 健康检查精度             | 误判故障   | 可调参数，支持快速失败和慢启动         |

## 附录

### A. 依赖版本

```toml
[dependencies]
pingora-core = "0.4"
pingora-proxy = "0.4"
pingora-server = "0.4"

prometheus = "0.13"
opentelemetry = "0.27"
serde = { version = "1.0", features = ["derive"] }
tokio = { version = "1.50", features = ["rt-multi-thread", "sync"] }
dashmap = "6.0"
```

### B. 与现有架构对比

| 现有组件             | 新架构对应               | 说明                     |
| -------------------- | ------------------------ | ------------------------ |
| InputNode            | Router Service           | 简化，专注路由分发       |
| ConcurrencyNode      | ConcurrencyLimit 插件    | 可与其他策略组合         |
| SequenceNode         | FailoverSequence 插件    | 更灵活的重试策略         |
| BackendNode          | UpstreamPeer             | Pingora 原生支持         |
| HealthMonitor        | HealthAware 插件         | 集成到策略链中           |

## 文档状态

草稿待评审

**下一步**: 评审通过后开始 PoC 验证
