# 产品设计文档

## 1. 产品愿景与定位

### 1.1 产品名称

**RustLLM Gateway** - 高性能 LLM 流量代理网关

### 1.2 目标用户

| 用户类型       | 使用场景             | 核心需求               |
|:---------------|:---------------------|:-----------------------|
| AI 应用开发者  | 多模型切换、成本控制 | 统一 API、自动故障转移 |
| 中小企业       | 内网 LLM 服务代理    | 简单部署、基础限流统计 |
| 平台团队       | 多租户 LLM 服务管理  | API Key 认证、配额管理 |
| 边缘部署场景   | 资源受限环境         | 低资源占用、快速启动   |

### 1.3 差异化定位

| 维度         | LiteLLM (Python)      | Bifrost (Go)          | Helicone (Rust)    | RustLLM Gateway (Rust) |
|:-------------|:----------------------|:----------------------|:-------------------|:-----------------------|
| 性能         | 中等（~440μs 开销）   | 高（~11μs 开销）      | 高（Rust 实现）    | **目标 <10μs**         |
| 部署         | Docker/Python 环境    | 单二进制              | 云托管/自托管      | **静态链接单二进制**   |
| 协议支持     | 最全                  | 中等                  | 中等               | **分层策略**           |
| 路由模型     | 简单路由              | 自适应负载均衡        | 健康感知路由       | **虚节点图结构**       |
| 扩展性       | Python 插件           | Go 模块               | 插件系统           | **Rust trait 插件**    |
| 学习曲线     | 低                    | 中                    | 中                 | 中上                   |

### 1.4 核心价值主张

1. **Rust 性能优势** - 零成本抽象，内存安全，无 GC 停顿
2. **极简部署** - 单静态二进制，无运行时依赖
3. **渐进式复杂度** - 从单节点简单配置起步，平滑扩展到分布式
4. **开发者体验** - 类型安全，编译期错误检查，完善测试
5. **图结构路由** - 虚节点网表，灵活编排路由策略

## 2. 功能需求规格

### 2.1 核心功能（Phase 1 MVP）

#### 2.1.1 协议转换（分层策略）

**Tier 1 协议（Phase 1 完全支持）：**

| 方向 | 协议                   | 端点                   |
|:-----|:-----------------------|:-----------------------|
| 输入 | OpenAI Chat Completion | `/v1/chat/completions` |
| 输入 | Anthropic Messages     | `/v1/messages`         |
| 输出 | OpenAI 兼容            | OpenAI API, vLLM, TGI  |
| 输出 | Anthropic Messages     | Anthropic API          |

**Tier 2 协议（Phase 2 支持）：**

- Ollama API
- Cohere API
- Google Vertex AI

**Tier 3 协议（Phase 3+ 支持）：**

- AWS Bedrock
- Azure OpenAI
- 自定义协议插件

**流式支持：**

- 所有 Tier 1 协议支持 SSE 流式响应（`plaintext/event-stream`）
- 流式数据透传，不做流式/非流式转换

#### 2.1.2 虚节点图结构路由

**核心概念：**

```plaintext
┌─────────────────────────────────────────────────────────────────┐
│                        Gateway 路由网表                         │
│                                                                 │
│   ┌─────────────┐                                               │
│   │  Input Node │  ← 入度=0，接收客户端请求                     │
│   │  (API Key)  │     根据模型/协议/API Key 构造路由负载        │
│   └──────┬──────┘                                               │
│          │                                                      │
│          ▼                                                      │
│   ┌─────────────┐     ┌─────────────┐                           │
│   │ Virtual     │────▶│ Virtual     │  ← 中间节点，执行策略     │
│   │ Node A      │     │ Node B      │     负载均衡/限流/统计    │
│   └──────┬──────┘     └──────┬──────┘                           │
│          │                   │                                  │
│          └─────────┬─────────┘                                  │
│                    │                                            │
│                    ▼                                            │
│              ┌─────────────┐                                    │
│              │ Output Node │  ← 出度=0，协议转换并发送后端      │
│              │ (Backend)   │     返回响应透传回客户端           │
│              └─────────────┘                                    │
└─────────────────────────────────────────────────────────────────┘
```

