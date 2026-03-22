# 技术架构设计

## 1. 整体架构

### 1.1 设计哲学

RustLLM Gateway 采用**分层路由图架构**，核心设计理念：

1. **Input Node = HTTP Server** - 每个输入节点是一个独立的 HTTP 服务，监听独立端口
2. **协议动态识别** - Input Node 不预设协议，根据请求 path/body 动态识别
3. **RoutingPayload = 已解析 HTTP 包 + 节点追溯** - 携带经过的节点表，支持回溯和调试
4. **健康感知内置** - 所有节点内置健康检查，DFS 路由 + 回溯机制
5. **图结构纯粹** - 节点直接引用 `Arc<Node>`，无需 Edge 类型
6. **策略封装权重** - 权重等信息在路由策略内部，不在图结构中

### 1.2 架构概览

```text
┌─────────────────────────────────────────────────────────────────────────────┐
│                              RustLLM Gateway                                │
│                                                                             │
│  ┌───────────────────────────────────────────────────────────────────────┐  │
│  │                         Input Node Layer                              │  │
│  │                       (HTTP Server, hyper)                            │  │
│  │                                                                       │  │
│  │   ┌─────────────────┐       ┌─────────────────┐                       │  │
│  │   │  Input Node #1  │       │  Input Node #2  │                       │  │
│  │   │  Port 8000      │       │  Port 8001      │                       │  │
│  │   │  [动态协议识别] │       │  [动态协议识别] │                       │  │
│  │   │  [构造 Payload] │       │  [构造 Payload] │                       │  │
│  │   └────────┬────────┘       └────────┬────────┘                       │  │
│  └────────────┼─────────────────────────┼────────────────────────────────┘  │
│               │                         │                                   │
│               └────────────┬────────────┘                                   │
│                            │                                                │
│                            ▼                                                │
│  ┌───────────────────────────────────────────────────────────────────────┐  │
│  │                      RoutingPayload                                   │  │
│  │  ┌─────────────────────────────────────────────────────────────────┐  │  │
│  │  │  path: String              // 用于协议识别                      │  │  │
│  │  │  headers: HeaderMap        // HTTP 头透传                       │  │  │
│  │  │  content: serde_json::Value // 解析后的 JSON 体                 │  │  │
│  │  │  visited_nodes: Vec<Arc<Node>> // 经过的节点表 (追溯/回溯)      │  │  │
│  │  │  metrics: RequestMetrics   // 统计上下文                        │  │  │
│  │  └─────────────────────────────────────────────────────────────────┘  │  │
│  └───────────────────────────────────────────────────────────────────────┘  │
│                           │                                                 │
│                           ▼                                                 │
│  ┌───────────────────────────────────────────────────────────────────────┐  │
│  │                       Virtual Node Layer                              │  │
│  │                    (路由策略执行层)                                   │  │
│  │                                                                       │  │
│  │    ┌───────────────┐         ┌───────────────┐                        │  │
│  │    │ Virtual Node  │  ────→  │ Virtual Node  │  ──→ 多级路由          │  │
│  │    │  [策略+ 健康] │         │  [策略+ 健康] │      负载均衡          │  │
│  │    │  [DFS+ 回溯]  │         │  [DFS+ 回溯]  │      聚合指标          │  │
│  │    │  targets:     │         │  targets:     │                        │  │
│  │    │  Vec<Arc<>>   │         │  Vec<Arc<>>   │                        │  │
│  │    └───────────────┘         └───────────────┘                        │  │
│  └───────────────────────────────────────────────────────────────────────┘  │
│                           │                                                 │
│                           ▼                                                 │
│  ┌───────────────────────────────────────────────────────────────────────┐  │
│  │                      Output Node Layer                                │  │
│  │                    (Backend Adapter)                                  │  │
│  │                                                                       │  │
│  │   ┌──────────────┐  ┌──────────────┐  ┌──────────────┐                │  │
│  │   │   OpenAI     │  │  Anthropic   │  │    Ollama    │                │  │
│  │   │   Backend    │  │   Backend    │  │   Backend    │                │  │
│  │   │  [健康检查]  │  │  [健康检查]  │  │  [健康检查]  │                │  │
│  │   │   [转换]     │  │   [转换]     │  │   [转换]     │                │  │
│  │   └──────────────┘  └──────────────┘  └──────────────┘                │  │
│  └───────────────────────────────────────────────────────────────────────┘  │
│                                                                             │
│  ┌───────────────────────────────────────────────────────────────────────┐  │
│  │                    Cross-Cutting Services                             │  │
│  │                                                                       │  │
│  │  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐                 │  │
│  │  │   Metrics    │  │    Config    │  │  Logging     │                 │  │
│  │  │  Collector   │  │   Manager    │  │  (tracing)   │                 │  │
│  │  │  [统计收集]  │  │  [TOML 解析] │  │ [结构化日志] │                 │  │
│  │  │  [Prometheus]│  │  [热加载]    │  │              │                 │  │
│  │  └──────────────┘  └──────────────┘  └──────────────┘                 │  │
│  └───────────────────────────────────────────────────────────────────────┘  │
│                                                                             │
│  ┌───────────────────────────────────────────────────────────────────────┐  │
│  │                    Health Check System (内置)                         │  │
│  │                                                                       │  │
│  │  ┌─────────────────────────────────────────────────────────────────┐  │  │
│  │  │  HealthRegistry                                                 │  │  │
│  │  │  - 所有节点共享的健康状态注册表                                 │  │  │
│  │  │  - 定期健康检查 (可配置间隔)                                    │  │  │
│  │  │  - DFS 路由时自动跳过不可用节点                                 │  │  │
│  │  └─────────────────────────────────────────────────────────────────┘  │  │
│  └───────────────────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────────────────┘
```

