//! LLM Gateway 配置解析模块
//!
//! 本模块提供 TOML 格式配置文件的解析功能，将配置转换为强类型的 [`GatewayConfig`] 结构。
//!
//! # 配置格式
//!
//! 配置文件支持三种节点类型：
//!
//! ## 输入节点 (Input Node)
//!
//! 定义网关的入口，指定监听端口和可用的模型列表：
//!
//! ```toml
//! [input.service]
//! port = 8000
//! models = ["model-a", "model-b"]
//! ```
//!
//! ## 虚拟节点 (Virtual Node)
//!
//! 定义模型的路由策略，目前支持顺序尝试（sequence）：
//!
//! ```toml
//! [node."model-a"]
//! sequence = ["backend-1", "backend-2"]
//! ```
//!
//! ## 后端节点 (Backend Node)
//!
//! 定义实际的后端服务，支持两种格式：
//!
//! 简单格式（直接转发）：
//! ```toml
//! [backend]
//! "backend-1" = "http://192.168.1.1:8000"
//! ```
//!
//! 复杂格式（支持协议转换和 API Key）：
//! ```toml
//! [backend.aliyun]
//! base-url = { anthropic = "https://api.example.com" }
//! api-key = "$API_KEY"
//! ```
//!
//! # 错误处理
//!
//! 解析失败时返回 [`ConfigParseError`]，包含详细的错误信息和路径

mod admin;
mod backend;
mod error;
mod health;
mod input;
mod node;

pub use admin::AdminConfig;
pub use backend::{BackendNode, BaseUrl, UrlResult};
pub use error::Error as ConfigParseError;
pub use health::{HealthConfig, InternalHealthConfig};
pub use input::InputNode;
pub use node::{Node, VirtualNode};
use std::{collections::HashMap, str::FromStr};

/// 网关配置根结构
///
/// 包含所有节点的映射，节点名称作为 key
#[derive(Clone, Debug)]
pub struct GatewayConfig {
    pub nodes: HashMap<String, Node>,
    pub statistics: Option<llm_gateway_statistics::StatisticsConfig>,
    pub health: Option<HealthConfig>,
    pub admin: Option<AdminConfig>,
}

impl FromStr for GatewayConfig {
    type Err = ConfigParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // 解析 TOML 为动态 Value 类型
        let parsed = toml::from_str::<toml::Value>(s)
            .map_err(|e| ConfigParseError::ParseError(e.to_string()))?;

        // 获取根表，空配置返回空配置对象
        let Some(root_table) = parsed.as_table() else {
            return Ok(GatewayConfig {
                nodes: HashMap::new(),
                statistics: None,
                health: None,
                admin: None,
            });
        };

        // 解析统计配置 [statistics]
        let statistics = parse_statistics_config(root_table);

        // 解析健康监控配置 [health]
        let health = root_table
            .get("health")
            .and_then(|v| v.as_table())
            .and_then(|table| {
                let toml_str = toml::to_string(table).ok()?;
                toml::from_str(&toml_str).ok()
            });

        // 解析管理配置 [admin]
        let admin = parse_admin_config(root_table);

        let mut nodes: HashMap<_, _> = HashMap::new();

        // 解析输入节点 [input.*]
        // 输入节点定义网关入口，包含 port 和 models 字段
        if let Some(input_table) = root_table.get("input").and_then(|v| v.as_table()) {
            for (name, value) in input_table {
                // 检查节点名称是否重复
                if nodes.contains_key(name.as_str()) {
                    return Err(ConfigParseError::DuplicateName(name.clone()));
                }

                if let Some(node_table) = value.as_table() {
                    // 解析 port 字段（必需）
                    let port = node_table
                        .get("port")
                        .and_then(|v| v.as_integer())
                        .ok_or_else(|| {
                            ConfigParseError::MissingField(
                                "port".to_string(),
                                format!("input.{name}"),
                            )
                        })? as u16;

                    // 解析 models 字段（可选，默认为空列表）
                    let models = node_table
                        .get("models")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();

                    // 验证 models 内部无重复
                    let mut model_set = std::collections::HashSet::new();
                    for model in &models {
                        if !model_set.insert(model.as_str()) {
                            return Err(ConfigParseError::DuplicateName(format!(
                                "input.{name}.models has duplicate model name: {model}"
                            )));
                        }
                    }

                    // 解析 alias 字段（可选，默认为空映射）
                    let alias = node_table
                        .get("alias")
                        .and_then(|v| v.as_table())
                        .map(|table| {
                            table
                                .iter()
                                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                                .collect::<HashMap<_, _>>()
                        })
                        .unwrap_or_default();

                    // 验证 alias 键与 models 无重复（别名不能与模型名相同）
                    for alias_key in alias.keys() {
                        if model_set.contains(alias_key.as_str()) {
                            return Err(ConfigParseError::DuplicateName(format!(
                                "input.{name}.alias has alias '{alias_key}' that duplicates a model name in models"
                            )));
                        }
                    }

                    // 验证 alias 的值（映射目标）都在 models 中
                    for (alias_key, target_model) in &alias {
                        if !model_set.contains(target_model.as_str()) {
                            return Err(ConfigParseError::ParseError(format!(
                                "input.{name}.alias has alias '{alias_key}' mapping to non-existent model '{target_model}'"
                            )));
                        }
                    }

                    nodes.insert(
                        name.clone(),
                        Node::Input(InputNode {
                            port,
                            models,
                            alias,
                        }),
                    );
                }
            }
        }

