# LLM Gateway POC

**日期**: 2026-04-02  
**状态**: ✅ 验证完成

## 概述

本 POC 验证 LLM Gateway 使用 Pingora 框架的三个核心功能：

1. **SSE 流式协议转换** - Anthropic ↔ OpenAI 协议双向转换
2. **并发限制** - 基于模型的并发请求数控制
3. **Failover 机制** - 后端失败时自动切换到备选

## 快速开始

### 环境要求

- Rust 工具链（stable）
- cmake（用于构建依赖）

### 运行 POC

```bash
# 进入 POC 目录
cd poc

# 编译并运行
cargo run
```

### 预期输出

```
[INFO] Starting LLM Gateway POC
[INFO] Starting mock backends...
[INFO] Mock backends started: OpenAI-A:18001, OpenAI-B:18002, Anthropic:18003

=== 验证项 1: 协议转换 ===
[INFO] 测试 Anthropic -> OpenAI 协议转换...
[INFO]   转换输出：data {"choices":[{"delta":{"content":"Hello"},...}
[INFO] 协议转换验证完成 ✓

=== 验证项 2: 并发限制 ===
[INFO] 设置并发限制为 2
[INFO]   请求 1: true
[INFO]   请求 2: true
[INFO]   请求 3: false
[INFO] 并发限制验证完成 ✓

=== 验证项 3: Failover ===
[INFO] 配置 Failover: [Backend-A(50% 失败), Backend-B(正常)]
[INFO] Failover 验证完成 ✓

=== 所有验证完成 ===
```

## 项目结构

```
poc/
├── Cargo.toml
├── src/
│   ├── main.rs              # 入口和验证测试
│   ├── backend.rs           # 后端管理模块
│   ├── middleware/
│   │   ├── mod.rs
│   │   ├── concurrency.rs   # 并发限制中间件
│   │   └── protocol.rs      # 协议转换中间件
│   └── strategy/
│       ├── mod.rs
│       ├── chain.rs         # 策略链执行器
│       └── failover.rs      # Failover 策略
└── README.md
```

## 验证详情

### 1. 协议转换

使用 `llm-gateway-protocols` crate 中的 `AnthropicToOpenai` 转换器：

```rust
let mut conv: Box<dyn StreamingCollector> = Box::new(AnthropicToOpenai::default());
let converted = converter.process_message(&mut conv, anthropic_message)?;
```

**输入** (Anthropic SSE):
```
event: content_block_delta
data: {"type":"content_block_delta","delta":{"type":"text_delta","text":"Hello"}}
```

**输出** (OpenAI SSE):
```
data: {"choices":[{"delta":{"content":"Hello"},"index":0}],"object":"chat.completion.chunk"}
```

### 2. 并发限制

使用 `tokio::sync::Semaphore` 实现：

```rust
let concurrency_layer = ConcurrencyLimitLayer::new(2); // 限制为 2
let permit = concurrency_layer.acquire("model-name").await?;
// permit 在作用域结束时自动释放
```

### 3. Failover

使用策略模式实现：

```rust
let failover = FailoverStrategy::new(vec![
    "127.0.0.1:18001".to_string(),  // 主后端
    "127.0.0.1:18002".to_string(),  // 备选后端
]);
let backend = failover.select_backend(&context).await?;
```

## Mock 后端

POC 包含三个内置 Mock 后端：

| 后端 | 端口 | 行为 |
|------|------|------|
| OpenAI-A | 18001 | 50% 随机失败 |
| OpenAI-B | 18002 | 总是正常 |
| Anthropic | 18003 | 返回 Anthropic 格式 SSE |

## 依赖

```toml
[dependencies]
llm-gateway-protocols = { path = "../../../crates/protocols" }
tokio = { version = "1", features = ["rt-multi-thread", "net", "sync", "time", "signal"] }
serde_json = "1"
log = "0.4"
env_logger = "0.11"
async-trait = "0.1"
dashmap = "6"
rand = "0.8"
```

## 验证结果

所有验证项均通过：

| 验证项 | 状态 |
|--------|------|
| SSE 流式协议转换 | ✅ |
| 并发限制 | ✅ |
| Failover | ✅ |

详细报告见：[docs/plan/2026-04-02-poc-report.md](../../docs/plan/2026-04-02-poc-report.md)

## 下一步

基于 POC 验证结果，将继续进行正式开发：

1. **Phase 1**: Pingora 基础框架集成
2. **Phase 2**: 核心策略插件实现
3. **Phase 3**: 协议转换与可观测性
4. **Phase 4**: 测试与文档

## 许可证

与主项目相同
