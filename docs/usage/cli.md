# CLI 工具

LLM Gateway 统计 CLI 工具 `llm-stats`，提供交互式 REPL 环境查询统计数据。

## 快速开始

### 启动

```bash
cargo run --release -p llm-gateway-statistics --bin llm-stats -- --db stats.db
```

### 参数

| 参数   | 说明                  | 默认值       |
|--------|-----------------------|--------------|
| `--db` | SQLite 数据库文件路径 | `./stats.db` |

启动后进入 REPL 环境：

```plaintext
LLM Gateway Statistics CLI
Stats DB: stats.db

Connected. Total events: 1234

>
```

## 命令参考

### query - 查询原始事件

查询符合条件的事件记录。

**语法：**

```plaintext
query [--last <duration>] [--start <timestamp>] [--end <timestamp>]
      [--model <name>] [--backend <name>] [--success <true|false>]
      [--limit <n>] [--format <table|json|csv>]
```

**选项：**

| 选项        | 说明                         | 示例                                   |
|-------------|------------------------------|----------------------------------------|
| `--last`    | 查询最近一段时间             | `--last 1h`, `--last 24h`, `--last 7d` |
| `--start`   | 起始时间（时间戳或 ISO8601） | `--start 1609459200000`                |
| `--end`     | 结束时间（时间戳或 ISO8601） | `--end 2021-01-01T00:00:00Z`           |
| `--model`   | 按模型名称过滤               | `--model qwen-35b`                     |
| `--backend` | 按后端服务过滤               | `--backend sglang`                     |
| `--success` | 按成功状态过滤               | `--success true`                       |
| `--limit`   | 限制返回数量                 | `--limit 50`                           |
| `--format`  | 输出格式                     | `--format json`                        |

**示例：**

```plaintext
> query --last 1h --model qwen-35b --limit 10
> query --last 24h --success true --format json
> query --backend sglang --last 7d
```

### stats - 查看聚合统计

查看按时间窗口聚合的统计数据。

**语法：**

```plaintext
stats [--last <duration>] [--granularity <duration>]
```

**选项：**

| 选项            | 说明             | 默认值         |
|-----------------|------------------|----------------|
| `--last`        | 统计最近一段时间 | `1h`           |
| `--granularity` | 时间窗口大小     | `1h` (3600 秒) |

**示例：**

```plaintext
> stats --last 24h --granularity 1h
> stats --last 7d --granularity 1d
```

**输出示例：**

```plaintext
Aggregated statistics:
  qwen3.5-35b-a3b / sglang: 1523 requests, 245ms avg
  qwen3.5-122b-a10b / sglang: 892 requests, 312ms avg
  Summary: finished at 2026-03-30T12:00:00+00:00, remaining 0s (finished)
```

### models - 列出所有模型

列出所有有请求记录的模型及其请求数量。

**语法：**

```plaintext
models [--sort <field>] [--format <table|json|csv>]
```

**选项：**

| 选项       | 说明     | 默认值  |
|------------|----------|---------|
| `--sort`   | 排序字段 | `count` |
| `--format` | 输出格式 | `table` |

**示例：**

```plaintext
> models
> models --sort name --format json
```

**输出示例：**

```plaintext
Available models:
  qwen3.5-35b-a3b      (1523 events)
  qwen3.5-122b-a10b    (892 events)
  deepseek-v3          (456 events)
```

### backends - 列出所有后端

列出所有后端服务及其请求统计和成功率。

**语法：**

```plaintext
backends [--sort <field>] [--format <table|json|csv>]
```

**选项：**

| 选项       | 说明     | 默认值  |
|------------|----------|---------|
| `--sort`   | 排序字段 | `count` |
| `--format` | 输出格式 | `table` |

**示例：**

```plaintext
> backends
> backends --sort name
```

**输出示例：**

```plaintext
Available backends:
  sglang               (2415 events, 98.5% success)
  vllm                 (456 events, 97.2% success)
```

### recent - 查看最近事件

快速查看最近的若干条事件记录。

**语法：**

```plaintext
recent [-n <limit>]
```

**选项：**

| 选项   | 说明     | 默认值 |
|--------|----------|--------|
| `-n`   | 显示数量 | `20`   |

**示例：**

```plaintext
> recent
> recent -n 50
```

### detail - 查看事件详情

查看缓存查询结果中指定索引的事件详情。

**语法：**

```plaintext
detail [<index>]
```

**参数：**

| 参数    | 说明                  | 默认值 |
|---------|-----------------------|--------|
| `index` | 事件索引（从 0 开始） | `0`    |

**示例：**

```plaintext
> query --last 1h
> detail 0
> detail 5
```

**输出示例：**

```plaintext
Event #0 Details:
────────────────────────────────────────
Timestamp:    2026-03-30T12:34:56.789+00:00
Model:        qwen3.5-35b-a3b
Backend:      sglang
Duration:     245ms
Success:      ✓ Yes

Request:
  Client:     192.168.1.100:8080
  Method:     POST
  Path:       /v1/chat/completions
  Input Port: 9000
  Size:       1024 bytes

Response:
  Size:       4096 bytes

Routing Path:
  /v1/chat/completions

Error:        (none)
```

### help / ? - 显示帮助

显示可用命令列表和简要说明。

**示例：**

```plaintext
> help
> ?
```

### exit / quit / q - 退出

退出 CLI 工具。

**示例：**

```plaintext
> exit
> quit
> q
```

## 输出格式

支持三种输出格式：

| 格式    | 说明       | 适用场景         |
|---------|------------|------------------|
| `table` | 表格格式   | 交互式查看（默认） |
| `json`  | JSON 格式  | 程序处理         |
| `csv`   | CSV 格式   | 导出到电子表格   |

**示例：**

```plaintext
> query --last 1h --format json
> models --format csv
```

## 时间格式

### 相对时间

使用 `humantime` 格式：

| 单位 | 格式 | 示例         |
|------|------|--------------|
| 秒   | `s`  | `30s`, `5s`  |
| 分   | `m`  | `15m`, `30m` |
| 时   | `h`  | `1h`, `24h`  |
| 天   | `d`  | `7d`, `30d`  |

### 绝对时间

支持两种格式：

- **Unix 时间戳（毫秒）**：`1609459200000`
- **ISO8601**：`2021-01-01T00:00:00Z`

## 使用场景

### 故障排查

```plaintext
# 查看最近 1 小时的失败请求
> query --last 1h --success false

# 查看特定模型的请求详情
> query --model qwen3.5-35b-a3b --last 24h
> detail 0
```

### 性能分析

```plaintext
# 查看各模型的请求延迟
> stats --last 24h --granularity 1h

# 查看后端服务的成功率
> backends
```

### 数据导出

```plaintext
# 导出最近 24 小时的数据为 JSON
> query --last 24h --format json

# 导出模型统计为 CSV
> models --format csv
```

## 与 Admin API 对比

| 特性         | CLI 工具           | Admin API        |
|--------------|--------------------|------------------|
| 交互方式     | 交互式 REPL        | REST API         |
| 输出格式     | table/json/csv     | JSON             |
| 适用场景     | 本地调试、排查问题 | 集成、监控面板   |
| 数据源       | 直接读取 SQLite    | HTTP 服务        |
| 认证         | 无                 | 支持 Token 认证  |
