# 开发路线图

## 概述

本项目采用分阶段开发策略，每个 Phase 交付可用的增量功能，确保早期可用性和持续迭代。

**开发方法：** TDD 测试驱动开发 + 文档先行

```plaintext
开发流程（每个功能模块）：

1. 协议/技术调研 → docs/protocols/ 或 docs/design/
         ↓
2. 设计接口（Trait/Struct 定义）
         ↓
3. 编写测试（失败）
         ↓
4. 实现功能（测试通过）
         ↓
5. 重构优化

Phase 1 (MVP) → Phase 2 (功能完善) → Phase 3 (生产就绪) → Phase 4 (生态建设)
    │                │                    │                    │
    │                │                    │                    │
    ▼                ▼                    ▼                    ▼
 核心代理          负载均衡            限流/HTTP API        插件系统
 健康感知          CLI 管理             高级观测性           分布式支持
 DFS 路由          热迁移 (非流式)      JWT 认证             语义缓存
 基础统计          Tier 2 协议          分布式追踪
```

---

## Phase 1：MVP（最小可行产品）

**目标：** 实现核心代理功能，支持 Tier 1 协议，健康感知 + DFS 路由

**预计周期：** 1-2 周

### 1.1 核心功能

#### 1.1.1 Input Node（HTTP Server）

- [ ] 基于 hyper 的 HTTP 服务器
- [ ] 多端口支持（每个 Input Node 独立端口）
- [ ] 协议识别（path → body 字段）
- [ ] 请求体解析为 `serde_json::Value`
- [ ] 构造 RoutingPayload（`visited_nodes: Vec<Arc<Node>>`）

**验收标准：**

- 可启动多个 Input Node，监听不同端口
- 正确识别 OpenAI 和 Anthropic 协议
- 解析请求体并构造 RoutingPayload

#### 1.1.2 Output Node（Backend Adapter）

- [ ] OpenAI Backend 实现
- [ ] Anthropic Backend 实现
- [ ] 基本错误处理
- [ ] 超时控制
- [ ] 健康状态自动更新（成功→健康，严重错误→不健康）

**验收标准：**

- 可配置 OpenAI 和 Anthropic 后端
- 成功转发请求并返回响应
- 错误情况正确返回错误码
- 健康状态自动更新

#### 1.1.3 路由图与健康感知 DFS

- [ ] 节点定义（Input/Virtual/Output）
- [ ] 连接关系配置（`targets: Vec<Arc<Node>>`）
- [ ] 基础路由策略（轮询、权重）
- [ ] 健康检查内置（所有节点 `is_healthy()` 方法）
- [ ] DFS 路由 + 回溯机制（自动跳过不可用节点）
- [ ] 环路检测（配置期）

**验收标准：**

- TOML 配置定义节点和连接
- DFS 路由自动跳过不健康节点
- 失败时自动回溯尝试下一个 target
- 配置期检测环路

#### 1.1.4 配置管理

- [ ] TOML 配置解析
- [ ] 配置验证
- [ ] 文件变更检测（notify crate）
- [ ] 热加载（原子替换）

**验收标准：**

- 支持完整配置结构
- 配置错误在启动时报出
- 修改配置后自动重载（无需重启）

#### 1.1.5 基础统计

- [ ] 每节点请求计数
- [ ] 每节点成功/失败计数
- [ ] Prometheus 格式导出（`/metrics` 端点）
- [ ] CLI 查看统计

**验收标准：**

- `/metrics` 端点输出 Prometheus 格式
- CLI 可查看每节点统计
- 指标实时更新

### 1.2 技术任务（TDD 流程）

**开发流程：**

```text
对于每个功能模块：

1. 文档准备（协议调研/技术调研）
       ↓
2. 设计接口（定义 Trait/Struct）
       ↓
3. 编写测试（此时编译失败或测试失败）
       ↓
4. 实现功能（测试通过）
       ↓
5. 重构优化
```