### 1.3 请求处理流程

```text
┌────────────────────────────────────────────────────────────────────────┐
│                      请求处理时序图 (DFS + 回溯)                       │
└────────────────────────────────────────────────────────────────────────┘

客户端                 Input Node           Virtual Node         Output Node
  │                       │                     │                     │
  │─── HTTP 请求 ────────→│                     │                     │
  │                       │                     │                     │
  │                       │ [1] 协议识别        │                     │
  │                       │     path → Protocol │                     │
  │                       │     body 字段验证   │                     │
  │                       │                     │                     │
  │                       │ [2] 解析 Body       │                     │
  │                       │     bytes → Value   │                     │
  │                       │                     │                     │
  │                       │ [3] 构造 Payload    │                     │
  │                       │  visited_nodes=[]   │                     │
  │                       │────────────────────→│                     │
  │                       │                     │                     │
  │                       │                     │ [4] DFS 路由        │
  │                       │                     │     strategy.       │
  │                       │                     │     select_next()   │
  │                       │                     │                     │
  │                       │                     │ [5] 检查健康状态    │
  │                       │                     │     is_healthy()?   │
  │                       │                     │                     │
  │                       │                     │ [6] 节点不可用？    │
  │                       │                     │─────→ 回溯 ────→    │
  │                       │                     │     尝试下一个      │
  │                       │                     │                     │
  │                       │                     │ [7] 更新 visited    │
  │                       │                     │     payload.        │
  │                       │                     │     visited_nodes.  │
  │                       │                     │     push(current)   │
  │                       │                     │                     │
  │                       │                     │ [8] 转发 Payload    │
  │                       │                     │────────────────────→│
  │                       │                     │                     │
  │                       │                     │                     │ [9] 健康检查
  │                       │                     │                     │     is_healthy()?
  │                       │                     │                     │
  │                       │                     │                     │ [10] 协议转换
  │                       │                     │                     │     Value → Backend
  │                       │                     │                     │
  │                       │                     │                     │ [11] 发送后端
  │                       │                     │                     │     POST /v1/...
  │                       │                     │                     │
  │                       │                     │                     │ [12] 接收响应
  │                       │                     │                     │     更新健康状态
  │                       │                     │←────────────────────│
  │                       │                     │                     │
  │                       │ [13] 响应透传       │                     │
  │←────────────────────────────────────────────│                     │
  │                       │                     │                     │
```

