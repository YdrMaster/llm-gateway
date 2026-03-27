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

mod backend;
mod error;
mod health;
mod input;
mod node;

pub use backend::{BackendNode, BaseUrl, UrlResult};
pub use error::Error as ConfigParseError;
pub use health::{HealthConfig, InternalHealthConfig};
pub use input::InputNode;
pub use node::{Node, VirtualNode};
use std::{collections::HashMap, str::FromStr, sync::Arc};

/// 网关配置根结构
///
/// 包含所有节点的映射，节点名称作为 key
#[derive(Clone, Debug)]
pub struct GatewayConfig {
    pub nodes: HashMap<Arc<str>, Node>,
    pub statistics: Option<llm_gateway_statistics::StatisticsConfig>,
    pub health: Option<HealthConfig>,
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

        let mut nodes: HashMap<Arc<str>, Node> = HashMap::new();

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
                                .collect()
                        })
                        .unwrap_or_default();

                    nodes.insert(name.clone().into(), Node::Input(InputNode { port, models }));
                }
            }
        }

        // 解析虚拟节点 [node.*]
        // 虚拟节点定义路由策略，目前支持 sequence（顺序尝试）
        if let Some(node_table) = root_table.get("node").and_then(|v| v.as_table()) {
            for (name, value) in node_table {
                // 检查节点名称是否重复
                if nodes.contains_key(name.as_str()) {
                    return Err(ConfigParseError::DuplicateName(name.clone()));
                }

                // 使用 let-chain 语法解析 sequence 字段
                if let Some(node_config) = value.as_table()
                    && let Some(sequence) = node_config.get("sequence").and_then(|v| v.as_array())
                {
                    let seq = sequence
                        .iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect();

                    nodes.insert(
                        name.clone().into(),
                        Node::Virtual(VirtualNode::Sequence(seq)),
                    );
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
                        name.clone().into(),
                        Node::Backend(BackendNode {
                            base_url,
                            api_key: None,
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

                    nodes.insert(
                        name.clone().into(),
                        Node::Backend(BackendNode { base_url, api_key }),
                    );
                }
            }
        }

        Ok(GatewayConfig {
            nodes,
            statistics,
            health,
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
}