| 模块 | 步骤 | 任务 | 预计工时 | 产出 |
|:-----|:-----|:-----|:---------|:-----|
| **协议准备** | 1 | OpenAI 协议调研 | 0.5 天 | `docs/protocols/openai.md` |
| | 1 | Anthropic 协议调研 | 0.5 天 | `docs/protocols/anthropic.md` |
| **Input Node** | 2 | 设计 HTTP Server 接口 | 0.25 天 | `src/input_node.rs` (接口定义) |
| | 3 | 编写协议识别测试 | 0.25 天 | `tests/protocol_detect_test.rs` |
| | 4 | 实现协议识别 | 0.25 天 | 测试通过 |
| | 3 | 编写请求处理测试 | 0.25 天 | `tests/request_test.rs` |
| | 4 | 实现 HTTP Server | 0.5 天 | 测试通过 |
| **Output Node** | 2 | 设计 BackendClient trait | 0.25 天 | `gateway-adapters/lib.rs` |
| | 3 | 编写 OpenAI Backend 测试 | 0.25 天 | `tests/openai_backend_test.rs` |
| | 4 | 实现 OpenAI Backend | 0.5 天 | 测试通过 |
| | 3 | 编写 Anthropic Backend 测试 | 0.25 天 | `tests/anthropic_backend_test.rs` |
| | 4 | 实现 Anthropic Backend | 0.5 天 | 测试通过 |
| **路由图** | 2 | 设计 RoutingGraph 接口 | 0.5 天 | `gateway-core/graph.rs` |
| | 2 | 设计 RoutingStrategy trait | 0.25 天 | `gateway-core/strategy.rs` |
| | 3 | 编写 DFS 路由测试 | 0.5 天 | `tests/dfs_routing_test.rs` |
| | 4 | 实现 DFS+ 回溯 | 1 天 | 测试通过 |
| | 3 | 编写健康感知测试 | 0.25 天 | `tests/health_check_test.rs` |
| | 4 | 实现健康状态管理 | 0.5 天 | 测试通过 |
| **配置** | 2 | 设计配置结构 | 0.25 天 | `gateway-config/lib.rs` |
| | 3 | 编写配置解析测试 | 0.25 天 | `tests/config_test.rs` |
| | 4 | 实现 TOML 解析 | 0.5 天 | 测试通过 |
| | 3 | 编写热加载测试 | 0.25 天 | `tests/hot_reload_test.rs` |
| | 4 | 实现热加载 | 0.5 天 | 测试通过 |
| **统计** | 2 | 设计指标收集接口 | 0.25 天 | `gateway-metrics/lib.rs` |
| | 3 | 编写指标测试 | 0.25 天 | `tests/metrics_test.rs` |
| | 4 | 实现指标收集 | 0.25 天 | 测试通过 |
| | 3 | 编写 Prometheus 导出测试 | 0.25 天 | `tests/prometheus_test.rs` |
| | 4 | 实现 Prometheus 导出 | 0.25 天 | 测试通过 |
| **CLI** | 2 | 设计 CLI 命令结构 | 0.25 天 | `gateway-cli/lib.rs` |
| | 3 | 编写 CLI 测试 | 0.25 天 | `tests/cli_test.rs` |
| | 4 | 实现 CLI 命令 | 0.25 天 | 测试通过 |
| **集成** | 5 | 端到端集成测试 | 1.5 天 | `tests/e2e_test.rs` |

**总计：** 约 12 天（1.5 周）

### 1.3 交付物

- [ ] 可运行的二进制文件
- [ ] 基础文档（README、配置示例）
- [ ] 协议文档（`docs/protocols/`）
- [ ] 单元测试覆盖核心模块
- [ ] 集成测试覆盖主要场景

### 1.4 配置示例