### 1.4 节点类型与职责

```text
┌─────────────────────────────────────────────────────────────────────────────┐
│                           节点类型详解                                      │
└─────────────────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────────────────┐
│ Input Node (输入节点)                                                       │
├─────────────────────────────────────────────────────────────────────────────┤
│ 职责：                                                                      │
│   1. 监听 HTTP 端口（hyper server）                                         │
│   2. 接收客户端请求                                                         │
│   3. 动态协议识别（path → body 字段 → header）                              │
│   4. 解析请求体为 serde_json::Value                                         │
│   5. 构造 RoutingPayload（visited_nodes 初始为空）                          │
│   6. 绑定统计上下文                                                         │
│                                                                             │
│ 特征：                                                                      │
│   - 入度 = 0（路由图起点）                                                  │
│   - 出度 ≥ 1（可路由到多个下游节点）                                        │
│   - 每节点独立端口                                                          │
│   - 不预设协议类型，动态识别                                                │
│   - 无状态设计                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│ Virtual Node (虚节点)                                                       │
├─────────────────────────────────────────────────────────────────────────────┤
│ 职责：                                                                      │
│   1. 执行路由策略（轮询/权重/随机/延迟感知）                                │
│   2. 内置健康检查（DFS 时自动跳过不可用节点）                               │
│   3. DFS 路由 + 回溯机制（失败时尝试下一个 target）                         │
│   4. 更新 visited_nodes（将当前节点加入）                                   │
│   5. 聚合流经该节点的所有流量统计                                           │
│                                                                             │
│ 特征：                                                                      │
│   - 入度 ≥ 1（可接收多个上游节点）                                          │
│   - 出度 ≥ 1（targets: Vec<Arc<Node>>）                                     │
│   - 可配置多级形成路由链                                                    │
│   - id 为主键，name 为次级索引（OnceCell 异步分发）                         │
│   - 权重等信息在策略内部，不在节点中                                        │
└─────────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│ Output Node (输出节点)                                                      │
├─────────────────────────────────────────────────────────────────────────────┤
│ 职责：                                                                      │
│   1. 协议转换（RoutingPayload.content → 后端协议格式）                      │
│   2. 发送请求到后端 LLM 服务                                                │
│   3. 接收后端响应                                                           │
│   4. 内置健康检查（定期探测后端状态）                                       │
│   5. 更新统计信息                                                           │
│   6. 响应透传回客户端                                                       │
│                                                                             │
│ 特征：                                                                      │
│   - 入度 ≥ 1（可接收多个上游节点）                                          │
│   - 出度 = 0（路由图终点）                                                  │
│   - 每节点对应一个后端服务                                                  │
│   - 包含 Backend Adapter 实现                                               │
│   - 健康状态共享到 HealthRegistry                                           │
└─────────────────────────────────────────────────────────────────────────────┘
```

## 2. 核心数据结构

### 2.1 数据组织型 Struct（POD 类型）

#### RoutingPayload（路由负载）

