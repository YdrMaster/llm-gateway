# LLM Gateway

LLM 网关服务，提供请求路由、协议转换和统计功能。

## 功能特性

- **请求路由**：根据模型名称自动路由到后端服务
- **协议转换**：支持不同 LLM API 协议之间的转换
- **健康监控**：自动检测后端服务健康状态，故障时自动切换
- **统计功能**：记录和聚合请求统计数据
- **管理 API**：提供 REST API 查询统计信息

## 快速开始

### 配置

编辑 `config.toml`：

```toml
[input.service]
port = 9000
models = ["qwen3.5-35b-a3b", "qwen3.5-122b-a10b"]

[backend]
"qwen3.5-35b-a3b" = "http://localhost:8001"

[statistics]
enabled = true
db-path = "stats.db"

[admin]
port = 8080
```

### 运行

```bash
cargo run --release
```

## 管理 API

启动服务后，可通过以下端点查询统计信息：

| 端点                     | 说明                                       |
|--------------------------|--------------------------------------------|
| `GET /v1/stats/overview` | 获取最近 1 小时的统计概览                  |
| `GET /v1/stats/aggregate`| 获取聚合统计数据，支持时间范围和过滤器     |

详情请参阅 [Admin API 文档](docs/usage/admin-api.md)。

## CLI 工具

统计模块提供独立的 CLI 工具 `llm-stats`，支持交互式查询统计数据。

```bash
cargo run --release -p llm-gateway-statistics --bin llm-stats -- --db stats.db
```

### 交互命令

| 命令       | 说明           |
|------------|----------------|
| `query`    | 查询原始事件   |
| `stats`    | 查看聚合统计   |
| `models`   | 列出所有模型   |
| `backends` | 列出所有后端   |
| `recent`   | 查看最近事件   |
| `detail`   | 查看事件详情   |

详情请参阅 [CLI 工具使用指南](docs/usage/cli.md)。