```toml
# Input Nodes - HTTP 服务器
[[input_nodes]]
id = "input-openai"
port = 8000

[[input_nodes]]
id = "input-anthropic"
port = 8001

# Virtual Nodes - 路由策略
[[virtual_nodes]]
id = "router-main"
strategy = "weighted"
targets = ["output-openai", "output-anthropic"]

[virtual_nodes.strategy_config]
weights = [70, 30]

# Output Nodes - 后端配置
[[output_nodes]]
id = "output-openai"
provider = "openai"
endpoint = "https://api.openai.com/v1"
api_key = "${OPENAI_API_KEY}"

# 健康检查配置
[output_nodes.health_check]
enabled = true
interval_secs = 30

# 连接关系
[[connections]]
from = "input-openai"
to = "router-main"

[[connections]]
from = "router-main"
to = "output-openai"

[[connections]]
from = "router-main"
to = "output-anthropic"
```

---

## Phase 2：功能完善

**目标：** 完善负载均衡能力，支持非流式热迁移，CLI 增强

**预计周期：** 2-3 周

### 2.1 核心功能

#### 2.1.1 高级负载均衡

- [ ] 最少连接策略
- [ ] 延迟感知策略
- [ ] 基于使用量路由

#### 2.1.2 非流式热迁移

- [ ] 后端故障检测
- [ ] 自动重试路由
- [ ] 可配置重试次数
- [ ] 重试统计

**验收标准：**

- 后端故障时自动切换到备用后端
- 客户端感知为延迟增加，非错误
- 可配置最大重试次数

#### 2.1.3 CLI 增强

- [ ] 节点启用/禁用
- [ ] 动态权重调整
- [ ] 节点连接关系图（ASCII）
- [ ] 实时日志查看

#### 2.1.4 Tier 2 协议支持

- [ ] Ollama Backend
- [ ] Cohere Backend
- [ ] Google Vertex AI Backend

### 2.2 技术任务

| 任务 | Crate | 预计工时 | 依赖 |
|:-----|:------|:---------|:-----|
| 最少连接策略 | gateway-core | 0.5 天 | - |
| 延迟感知策略 | gateway-core | 1 天 | gateway-metrics |
| 热迁移逻辑 | gateway-core | 2 天 | gateway-adapters |
| CLI 动态命令 | gateway-cli | 1 天 | gateway-core |
| Ollama Backend | gateway-adapters | 1 天 | - |
| Cohere Backend | gateway-adapters | 1 天 | - |
| Vertex AI Backend | gateway-adapters | 1.5 天 | - |
| 集成测试 | tests/ | 2 天 | 全部 |

**总计：** 约 10 天（1.5 周）

---

## Phase 3：生产就绪

**目标：** 限流、HTTP 管理 API、高级观测性、JWT 认证

**预计周期：** 2-3 周

### 3.1 核心功能

#### 3.1.1 高级限流

- [ ] 令牌桶算法
- [ ] 滑动窗口限流
- [ ] 每节点独立限流配置
- [ ] 分层限流（Input → Virtual → Output）

#### 3.1.2 HTTP 管理 API

- [ ] REST API 动态配置
- [ ] 节点管理 API
- [ ] 连接管理 API
- [ ] API 认证

#### 3.1.3 JWT 认证

- [ ] JWT 验证中间件
- [ ] OAuth2/OIDC 集成
- [ ] 多租户支持
- [ ] Claims 映射到路由上下文

#### 3.1.4 分布式追踪

- [ ] OpenTelemetry 集成
- [ ] 追踪 ID 透传
- [ ] Span 创建（每节点）
- [ ] 导出到 Jaeger/Zipkin

### 3.2 技术任务

| 任务 | Crate | 预计工时 | 依赖 |
|:-----|:------|:---------|:-----|
| 令牌桶限流 | gateway-core | 1 天 | - |
| 滑动窗口限流 | gateway-core | 1 天 | - |
| HTTP API Server | src/ | 2 天 | gateway-core |
| JWT 中间件 | src/ | 1.5 天 | - |
| OpenTelemetry 集成 | gateway-metrics | 2 天 | - |
| 集成测试 | tests/ | 2 天 | 全部 |

**总计：** 约 10 天（1.5 周）

---