```rust
/// 路由负载 - 在图结构中流动的请求上下文
///
/// 关键设计：
/// - visited_nodes 携带经过的节点表，支持路由追溯和 DFS 回溯
/// - 不使用 hop_count，直接通过 visited_nodes.len() 获取跳数
pub struct RoutingPayload {
    // HTTP 层信息（已解析的 HTTP 包）
    pub path: String,              // 用于协议识别
    pub headers: HeaderMap,        // HTTP 头透传
    pub content: Value,            // 解析后的 JSON 体

    // 路由上下文
    pub visited_nodes: Vec<Arc<Node>>,  // 经过的节点表（追溯/回溯）

    // 统计上下文（随路由流动）
    pub metrics: RequestMetrics,
}

impl RoutingPayload {
    /// 创建新的 RoutingPayload
    pub fn new(path: String, headers: HeaderMap, content: Value) -> Self {
        Self {
            path,
            headers,
            content,
            visited_nodes: Vec::new(),
            metrics: RequestMetrics::new(),
        }
    }

    /// 获取模型名称（从 content 中提取）
    pub fn model(&self) -> Option<&str> {
        self.content.get("model")?.as_str()
    }

    /// 检查是否流式请求
    pub fn is_streaming(&self) -> bool {
        self.content
            .get("stream")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    /// 获取当前跳数
    pub fn hop_count(&self) -> usize {
        self.visited_nodes.len()
    }

    /// 检查是否已访问过某节点（防环）
    pub fn has_visited(&self, node_id: &NodeId) -> bool {
        self.visited_nodes.iter().any(|n| n.id() == node_id)
    }

    /// 添加节点到访问表（路由时调用）
    pub fn visit_node(&mut self, node: Arc<Node>) {
        self.visited_nodes.push(node);
    }

    /// 回溯（移除最后一个节点）
    pub fn backtrack(&mut self) -> Option<Arc<Node>> {
        self.visited_nodes.pop()
    }
}
```

#### NodeMetrics（节点指标）

```rust
/// 节点指标 - 统计数据结构
pub struct NodeMetrics {
    pub node_id: NodeId,
    pub request_count: AtomicU64,
    pub success_count: AtomicU64,
    pub error_count: AtomicU64,
    pub prompt_tokens: AtomicU64,
    pub completion_tokens: AtomicU64,
    pub latency_histogram: RwLock<Histogram>,
}
```

### 2.2 功能实现型 Struct

#### Node（节点统一表示）

```rust
/// Node - 统一节点表示
pub enum Node {
    Input(InputNode),
    Virtual(VirtualNode),
    Output(OutputNode),
}

impl Node {
    /// 获取节点 ID
    pub fn id(&self) -> &NodeId {
        match self {
            Node::Input(n) => n.id(),
            Node::Virtual(n) => n.id(),
            Node::Output(n) => n.id(),
        }
    }

    /// 获取节点名称（仅 Virtual Node 有名称）
    pub fn name(&self) -> Option<&str> {
        match self {
            Node::Virtual(n) => Some(n.name()),
            _ => None,
        }
    }

    /// 检查是否健康
    pub async fn is_healthy(&self) -> bool {
        match self {
            Node::Input(n) => n.is_healthy().await,
            Node::Virtual(n) => n.is_healthy().await,
            Node::Output(n) => n.is_healthy().await,
        }
    }
}
```

#### InputNode（输入节点）