        // 解析虚拟节点 [node.*]
        // 虚拟节点定义路由策略，目前支持 sequence（顺序尝试）和 concurrency（并发控制）
        if let Some(node_table) = root_table.get("node").and_then(|v| v.as_table()) {
            for (name, value) in node_table {
                // 检查节点名称是否重复
                if nodes.contains_key(name.as_str()) {
                    return Err(ConfigParseError::DuplicateName(name.clone()));
                }

                if let Some(node_config) = value.as_table() {
                    // 解析 sequence 字段
                    if let Some(sequence) = node_config.get("sequence").and_then(|v| v.as_array()) {
                        let seq = sequence
                            .iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect();

                        nodes.insert(name.clone(), Node::Virtual(VirtualNode::Sequence(seq)));
                    }
                    // 解析 concurrency 配置
                    else if let Some(concurrency_config) = node_config.get("concurrency") {
                        // 解析 max 字段
                        let max_val = concurrency_config
                            .get("max")
                            .and_then(|v| v.as_integer())
                            .ok_or_else(|| {
                                ConfigParseError::MissingField(
                                    "max".to_string(),
                                    format!("node.{name}"),
                                )
                            })?;

                        // 验证 max 为正整数
                        if max_val <= 0 {
                            return Err(ConfigParseError::ParseError(format!(
                                "node.{name}.concurrency.max must be a positive integer, got: {max_val}"
                            )));
                        }
                        let max = max_val as usize;

                        // 解析 successor 字段（单后继）
                        let successor = concurrency_config
                            .get("successor")
                            .and_then(|v| v.as_str())
                            .map(String::from)
                            .ok_or_else(|| {
                                ConfigParseError::MissingField(
                                    "successor".to_string(),
                                    format!("node.{name}"),
                                )
                            })?;

                        nodes.insert(
                            name.clone(),
                            Node::Virtual(VirtualNode::Concurrency { max, successor }),
                        );
                    } else {
                        return Err(ConfigParseError::ParseError(format!(
                            "node.{name} must have either 'sequence' or 'concurrency' field"
                        )));
                    }
                }
            }
        }

        // 解析后端节点 [backend.*]
        // 后端节点支持两种格式：
        // 1. 简单字符串：[backend] 下的键值对，直接转发
        // 2. 表配置：[backend.name] 下的 base-url 和 api-key
        if let Some(backend_table) = root_table.get("backend").and_then(|v| v.as_table()) {
            for (name, value) in backend_table {
                // 检查节点名称是否重复
                if nodes.contains_key(name.as_str()) {
                    return Err(ConfigParseError::DuplicateName(name.clone()));
                }

                // 格式 1: 简单字符串值 - [backend] 下的键值对
                if let Some(url_str) = value.as_str() {
                    let base_url = BaseUrl::AllInOne(url_str.into());

                    nodes.insert(
                        name.clone(),
                        Node::Backend(BackendNode {
                            base_url,
                            api_key: None,
                            connect_timeout_ms: None,
                        }),
                    );
                    continue;
                }

                // 格式 2: 表配置 - [backend.name] 下的配置
                if let Some(backend_config) = value.as_table() {
                    // 解析 base-url 字段（必需）
                    let base_url = if let Some(url_value) = backend_config.get("base-url") {
                        if let Some(url_str) = url_value.as_str() {
                            // 简单字符串格式
                            BaseUrl::AllInOne(url_str.into())
                        } else if let Some(url_table) = url_value.as_table() {
                            // 表格式带协议特定 URL
                            let mut map = HashMap::new();
                            for (protocol, url) in url_table {
                                if let Some(url_str) = url.as_str() {
                                    map.insert(protocol.clone(), url_str.to_string());
                                }
                            }
                            BaseUrl::Multi(map)
                        } else {
                            return Err(ConfigParseError::ParseError(
                                "base-url must be a string or table".to_string(),
                            ));
                        }
                    } else {
                        return Err(ConfigParseError::MissingField(
                            "base-url".to_string(),
                            format!("backend.{name}"),
                        ));
                    };

                    // 解析 api-key 字段（可选）
                    let api_key = backend_config
                        .get("api-key")
                        .and_then(|v| v.as_str())
                        .map(String::from);

                    // 解析 connect-timeout-ms 字段（可选）
                    let connect_timeout_ms = backend_config
                        .get("connect-timeout-ms")
                        .and_then(|v| v.as_integer())
                        .map(|v| v as u64);

                    nodes.insert(
                        name.clone(),
                        Node::Backend(BackendNode {
                            base_url,
                            api_key,
                            connect_timeout_ms,
                        }),
                    );
                }
            }
        }