## Phase 4：生态建设

**目标：** 插件系统、语义缓存、多区域集群、流式热迁移

**预计周期：** 3-4 周

### 4.1 核心功能

#### 4.1.1 插件系统

- [ ] WASM 插件支持
- [ ] 动态库插件支持
- [ ] 插件生命周期管理
- [ ] 插件沙箱

#### 4.1.2 语义缓存

- [ ] 嵌入生成集成
- [ ] 向量存储（可选 Weaviate/内存）
- [ ] 相似度匹配
- [ ] 缓存失效策略

#### 4.1.3 多区域集群

- [ ] 节点状态同步（Redis）
- [ ] 分布式图路由
- [ ] 跨区域故障转移
- [ ] 一致性哈希

#### 4.1.4 流式负载热迁移

- [ ] 中断点检测
- [ ] 状态保存与恢复
- [ ] 多后端上下文协调
- [ ] 输出连贯性保证

### 4.2 技术任务

| 任务 | Crate | 预计工时 | 依赖 |
|:-----|:------|:---------|:-----|
| WASM 插件系统 | gateway-core | 3 天 | - |
| 语义缓存 | gateway-core | 2 天 | - |
| Redis 状态后端 | gateway-core | 1.5 天 | - |
| 分布式路由 | gateway-core | 2 天 | Redis 后端 |
| 流式热迁移 | gateway-core | 3 天 | Phase 2 热迁移 |
| 集成测试 | tests/ | 3 天 | 全部 |

**总计：** 约 15 天（2 周）

---

## 里程碑总览

| 里程碑 | 预计完成 | 核心交付 |
|:-------|:---------|:---------|
| Phase 1 (MVP) | 第 1.5 周 | 核心代理、Tier 1 协议、健康感知 DFS、基础统计 |
| Phase 2 (功能完善) | 第 3 周 | 高级负载均衡、热迁移、Tier 2 协议、CLI 增强 |
| Phase 3 (生产就绪) | 第 4.5 周 | 限流、HTTP API、JWT、分布式追踪 |
| Phase 4 (生态建设) | 第 6.5 周 | 插件系统、语义缓存、多区域集群 |

---

## 版本规划

### v0.1.0（Phase 1 完成）

```toml
version = "0.1.0"

[features]
default = ["openai", "anthropic"]
openai = []
anthropic = []
```

**功能：**

- OpenAI 和 Anthropic 协议支持
- 基础路由（轮询、权重）
- 健康感知 + DFS 回溯
- TOML 配置 + 热加载
- Prometheus 指标
- CLI 基础命令

### v0.2.0（Phase 2 完成）

```toml
version = "0.2.0"

[features]
default = ["openai", "anthropic", "ollama", "cohere"]
ollama = []
cohere = []
vertex-ai = []
```

**新增功能：**

- Ollama、Cohere、Vertex AI 支持
- 高级负载均衡（最少连接、延迟感知）
- 非流式热迁移
- CLI 动态管理

### v0.3.0（Phase 3 完成）

```toml
version = "0.3.0"

[features]
default = ["full"]
full = ["openai", "anthropic", "ollama", "cohere", "rate-limit", "http-api", "jwt", "otel"]
rate-limit = []
http-api = []
jwt = []
otel = []
```

**新增功能：**

- 高级限流
- HTTP 管理 API
- JWT 认证
- OpenTelemetry 集成

### v1.0.0（Phase 4 完成）

```toml
version = "1.0.0"

[features]
default = ["full"]
full = ["openai", "anthropic", "ollama", "cohere", "rate-limit", "http-api", "jwt", "otel", "plugins", "semantic-cache", "cluster"]
plugins = []
semantic-cache = []
cluster = []
```

**新增功能：**

- WASM 插件系统
- 语义缓存
- 多区域集群支持
- 流式负载热迁移

---

## 下一步行动

1. 确认 Phase 1 功能范围
2. 创建项目仓库和初始结构
3. 开始 Phase 1 开发（TDD 流程）