```rust
/// Input Node - HTTP 服务器
///
/// 关键设计：
/// - 不预设 protocol 字段，协议动态识别
pub struct InputNode {
    id: NodeId,
    port: u16,
    graph: Arc<RoutingGraph>,
    metrics: Arc<MetricsCollector>,
}

impl InputNode {
    pub fn new(
        id: NodeId,
        port: u16,
        graph: Arc<RoutingGraph>,
        metrics: Arc<MetricsCollector>,
    ) -> Self {
        Self { id, port, graph, metrics }
    }

    pub fn id(&self) -> &NodeId {
        &self.id
    }

    /// 运行 HTTP 服务器
    pub async fn run(self) -> Result<(), GatewayError> {
        let addr = format!("0.0.0.0:{}", self.port);
        let listener = TcpListener::bind(&addr).await?;

        tracing::info!("Input Node {} listening on {}", self.id, addr);

        loop {
            let (stream, _) = listener.accept().await?;
            let io = TokioIo::new(stream);

            let node = self.clone();
            tokio::task::spawn(async move {
                let service = service_fn(|req| node.handle_request(req));
                if let Err(e) = http1::Builder::new()
                    .serve_connection(io, service)
                    .await
                {
                    tracing::warn!("Connection error: {}", e);
                }
            });
        }
    }

    async fn handle_request(
        &self,
        req: Request<hyper::body::Incoming>,
    ) -> Result<Response<String>, GatewayError> {
        // 1. 协议识别（通过 path，不依赖预设 protocol）
        let protocol = detect_protocol_from_path(req.uri().path())
            .or_else(|| detect_protocol_from_body_hint(req.headers()))
            .ok_or(GatewayError::UnknownProtocol)?;

        // 2. 解析 body 为 serde_json::Value
        let body = hyper::body::to_bytes(req.into_body()).await?;
        let content: Value = serde_json::from_slice(&body)?;

        // 3. 构造 RoutingPayload（visited_nodes 初始为空）
        let payload = RoutingPayload::new(
            req.uri().path().to_string(),
            req.headers().clone(),
            content,
        );

        // 4. 启动路由
        let response = self.graph.route(payload).await?;

        // 5. 返回响应
        Ok(Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "application/json")
            .body(serde_json::to_string(&response)?)?)
    }

    pub async fn is_healthy(&self) -> bool {
        true
    }
}
```

#### VirtualNode（虚节点）

```rust
/// Virtual Node - 路由策略执行节点
///
/// 关键设计：
/// - id 为主键，name 为次级索引
/// - targets 直接存储 Arc<Node>，无需 Edge 类型
/// - 权重等信息在策略内部维护
/// - DFS 路由 + 回溯机制
pub struct VirtualNode {
    id: NodeId,
    name: String,
    strategy: Box<dyn RoutingStrategy>,
    targets: Vec<Arc<Node>>,
    metrics: NodeMetrics,
}

impl VirtualNode {
    pub fn new(
        id: NodeId,
        name: String,
        strategy: Box<dyn RoutingStrategy>,
        targets: Vec<Arc<Node>>,
    ) -> Self {
        Self {
            id,
            name,
            strategy,
            targets,
            metrics: NodeMetrics::new(id.clone()),
        }
    }

    pub fn id(&self) -> &NodeId {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// 执行 DFS 路由（带回溯）
    pub async fn route(
        &self,
        mut payload: RoutingPayload,
        graph: &RoutingGraph,
    ) -> Result<RoutingResult, RoutingError> {
        // 检查是否已访问过（防环）
        if payload.has_visited(&self.id) {
            return Err(RoutingError::CycleDetected);
        }

        // 将当前节点加入访问表
        let self_arc = graph.get_node(&self.id).unwrap();
        payload.visit_node(self_arc.clone());

        // DFS 路由：遍历所有 target，直到成功
        for target in &self.targets {
            // 健康检查：跳过不可用节点
            if !target.is_healthy().await {
                tracing::debug!("Skipping unhealthy node: {}", target.id());
                continue;
            }

            // 递归路由到目标节点
            match target.route(payload, graph).await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    // 路由失败，回溯并尝试下一个 target
                    tracing::debug!("Route failed, trying next: {}", e);
                    payload.backtrack();
                    continue;
                }
            }
        }

        // 所有 target 都失败
        Err(RoutingError::NoAvailableBackend)
    }

    pub async fn is_healthy(&self) -> bool {
        true
    }
}
```

#### OutputNode（输出节点）