        Ok(GatewayConfig {
            nodes,
            statistics,
            health,
            admin,
        })
    }
}

/// 解析统计配置
fn parse_statistics_config(
    root_table: &toml::Table,
) -> Option<llm_gateway_statistics::StatisticsConfig> {
    root_table
        .get("statistics")
        .and_then(|v| v.as_table())
        .and_then(|table| {
            // 将 TOML 表转换为字符串
            let toml_str = toml::to_string(table).ok()?;
            // 使用 serde 反序列化
            toml::from_str(&toml_str).ok()
        })
}

/// 解析管理配置
fn parse_admin_config(root_table: &toml::Table) -> Option<AdminConfig> {
    root_table.get("admin").map(|v| {
        // 如果 [admin] 存在但为空，使用默认配置（随机端口）
        v.as_table()
            .and_then(|table| {
                let toml_str = toml::to_string(table).ok()?;
                toml::from_str(&toml_str).ok()
            })
            .unwrap_or_default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    /// 完整配置解析测试
    ///
    /// 验证所有节点类型的正确解析
    #[test]
    fn test_parse_full_config() {
        let toml_str = r#"
[input.service]
port = 8000
models = ["qwen3.5-35b-a3b", "qwen3.5-122b-a10b", "kimi-k2.5"]

[node."qwen3.5-35b-a3b"]
sequence = ["sglang-qwen3.5-35b-a3b", "aliyun"]

[node."qwen3.5-122b-a10b"]
sequence = ["sglang-qwen3.5-122b-a10b", "aliyun"]

[backend]
"sglang-qwen3.5-35b-a3b" = "http://172.17.250.163:30001"
"sglang-qwen3.5-122b-a10b" = "http://172.17.250.163:30002"
"sglang-kimi-k2.5" = "http://172.17.250.176:30001"

[backend.aliyun]
base-url = { anthropic = "https://dashscope.aliyuncs.com/apps/anthropic" }
api-key = "$ALIYUN_API_KEY"
"#;

        let result = GatewayConfig::from_str(toml_str);
        assert!(result.is_ok());
        let config = result.unwrap();

        // 验证输入节点
        assert!(config.nodes.contains_key("service"));
        match &config.nodes["service"] {
            Node::Input(input) => {
                assert_eq!(input.port, 8000);
                assert_eq!(input.models.len(), 3);
            }
            _ => panic!("Expected Input node"),
        }

        // 验证虚拟节点
        assert!(config.nodes.contains_key("qwen3.5-35b-a3b"));
        assert!(config.nodes.contains_key("qwen3.5-122b-a10b"));

        // 验证后端
        assert!(config.nodes.contains_key("sglang-qwen3.5-35b-a3b"));
        assert!(config.nodes.contains_key("aliyun"));

        match &config.nodes["aliyun"] {
            Node::Backend(backend) => {
                assert_eq!(backend.api_key, Some("$ALIYUN_API_KEY".to_string()));
            }
            _ => panic!("Expected Backend node"),
        }
    }

    /// 测试输入节点别名解析
    #[test]
    fn test_parse_input_node_with_alias() {
        let toml_str = r#"
[input.service]
port = 8000
models = ["qwen3.5-35b-a3b", "qwen3.5-122b-a10b"]

[input.service.alias]
"data/Qwen3.5-35B-A3B" = "qwen3.5-35b-a3b"
"Qwen3.5-122B-A10B" = "qwen3.5-122b-a10b"
"#;

        let result = GatewayConfig::from_str(toml_str);
        assert!(result.is_ok());
        let config = result.unwrap();

        match &config.nodes["service"] {
            Node::Input(input) => {
                assert_eq!(input.port, 8000);
                assert_eq!(input.models.len(), 2);
                assert_eq!(input.alias.len(), 2);
                assert_eq!(
                    input.alias.get("data/Qwen3.5-35B-A3B"),
                    Some(&"qwen3.5-35b-a3b".to_string())
                );
                assert_eq!(
                    input.alias.get("Qwen3.5-122B-A10B"),
                    Some(&"qwen3.5-122b-a10b".to_string())
                );
            }
            _ => panic!("Expected Input node"),
        }
    }

    /// 测试 models 内部重复检测
    #[test]
    fn test_error_on_duplicate_model_name() {
        let toml_str = r#"
[input.service]
port = 8000
models = ["model-a", "model-b", "model-a"]
"#;

        let result = GatewayConfig::from_str(toml_str);
        assert!(result.is_err());

        match result.unwrap_err() {
            crate::error::Error::DuplicateName(msg) => {
                assert!(msg.contains("model-a"));
                assert!(msg.contains("models"));
            }
            _ => panic!("Expected DuplicateName error for duplicate model"),
        }
    }

    /// 测试别名与模型名重复检测
    #[test]
    fn test_error_on_alias_duplicate_with_model() {
        let toml_str = r#"
[input.service]
port = 8000
models = ["qwen3.5-35b-a3b", "qwen3.5-122b-a10b"]

[input.service.alias]
"qwen3.5-35b-a3b" = "qwen3.5-122b-a10b"
"#;

        let result = GatewayConfig::from_str(toml_str);
        assert!(result.is_err());

        match result.unwrap_err() {
            crate::error::Error::DuplicateName(msg) => {
                assert!(msg.contains("qwen3.5-35b-a3b"));
                assert!(msg.contains("alias"));
                assert!(msg.contains("models"));
            }
            _ => panic!("Expected DuplicateName error for alias-model conflict"),
        }
    }

    /// 测试别名映射目标不存在
    #[test]
    fn test_error_on_alias_target_not_in_models() {
        let toml_str = r#"
[input.service]
port = 8000
models = ["qwen3.5-35b-a3b"]

[input.service.alias]
"data/Qwen3.5-35B-A3B" = "nonexistent-model"
"#;

        let result = GatewayConfig::from_str(toml_str);
        assert!(result.is_err());

        match result.unwrap_err() {
            crate::error::Error::ParseError(msg) => {
                assert!(msg.contains("nonexistent-model"));
                assert!(msg.contains("alias"));
            }
            _ => panic!("Expected ParseError for alias target not in models"),
        }
    }

    /// 测试 admin 配置解析
    #[test]
    fn test_parse_admin_config() {
        let toml_str = r#"
[admin]
port = 8080
auth-token = "secret-token"
"#;

        let result = GatewayConfig::from_str(toml_str);
        assert!(result.is_ok());
        let config = result.unwrap();

        assert!(config.admin.is_some());
        let admin = config.admin.unwrap();
        assert_eq!(admin.port, 8080);
        assert_eq!(admin.auth_token, Some("secret-token".to_string()));
    }

    /// 测试 admin 配置默认值（未指定时）
    #[test]
    fn test_admin_config_default() {
        let toml_str = r#""#;
        let result = GatewayConfig::from_str(toml_str);
        assert!(result.is_ok());
        let config = result.unwrap();

        // Default should be None (not specified)
        assert!(config.admin.is_none());
    }

    /// 测试并发控制配置完整解析
    #[test]
    fn test_parse_concurrency_full_config() {
        let toml_str = r#"
[input.service]
port = 8000
models = ["qwen3.5-35b-a3b"]

[node."qwen3.5-35b-a3b"]
concurrency = { max = 100, successor = "sglang-backend" }

[backend]
"sglang-backend" = "http://172.17.250.163:30001"
"#;

        let result = GatewayConfig::from_str(toml_str);
        assert!(result.is_ok());
        let config = result.unwrap();

        // Verify input node
        assert!(config.nodes.contains_key("service"));

        // Verify concurrency node
        assert!(config.nodes.contains_key("qwen3.5-35b-a3b"));
        match &config.nodes["qwen3.5-35b-a3b"] {
            Node::Virtual(virtual_node) => {
                assert_eq!(virtual_node.concurrency(), Some(100));
                assert_eq!(virtual_node.sequence(), &["sglang-backend"]);
            }
            _ => panic!("Expected Virtual node"),
        }

        // Verify backends
        assert!(config.nodes.contains_key("sglang-backend"));
    }

    /// 测试并发控制配置解析和节点创建
    #[test]
    fn test_concurrency_config_parsing() {
        let toml_str = r#"
[input.service]
port = 8000
models = ["test-model"]

[node."test-model"]
concurrency = { max = 2, successor = "backend-1" }

[backend]
"backend-1" = "http://localhost:8001"
"#;

        let config = GatewayConfig::from_str(toml_str).unwrap();

        // Verify concurrency node exists
        assert!(config.nodes.contains_key("test-model"));
        match &config.nodes["test-model"] {
            Node::Virtual(r#virtual) => {
                assert_eq!(r#virtual.concurrency(), Some(2));
                assert_eq!(r#virtual.sequence(), &["backend-1"]);
            }
            _ => panic!("Expected Virtual node"),
        }
    }

    /// 测试完整配置解析（包含 sequence 和 concurrency）
    #[test]
    fn test_full_config_with_concurrency() {
        let toml_str = r#"
[input.service]
port = 8000
models = ["qwen-35b", "qwen-72b"]

[node."qwen-35b"]
concurrency = { max = 100, successor = "model-router" }

[node."model-router"]
sequence = ["sglang-35b", "aliyun"]

[node."qwen-72b"]
sequence = ["sglang-72b", "aliyun"]

[backend]
"sglang-35b" = "http://172.17.250.163:30001"
"sglang-72b" = "http://172.17.250.163:30002"

[backend.aliyun]
base-url = { anthropic = "https://dashscope.aliyuncs.com/apps/anthropic" }
api-key = "$ALIYUN_API_KEY"
"#;

        let config = GatewayConfig::from_str(toml_str).unwrap();

        // Verify input node
        assert!(config.nodes.contains_key("service"));

        // Verify concurrency node
        assert!(config.nodes.contains_key("qwen-35b"));
        match &config.nodes["qwen-35b"] {
            Node::Virtual(r#virtual) => {
                assert_eq!(r#virtual.concurrency(), Some(100));
                assert_eq!(r#virtual.sequence(), &["model-router"]);
            }
            _ => panic!("Expected Virtual node"),
        }

        // Verify model-router sequence node
        assert!(config.nodes.contains_key("model-router"));
        match &config.nodes["model-router"] {
            Node::Virtual(r#virtual) => {
                assert_eq!(r#virtual.concurrency(), None);
                assert_eq!(r#virtual.sequence(), &["sglang-35b", "aliyun"]);
            }
            _ => panic!("Expected Virtual node"),
        }

        // Verify sequence node
        assert!(config.nodes.contains_key("qwen-72b"));
        match &config.nodes["qwen-72b"] {
            Node::Virtual(r#virtual) => {
                assert_eq!(r#virtual.concurrency(), None);
                assert_eq!(r#virtual.sequence(), &["sglang-72b", "aliyun"]);
            }
            _ => panic!("Expected Virtual node"),
        }

        // Verify backends
        assert!(config.nodes.contains_key("sglang-35b"));
        assert!(config.nodes.contains_key("sglang-72b"));
        assert!(config.nodes.contains_key("aliyun"));
    }

    /// 测试项目根目录的 config.toml 配置文件能否正确解析
    #[test]
    fn test_parse_project_config_file() {
        use std::path::PathBuf;

        // 项目根目录的 config.toml 路径
        let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("config.toml");

        println!("Loading config from: {:?}", config_path);

        // 读取文件内容
        let content = std::fs::read_to_string(&config_path).expect("Failed to read config.toml");

        // 解析配置
        let config = GatewayConfig::from_str(&content).expect("Failed to parse config.toml");

        // 验证输入节点
        assert!(config.nodes.contains_key("service"));
        assert!(config.nodes.contains_key("test"));

        // 验证限流节点 (concurrency)
        assert!(config.nodes.contains_key("limit-sglang-qwen-35b"));
        assert!(config.nodes.contains_key("limit-sglang-qwen-122b"));

        match config.nodes.get("limit-sglang-qwen-35b").unwrap() {
            Node::Virtual(virtual_node) => {
                assert_eq!(virtual_node.concurrency(), Some(8));
                assert_eq!(virtual_node.sequence(), &["sglang-qwen3.5-35b-a3b"]);
            }
            _ => panic!("Expected Virtual node for limit-sglang-qwen-35b"),
        }

        match config.nodes.get("limit-sglang-qwen-122b").unwrap() {
            Node::Virtual(virtual_node) => {
                assert_eq!(virtual_node.concurrency(), Some(8));
                assert_eq!(virtual_node.sequence(), &["sglang-qwen3.5-122b-a10b"]);
            }
            _ => panic!("Expected Virtual node for limit-sglang-qwen-122b"),
        }

        // 验证模型路由节点 (sequence)
        match config.nodes.get("qwen3.5-35b-a3b").unwrap() {
            Node::Virtual(virtual_node) => {
                assert_eq!(
                    virtual_node.sequence(),
                    &["limit-sglang-qwen-35b", "aliyun"]
                );
            }
            _ => panic!("Expected Virtual node"),
        }

        match config.nodes.get("qwen3.5-122b-a10b").unwrap() {
            Node::Virtual(virtual_node) => {
                assert_eq!(
                    virtual_node.sequence(),
                    &["limit-sglang-qwen-122b", "aliyun"]
                );
            }
            _ => panic!("Expected Virtual node"),
        }

        // 验证后端节点
        assert!(config.nodes.contains_key("sglang-qwen3.5-35b-a3b"));
        assert!(config.nodes.contains_key("sglang-qwen3.5-122b-a10b"));
        assert!(config.nodes.contains_key("qwen3.5-35b-a3b-no-mtp"));
        assert!(config.nodes.contains_key("aliyun"));

        // 验证 admin 配置（端口设置为 8080）
        assert!(config.admin.is_some(), "Admin config should be enabled");
        let admin = config.admin.as_ref().unwrap();
        assert_eq!(admin.port, 8080, "Admin port should be 8080");
        assert!(admin.auth_token.is_none());

        println!("Config validation passed!");
    }

    /// 测试 [admin] 配置解析 - 空配置应使用默认随机端口
    #[test]
    fn test_admin_config_empty_uses_default() {
        let toml_str = r#"
[input.service]
port = 8000
models = ["test-model"]

[admin]
"#;
        let config = GatewayConfig::from_str(toml_str).expect("Failed to parse config");

        // [admin] 存在，应该启用 admin 服务
        assert!(config.admin.is_some());
        let admin = config.admin.unwrap();
        // 默认端口为 0（随机端口）
        assert_eq!(admin.port, 0);
        assert!(admin.auth_token.is_none());
    }

    /// 测试 [admin] 配置解析 - 显式设置端口
    #[test]
    fn test_admin_config_with_port() {
        let toml_str = r#"
[input.service]
port = 8000
models = ["test-model"]

[admin]
port = 9090
auth-token = "secret123"
"#;
        let config = GatewayConfig::from_str(toml_str).expect("Failed to parse config");

        assert!(config.admin.is_some());
        let admin = config.admin.unwrap();
        assert_eq!(admin.port, 9090);
        assert_eq!(admin.auth_token, Some("secret123".to_string()));
    }

    /// 测试 [admin] 不存在时返回 None
    #[test]
    fn test_admin_config_missing() {
        let toml_str = r#"
[input.service]
port = 8000
models = ["test-model"]
"#;
        let config = GatewayConfig::from_str(toml_str).expect("Failed to parse config");

        assert!(config.admin.is_none());
    }

    /// 测试后端节点连接超时配置解析
    #[test]
    fn test_parse_backend_connect_timeout() {
        let toml_str = r#"
[backend.test-backend]
base-url = "http://localhost:8001"
connect-timeout-ms = 1500
"#;
        let config = GatewayConfig::from_str(toml_str).unwrap();
        match &config.nodes["test-backend"] {
            Node::Backend(backend) => assert_eq!(backend.connect_timeout_ms, Some(1500)),
            _ => panic!("Expected Backend node"),
        }
    }

    /// 测试后端节点默认超时值（未指定时为 None）
    #[test]
    fn test_parse_backend_timeout_defaults() {
        let toml_str = r#"
[backend.test-backend]
base-url = "http://localhost:8001"
"#;
        let config = GatewayConfig::from_str(toml_str).unwrap();
        match &config.nodes["test-backend"] {
            Node::Backend(backend) => assert_eq!(backend.connect_timeout_ms, None),
            _ => panic!("Expected Backend node"),
        }
    }
}