**节点类型：**

| 节点类型     | 入度 | 出度 | 职责                             |
|:-------------|:-----|:-----|:---------------------------------|
| Input Node   | 0    | ≥1   | 接收请求，构造路由负载，绑定统计 |
| Virtual Node | ≥1   | ≥1   | 执行路由策略，流量聚合统计       |
| Output Node  | ≥1   | 0    | 协议转换，发送后端，绑定统计     |

**路由负载（Routing Payload）：**

```rust
struct RoutingPayload {
    // 客户端请求信息
    client_protocol: Protocol,
    api_key: String,
    model: String,

    // 请求内容
    messages: Vec<Message>,
    stream: bool,

    // 路由上下文
    hop_count: u32,              // 已经过的虚节点数
    visited_nodes: Vec<NodeId>,  // 已访问节点（防环）

    // 统计信息（随路由流动）
    metrics: RequestMetrics,
}
```

**Phase 1 路由策略（每节点可独立配置）：**

- `round_robin` - 轮询
- `random` - 随机
- `first_available` - 第一个可用（简单故障转移）
- `weighted` - 权重路由

#### 2.1.3 流量统计

**统计绑定位置：**

- **Input Node** - 每 API Key / 每客户端协议统计
- **Output Node** - 每后端统计
- **Virtual Node** - 聚合流经该节点的所有流量统计

**采集指标：**

```rust
struct NodeMetrics {
    // 计数指标
    request_count: u64,
    success_count: u64,
    error_count: u64,

    // 延迟指标
    latency_p50: Duration,
    latency_p95: Duration,
    latency_p99: Duration,

    // Token 指标
    prompt_tokens: u64,
    completion_tokens: u64,

    // 流式指标（Phase 2）
    stream_chunks: u64,
    stream_completions: u64,
}
```

**统计流动机制：**

- 请求从 Input Node 流向 Output Node 时，携带统计上下文
- 每个节点更新自己的统计计数器
- Virtual Node 聚合所有输入边的统计

**暴露方式：**

- Prometheus 格式 `/metrics` 端点
- CLI 实时查看（按节点过滤）

#### 2.1.4 基础认证

- API Key 验证（Header: `Authorization: Bearer <key>`）
- 每 Input Node 可配置独立的 API Key 集合
- 配置文件静态定义

### 2.2 辅助功能（Phase 1-2）

#### 2.2.1 配置管理（基于虚节点）

**配置文件格式：** TOML

**配置结构示例：**

```toml
# ============ Input Nodes ============
[[input_nodes]]
id = "input-openai"
protocol = "openai"
endpoint = "/v1/chat/completions"

[[input_nodes.api_keys]]
key = "${API_KEY_1}"
name = "team-a"
rate_limit = 1000  # requests per minute

[[input_nodes.api_keys]]
key = "${API_KEY_2}"
name = "team-b"
rate_limit = 500

# ============ Virtual Nodes ============
[[virtual_nodes]]
id = "router-primary"
strategy = "weighted"

[[virtual_nodes.edges]]
target = "output-openai"
weight = 80

[[virtual_nodes.edges]]
target = "output-anthropic"
weight = 20

[[virtual_nodes]]
id = "router-fallback"
strategy = "first_available"

[[virtual_nodes.edges]]
target = "output-openai-backup"

# ============ Output Nodes ============
[[output_nodes]]
id = "output-openai"
provider = "openai"
endpoint = "https://api.openai.com/v1"
api_key = "${OPENAI_API_KEY}"
protocol = "openai"

[[output_nodes]]
id = "output-anthropic"
provider = "anthropic"
endpoint = "https://api.anthropic.com/v1"
api_key = "${ANTHROPIC_API_KEY}"
protocol = "anthropic"

[[output_nodes]]
id = "output-openai-backup"
provider = "openai"
endpoint = "https://api.openai.com/v1"
api_key = "${OPENAI_API_KEY_2}"
protocol = "openai"

# ============ Connections ============
# Input -> Virtual
[[connections]]
from = "input-openai"
to = "router-primary"

# Virtual -> Virtual
[[connections]]
from = "router-primary"
to = "router-fallback"
condition = "on_error"  # 仅在错误时路由

# Virtual -> Output
[[connections]]
from = "router-primary"
to = "output-openai"

[[connections]]
from = "router-primary"
to = "output-anthropic"
```