```rust
/// Output Node - 后端适配器
///
/// 关键设计：
/// - 内置健康检查，定期探测后端状态
/// - 健康状态注册到 HealthRegistry
pub struct OutputNode {
    id: NodeId,
    backend: Box<dyn BackendClient>,
    metrics: NodeMetrics,
    health_status: Arc<AtomicBool>,
}

impl OutputNode {
    pub fn new(
        id: NodeId,
        backend: Box<dyn BackendClient>,
    ) -> Self {
        Self {
            id,
            backend,
            metrics: NodeMetrics::new(id.clone()),
            health_status: Arc::new(AtomicBool::new(true)),
        }
    }

    pub fn id(&self) -> &NodeId {
        &self.id
    }

    /// 执行请求（终点节点）
    pub async fn route(
        &self,
        payload: &RoutingPayload,
    ) -> Result<BackendResponse, BackendError> {
        let start = Instant::now();

        let response = self.backend
            .send_request(payload, Duration::from_secs(30))
            .await;

        // 更新统计
        let latency = start.elapsed();
        self.metrics.record_request();
        self.metrics.record_latency(latency);

        // 更新健康状态
        match &response {
            Ok(_) => self.health_status.store(true, Ordering::Relaxed),
            Err(e) if e.is_critical() => self.health_status.store(false, Ordering::Relaxed),
            _ => {}
        }

        response
    }

    pub async fn is_healthy(&self) -> bool {
        self.health_status.load(Ordering::Relaxed)
    }

    pub fn set_healthy(&self, healthy: bool) {
        self.health_status.store(healthy, Ordering::Relaxed);
    }
}
```

### 2.3 健康检查系统

```rust
/// 健康状态注册表 - 所有节点共享
pub struct HealthRegistry {
    nodes: DashMap<NodeId, Arc<AtomicBool>>,
}

impl HealthRegistry {
    pub fn new() -> Self {
        Self { nodes: DashMap::new() }
    }

    pub fn register(&self, node_id: NodeId, status: Arc<AtomicBool>) {
        self.nodes.insert(node_id, status);
    }

    pub fn is_healthy(&self, node_id: &NodeId) -> bool {
        self.nodes
            .get(node_id)
            .map(|s| s.load(Ordering::Relaxed))
            .unwrap_or(false)
    }

    pub fn unhealthy_nodes(&self) -> Vec<NodeId> {
        self.nodes
            .iter()
            .filter(|e| !e.value().load(Ordering::Relaxed))
            .map(|e| e.key().clone())
            .collect()
    }
}
```

### 2.4 路由策略

```rust
/// 路由策略 trait
///
/// 关键设计：
/// - 健康检查已内置到节点中，策略只需关注选择逻辑
/// - 权重等信息在策略内部维护，不在图结构中
pub trait RoutingStrategy: Send + Sync {
    fn select_next(
        &self,
        payload: &RoutingPayload,
        candidates: &[NodeRef],
    ) -> Result<NodeRef, RoutingError>;
}

/// 轮询策略
pub struct RoundRobinStrategy {
    counter: AtomicUsize,
}

impl RoutingStrategy for RoundRobinStrategy {
    fn select_next(
        &self,
        _payload: &RoutingPayload,
        candidates: &[NodeRef],
    ) -> Result<NodeRef, RoutingError> {
        let idx = self.counter.fetch_add(1, Ordering::Relaxed) % candidates.len();
        Ok(candidates[idx].clone())
    }
}

/// 权重策略 - 权重信息在策略内部维护
pub struct WeightedStrategy {
    weights: Vec<u32>,  // 与 targets 索引对应
}

impl WeightedStrategy {
    pub fn new(weights: Vec<u32>) -> Self {
        Self { weights }
    }
}

impl RoutingStrategy for WeightedStrategy {
    fn select_next(
        &self,
        _payload: &RoutingPayload,
        candidates: &[NodeRef],
    ) -> Result<NodeRef, RoutingError> {
        // 权重选择逻辑（轮盘赌算法）
        let total_weight: u32 = self.weights.iter().sum();
        let mut rng = rand::thread_rng();
        let mut random = rng.gen_range(0..total_weight);

        for (i, &weight) in self.weights.iter().enumerate() {
            if i >= candidates.len() {
                break;
            }
            if random < weight {
                return Ok(candidates[i].clone());
            }
            random -= weight;
        }

        Ok(candidates[0].clone())
    }
}
```

