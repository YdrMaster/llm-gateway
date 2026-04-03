# POC 开发工作总结

> **完成日期**: 2026-04-03
> **分支**: `feature/poc-llm-gateway`
> **提交数**: 5 次 (7a05385 → 0fbb98e → 78219bb)

## 1. 验证结果

| 验证项                        | 结果                                                                                                                                                                       | 状态        |
|-------------------------------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------|-------------|
| **SSE 流式协议转换**          | `llm-gateway-protocols` 复用的 `SseCollector` + `StreamingCollector` 在 Pingora 管道中正常工作，`upstream_response_body_filter` 实现逐帧 SSE 解析和协议转换                | ✅ 通过     |
| **并发限制**                  | 通过 `tokio::sync::Semaphore` + `OwnedSemaphorePermit` 存储在 `ProxyContext` 中，`CTX` 生命周期覆盖完整请求（从 `request_body_filter` 到 `logging`），流结束后许可自动释放 | ✅ 通过     |
| **Failover（TCP 级别）**      | 基于健康评分的 `FailoverStrategy` 自动选择后端，连接级错误通过 `fail_to_connect` 触发 Pingora 重试循环                                                                     | ✅ 通过     |
| **Failover（HTTP 5xx 级别）** | 可在 `upstream_response_filter` 中检测 5xx，但**无法静默重试**——Pingora 标准管道在 `upstream_response_filter` 后立即将响应头写入下游，此时 5xx 状态已发送给客户端          | ⚠️ 限制确认 |

## 2. 产出物

### 2.1 代码

```plaintext
poc/
├── Cargo.toml
├── src/
│   ├── main.rs                          # 入口 + Mock 后端（流式 SSE）
│   ├── proxy.rs                         # ProxyHttp 实现（核心代理逻辑）
│   ├── backend.rs                       # 后端管理（地址、健康度）
│   ├── middleware/
│   │   ├── concurrency.rs               # 并发限制中间件
│   │   └── protocol.rs                  # 协议转换中间件
│   └── strategy/
│       ├── mod.rs                       # streaming_context 模块
│       ├── chain.rs                     # 策略链执行器
│       ├── failover.rs                  # 健康度 + Failover 策略
│       └── streaming_context.rs         # 流式转换上下文
├── tests/
│   ├── common/mod.rs                    # 测试基础设施
│   ├── integration_streaming.rs         # SSE 流式 + 协议转换测试
│   ├── integration_failover.rs          # Failover 成功率测试
│   ├── integration_concurrency.rs       # 并发限制测试
│   └── integration_http_retry.rs        # HTTP 5xx 检测测试
└── README.md
```

- **文件数**: 17 个
- **Rust 代码行数**: ~1539 行

### 2.2 测试覆盖

| 测试文件                     | 验证内容                                       | 结果            |
|------------------------------|------------------------------------------------|-----------------|
| `integration_streaming.rs`   | SSE 流式响应通过代理，协议转换正确             | ✅ 通过         |
| `integration_failover.rs`    | 5 并发请求，Mock A 50% 失败时自动切换到 Mock B | ✅ 通过率 > 90% |
| `integration_concurrency.rs` | 限制 5 并发，8 并发请求时 3 个返回 503         | ✅ 通过         |
| `integration_http_retry.rs`  | 5xx 检测日志记录                               | ✅ 通过         |

### 2.3 技术发现

详见 [Pingora HTTP 级别 failover 架构限制](#4-http-5xx-级别-failover-限制说明)

## 3. 关键技术决策

### 3.1 复用 `llm-gateway-protocols` crate

✅ **成功**。`SseCollector`、`StreamingCollector`、`AnthropicToOpenai` 等类型可直接在 Pingora 管道中使用，无需修改。`upstream_response_body_filter` 中通过 `StreamingTransformCtx` 包装转换器，逐 chunk 处理 SSE 数据。

### 3.2 并发限制通过 `ProxyContext` 持有 `OwnedSemaphorePermit`

✅ **成功**。`request_body_filter` 中获取许可，存储在 `ctx._permit`，`CTX` 在 `logging` 阶段后 Drop，许可自动释放。生命周期覆盖整个请求。

### 3.3 Pingora 管道中响应头写入时机

⚠️ **关键限制**。Pingora 的 `ProxyHttp` 管道调用顺序：

```plaintext
upstream_response_filter  →  可检测 5xx 状态码
response_filter           →  可读取/修改响应头（但已准备写入下游）
upstream_response_body_filter →  body 过滤
response_body_filter      →  body 过滤
```

在 `upstream_response_filter` 之后，响应头已经**写入下游客户端**。即使后续在 `upstream_response_body_filter` 或 `response_body_filter` 中返回 `Err`，5xx 状态码也已经发送给客户端，客户端连接已终结，无法"撤回"响应并静默重试。

## 4. HTTP 5xx 级别 Failover 限制说明

### 问题

Pingora 默认将 HTTP 5xx 响应视为"有效响应"而非错误。它不会触发 `error_while_proxy`、`fail_to_connect` 等错误回调，也不会进入重试循环。

### 尝试的方案

在 `upstream_response_filter` 中检测 5xx → 在 `upstream_response_body_filter` 中清空 body → 返回 `Err` 触发 Pingora 重试循环。

### 为什么行不通

Pingora 的响应头在 `upstream_response_filter` 之后立即通过 `session.write_response_tasks()` 写入下游。此时 `500 Internal Server Error` 已经通过网络发送给客户端。后续阶段返回 `Err` 只能影响 body 传输，无法改变已发送的响应头。

### 解决方案（Phase 1）

**`custom_forwarding`**：放弃标准 `ProxyHttp` 管道，在 `custom_forwarding` 回调中手动控制 HTTP 请求/响应周期：

1. 读取完整请求体
2. 向后端发起 HTTP 请求（使用自己的 HTTP 客户端）
3. 检测 5xx → 切换到下一个后端
4. 将最终响应写回客户端

这样完全控制请求/响应周期，不受标准管道限制。

## 5. 决策建议

| 检查项              | POC 结果                               | 决策    |
|---------------------|----------------------------------------|---------|
| SSE 流式转换        | 功能正确，实时流式处理                 | ✅ Done |
| 并发控制            | 能正确保持到流结束                     | ✅ Done |
| Failover (TCP)      | 能自动切换后端                         | ✅ Done |
| Failover (HTTP 5xx) | 标准管道无法实现，需 custom_forwarding | ⚠️ 待定 |
| 开发复杂度          | 代码清晰，管道职责分明                 | ✅ Done |

**结论**: Pingora 适合本项目。POC 验证了所有核心能力，HTTP 5xx failover 的限制需在 Phase 1 中评估是否通过 `custom_forwarding` 解决。