**热加载机制：**

- 文件变更检测（`notify` crate）
- 配置验证后原子替换
- CLI 触发重载命令

#### 2.2.2 CLI 管理接口（基于虚节点）

**基于 stdio 的子命令：**

```bash
# 查看全局统计
rustllm-cli stats

# 查看指定节点统计
rustllm-cli stats node input-openai
rustllm-cli stats node output-anthropic

# 查看所有节点状态
rustllm-cli nodes

# 查看节点连接关系（ASCII 图）
rustllm-cli graph

# 重载配置
rustllm-cli reload

# 动态启用/禁用节点
rustllm-cli node disable output-openai
rustllm-cli node enable output-openai

# 动态调整虚节点权重（Phase 2）
rustllm-cli node set-weight router-primary output-openai 60
```

**实现方式：** Unix Domain Socket / 共享内存 IPC

### 2.3 未来扩展功能

#### 2.3.1 负载热迁移（Phase 2）

**愿景：** 后端中断时自动重新路由，客户端无感知

**非流式热迁移：**

```plaintext
1. 请求已路由到 Output Node A
2. Output Node A 发送请求到后端
3. 后端连接中断（网络错误/服务崩溃）
4. Gateway 检测到错误，自动重新路由到 Output Node B
5. 客户端收到响应（延迟增加，但请求成功）
```

**验收标准：**

- 自动检测后端故障
- 自动重试路由（可配置重试次数）
- 客户端感知为单次延迟增加，非错误

#### 2.3.2 流式负载热迁移（Phase 3）

**愿景：** 流式输出过程中后端中断，无缝切换到备用后端

```plaintext
客户端： "Hello, [streaming...] world [后端中断] [切换后端] ! How can I help?"
                    ▲                     ▲
                后端 A 输出           后端 B 继续输出
```

**技术挑战：**

- 保持输出文本连贯性（需要多后端协调或智能拼接）
- 中断点检测与状态保存
- 新后端上下文同步

**验收标准：**

- 已输出内容不被破坏
- 新后端继续输出，保持语义连贯
- 客户端感知为短暂停顿

#### 2.3.3 Phase 3+ 功能

| 功能          | Phase | 描述                             |
|:--------------|:------|:---------------------------------|
| 高级限流      | 3     | 令牌桶、滑动窗口、分层限流       |
| HTTP 管理 API | 3     | REST API 动态配置节点和连接      |
| JWT 认证      | 3     | OAuth2/OIDC 集成                 |
| 语义缓存      | 4     | 基于嵌入的相似请求缓存           |
| 分布式追踪    | 3     | OpenTelemetry 集成，追踪 ID 透传 |
| 插件系统      | 4     | WASM 或动态库插件                |
| 多区域集群    | 4     | 跨节点状态同步，分布式图路由     |

## 3. 非功能需求

### 3.1 性能目标

| 指标         | 目标值                   | 测量条件         |
|:-------------|:-------------------------|:-----------------|
| 代理延迟开销 | <10μs (P50), <50μs (P99) | 5000 RPS, 空转发 |
| 吞吐量       | >10,000 RPS              | 单节点，8 核 CPU |
| 启动时间     | <100ms                   | 冷启动到就绪     |
| 连接数       | >10,000 并发             | 单节点           |

### 3.2 可靠性要求

| 指标         | 目标值                        |
|:-------------|:------------------------------|
| 可用性       | 99.9% (单节点)                |
| 故障恢复     | 自动故障转移 <1s              |
| 配置错误     | 编译期/启动时检测             |
| 内存安全     | 无 use-after-free, 无数据竞争 |
| 路由环路     | 编译期/配置期检测，运行期防护 |