## 3. 协议识别

### 3.1 协议识别优先级

```text
HTTP 请求
    │
    ▼
┌─────────────────────────────────────────┐
│ 1. Path 识别（主要方式，最高优先级）    │
│    /v1/chat/completions → OpenAI        │
│    /v1/messages → Anthropic             │
│    /api/generate → Ollama               │
└─────────────────────────────────────────┘
    │
    ▼ (Path 无法识别时)
┌─────────────────────────────────────────┐
│ 2. Body 字段识别（次要方式）            │
│    content 包含 "messages" 数组         │
│    content 包含 "model" 字段            │
│    content 包含 "prompt" 字段           │
└─────────────────────────────────────────┘
    │
    ▼ (仍无法识别时)
┌─────────────────────────────────────────┐
│ 3. Header 识别（备选方式）              │
│    X-Protocol: openai                   │
│    X-Protocol: anthropic                │
└─────────────────────────────────────────┘
    │
    ▼ (仍无法识别时)
┌─────────────────────────────────────────┐
│ 4. 返回 UnknownProtocol 错误            │
└─────────────────────────────────────────┘
```

### 3.2 协议识别实现

```rust
/// 从 path 识别协议（主要方式）
pub fn detect_protocol_from_path(path: &str) -> Option<Protocol> {
    match path {
        "/v1/chat/completions" => Some(Protocol::OpenAI),
        "/v1/messages" => Some(Protocol::Anthropic),
        "/api/generate" | "/api/chat" => Some(Protocol::Ollama),
        _ => None,
    }
}

/// 从 body 字段提示识别协议（次要方式）
pub fn detect_protocol_from_body_hint(content: &Value) -> Option<Protocol> {
    // 检查是否有 "messages" 数组（OpenAI 或 Anthropic）
    if let Some(messages) = content.get("messages").and_then(|v| v.as_array()) {
        if messages.iter().any(|m| {
            m.get("role").and_then(|v| v.as_str()) == Some("user")
            && m.get("content").is_some()
        }) {
            // Anthropic 特有字段
            if content.get("system").is_some()
                || content.get("max_tokens").is_some()
            {
                return Some(Protocol::Anthropic);
            }
            return Some(Protocol::OpenAI);
        }
    }

    // 检查是否有 "prompt" 字段（Ollama）
    if content.get("prompt").is_some() {
        return Some(Protocol::Ollama);
    }

    // 检查 model 字段
    if let Some(model) = content.get("model").and_then(|v| v.as_str()) {
        if model.starts_with("claude-") {
            return Some(Protocol::Anthropic);
        }
        if model.starts_with("llama") || model.contains("ollama") {
            return Some(Protocol::Ollama);
        }
    }

    None
}
```

## 4. 路由图设计

### 4.1 路由图实现

```rust
/// 路由图 - 管理所有节点和连接
///
/// 关键设计：
/// - 节点直接存储 Arc<Node> 引用，无需 Edge 类型
/// - 权重等信息在策略内部，不在图结构中
pub struct RoutingGraph {
    nodes: DashMap<NodeId, Arc<Node>>,
    input_nodes: Vec<NodeId>,
    output_nodes: Vec<NodeId>,
    health_registry: Arc<HealthRegistry>,
    name_index: OnceCell<DashMap<String, NodeId>>,
}

impl RoutingGraph {
    pub fn new() -> Self {
        Self {
            nodes: DashMap::new(),
            input_nodes: Vec::new(),
            output_nodes: Vec::new(),
            health_registry: Arc::new(HealthRegistry::new()),
            name_index: OnceCell::new(),
        }
    }

    /// 添加节点
    pub fn add_node(&self, node: Node) {
        let id = node.id().clone();
        let arc = Arc::new(node);

        // 注册健康状态（仅 Output Node）
        if let Node::Output(n) = arc.as_ref() {
            self.health_registry.register(id.clone(), n.health_status.clone());
        }

        self.nodes.insert(id.clone(), arc);
    }

    /// 获取节点
    pub fn get_node(&self, node_id: &NodeId) -> Option<Arc<Node>> {
        self.nodes.get(node_id).map(|e| e.value().clone())
    }

    /// 通过名称获取节点（异步分发）
    pub async fn get_node_by_name(&self, name: &str) -> Option<Arc<Node>> {
        let index = self.name_index.get_or_init(|| {
            let index = DashMap::new();
            for entry in self.nodes.iter() {
                if let Some(node_name) = entry.value().name() {
                    index.insert(node_name.to_string(), entry.key().clone());
                }
            }
            index
        });

        index.get(name).and_then(|id| self.get_node(&id))
    }

    /// 验证图结构
    pub fn validate(&self) -> Result<(), GraphError> {
        self.check_cycles()?;
        self.check_connectivity()?;
        self.check_input_output_nodes()?;
        Ok(())
    }
}
```

---

## 5. 配置设计

### 5.1 配置结构

```toml
# Input Nodes - 不预设协议，动态识别
[[input_nodes]]
id = "input-main"
port = 8000

[[input_nodes]]
id = "input-alt"
port = 8001

# Virtual Nodes - 带 name 字段，targets 直接列出节点 ID
[[virtual_nodes]]
id = "router-001"
name = "primary-router"
strategy = "weighted"

# 权重在策略配置中，与 targets 顺序对应
[virtual_nodes.strategy_config]
weights = [70, 30]

# targets 直接列出目标节点 ID（无需 Edge 类型）
targets = ["output-openai", "output-anthropic"]

[[virtual_nodes]]
id = "router-fallback"
name = "fallback-router"
strategy = "first_available"
targets = ["output-backup-1", "output-backup-2"]

# Output Nodes - 内置健康检查
[[output_nodes]]
id = "output-openai"
provider = "openai"
endpoint = "https://api.openai.com/v1"
api_key = "${OPENAI_API_KEY}"

# 健康检查配置（可选）
[output_nodes.health_check]
enabled = true
interval_secs = 30
timeout_secs = 5
```

## 6. 设计决策总结

| 设计点            | 设计方案                        | 理由                            |
|:------------------|:--------------------------------|:--------------------------------|
| Input Node 协议   | 动态识别，不预设                | 更灵活，同端口支持多协议        |
| 路由追溯          | `visited_nodes: Vec<Arc<Node>>` | 支持回溯调试，信息完整          |
| Virtual Node 索引 | `id` 主键 + `name` 次级索引     | 便于调试和搜索                  |
| 健康感知          | 所有节点内置，DFS+ 回溯         | 避免无效路由，算法保证          |
| 协议识别次要方式  | Body 字段提示                   | Content-Type 都是 JSON 无法区分 |
| 图结构连接        | `Vec<Arc<Node>>`                | 简化设计，权重在策略内部        |
| 故障转移          | DFS 回溯自动处理                | 无需配置，算法保证              |

## 7. 目录结构

```text
rustllm-gateway/
├── Cargo.toml              # Workspace 根
├── crates/
│   ├── gateway-core/       # 核心路由逻辑
│   │   ├── node.rs         # 节点定义 (Input/Virtual/Output)
│   │   ├── graph.rs        # 路由图 (无 Edge 类型)
│   │   ├── routing.rs      # 路由逻辑 (DFS+ 回溯)
│   │   └── strategy.rs     # 路由策略 (权重/轮询等)
│   ├── gateway-protocol/   # 协议定义
│   ├── gateway-adapters/   # 后端适配器
│   ├── gateway-metrics/    # 指标收集
│   ├── gateway-config/     # 配置管理
│   └── gateway-cli/        # CLI 工具
└── src/                    # 主程序（二进制）
    ├── main.rs
    └── input_node.rs       # Input Node = HTTP Server
```