### 3.3 可扩展性设计

**纵向扩展（Scale Up）：**

- 充分利用多核（Tokio 多线程运行时）
- 无锁数据结构优先

**横向扩展（Scale Out）预留：**

- 状态外部化（Redis 接口预留）
- 无状态设计（任何节点可处理任何请求）
- 分布式追踪 ID 透传

**图结构扩展：**

- 虚节点数量无硬编码限制
- 支持运行时动态添加/删除节点
- 支持配置热更新图结构

## 4. 用户故事

### 4.1 开发者快速开始

> 作为一个 AI 应用开发者，我希望 5 分钟内完成部署并代理到我的 LLM 后端，这样我可以快速验证我的应用。

**验收标准：**

1. 下载单二进制文件
2. 编写 20 行 TOML 配置（1 Input + 1 Output + 1 Connection）
3. 启动服务
4. 修改应用 API base URL 即可

### 4.2 多模型故障转移

> 作为一个平台工程师，我希望在主模型不可用时自动切换到备用模型，这样我的用户不会感知到服务中断。

**验收标准：**

1. 配置主备 Output Node
2. 配置 Virtual Node 使用 `first_available` 策略
3. 主后端故障时自动切换
4. 切换延迟 <1s

### 4.3 基于 API Key 的路由

> 作为一个平台负责人，我希望不同团队的 API Key 路由到不同的后端，这样我可以进行成本隔离和配额管理。

**验收标准：**

1. 配置多个 Input Node，每团队一个
2. 每 Input Node 绑定独立的 API Key 集合
3. 不同 Input Node 路由到不同的 Output Node
4. CLI 可查看每团队的独立统计

### 4.4 复杂路由编排

> 作为一个架构师，我希望构建多级路由策略，例如先按成本路由，失败后再按延迟路由。

**验收标准：**

1. 配置多级 Virtual Node
2. 第一级使用 `weighted` 策略（成本优先）
3. 第二级使用 `first_available` 策略（故障转移）
4. 请求依次经过多级路由

## 5. 约束与假设

### 5.1 技术约束

- **Rust 版本** - 最新稳定版 + edition 2024
- **目标平台** - Linux x86_64/aarch64 优先，macOS/Windows 后续
- **依赖策略** - 核心依赖最小化，避免重型框架

### 5.2 假设

- 用户有基础 Rust 知识（阅读源码/贡献时）
- 后端 LLM 服务支持标准协议（OpenAI/Anthropic 兼容）
- 初期用户接受配置文件部署（无 UI）
- 路由图结构为有向无环图（DAG），运行期检测环路

## 6. 风险与缓解

| 风险               | 影响 | 概率 | 缓解措施                                 |
|:-------------------|:-----|:-----|:-----------------------------------------|
| 协议碎片化         | 高   | 中   | Tier 分层策略，优先保证 Tier 1 质量      |
| 性能不达标         | 高   | 低   | 早期基准测试，使用 `criterion` 持续监控  |
| 生态竞争           | 中   | 中   | 聚焦 Rust 生态集成，图结构路由差异化     |
| 维护负担           | 中   | 中   | 严格测试覆盖，文档驱动开发               |
| 路由环路           | 高   | 低   | 配置期图算法检测 + 运行期 hop_count 限制 |
| 流式热迁移复杂度   | 高   | 高   | Phase 3 实现，前期充分技术验证           |

## 7. 术语表

| 术语         | 定义                                           |
|:-------------|:-----------------------------------------------|
| Input Node   | 入度为 0 的节点，接收客户端请求，构造路由负载  |
| Virtual Node | 中间节点，执行路由策略，聚合统计               |
| Output Node  | 出度为 0 的节点，协议转换并发送后端            |
| 路由负载     | 在图结构中流动的请求上下文，包含协议/模型/统计 |
| 路由网表     | 由虚节点和连接构成的有向图结构                 |
| 负载热迁移   | 后端故障时自动重新路由到备用后端               |
| 流式热迁移   | 流式输出过程中后端故障，无缝切换并保持输出连贯 |
