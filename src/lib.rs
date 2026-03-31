//! LLM Gateway 核心库
//!
//! 提供请求路由、协议转换和统计功能的核心模块

mod api;
mod backend_node;
mod error;
mod health_monitor;
mod input_node;
mod sequence_node;
mod serve;

#[macro_use]
extern crate log;

pub use api::admin::AdminServer;
pub use error::GatewayError;
pub use input_node::InputNode;
pub use serve::serve;
// 统计模块重新导出
pub use llm_gateway_statistics::{
    AggQuery, AggStats, EventFilter, RoutingEvent, StatisticsConfig, StatsQueryBuilder,
    StatsStoreManager, parse_time,
};

use backend_node::BackendNode;
use health_monitor::HealthMonitor;
use http::{Request, request};
use http_body_util::BodyExt;
use hyper::body::Incoming;
use llm_gateway_config::{GatewayConfig, VirtualNode};
use llm_gateway_protocols::Protocol;
use sequence_node::SequenceNode;
use serde_json::Value as Json;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// 路由负载 - 在路由图中流动的请求上下文
#[derive(Clone)]
pub struct RoutePayload {
    pub protocol: Protocol,
    pub parts: request::Parts,
    /// 请求体（已解析为 JSON）
    pub body: Json,
}

impl RoutePayload {
    /// 从 HTTP 请求创建 RoutePayload，解析请求体为 JSON
    pub async fn new(req: Request<Incoming>) -> Result<Self, GatewayError> {
        let (parts, incoming) = req.into_parts();
        let body = incoming.collect().await?;
        Ok(Self {
            protocol: Protocol::from_path(parts.uri.path()).ok_or(GatewayError::UnknownProtocol)?,
            parts,
            body: serde_json::from_slice(&body.to_bytes())?,
        })
    }

    /// 根据请求路径判断使用的协议
    const fn protocol(&self) -> Protocol {
        self.protocol
    }

    /// 根据请求路径判断使用的协议
    fn get_model(&self) -> &str {
        self.body
            .get("model")
            .and_then(Json::as_str)
            .unwrap_or("missing field")
    }
}

/// 路由结果
pub type RouteResult = Result<Route, RouteError>;

/// 路由结果，包含路由路径和后端信息
pub struct Route {
    /// 倒序：backend -> #n -> #n-1 -> ... -> model
    pub nodes: Vec<Arc<dyn Node>>,
    pub backend: Backend,
}

impl Route {
    pub fn model_name(&self) -> &str {
        self.nodes.last().map(|n| n.name()).unwrap_or("no model")
    }

    pub fn backend_name(&self) -> &str {
        self.nodes.first().map(|n| n.name()).unwrap_or("no backend")
    }
}

/// 后端配置信息
pub struct Backend {
    /// 使用的协议
    protocol: Protocol,
    /// 基础 URL
    base_url: String,
    /// API 密钥（可选）
    api_key: Option<String>,
}

/// 路由错误类型
pub enum RouteError {
    /// 没有可用的后端
    NoAvailable,
}

/// 节点 Trait - 网关路由图中的节点接口
pub trait Node: Send + Sync {
    /// 获取节点名称
    fn name(&self) -> &str;
    /// 执行路由逻辑，返回路由结果
    fn route(&self, payload: &RoutePayload) -> RouteResult;
    /// 替换节点间的连接关系
    fn replace_connections(&self, nodes: &HashMap<&str, Arc<dyn Node>>);
    /// 获取健康监控器（仅 BackendNode 有实现）
    fn health(&self) -> Option<&Arc<HealthMonitor>> {
        None
    }
}

/// 根据配置构建网关节点图
///
/// 从配置文件中读取节点定义，创建节点实例并建立节点间的连接关系
pub fn build(config: &GatewayConfig) -> Vec<Arc<InputNode>> {
    use llm_gateway_config::Node as ConfigNode;

    /// 占位节点，用于在连接建立前保存节点名称引用
    struct PlaceHolder(Arc<str>);

    impl Node for PlaceHolder {
        fn name(&self) -> &str {
            &self.0
        }
        fn route(&self, _: &RoutePayload) -> RouteResult {
            unimplemented!()
        }
        fn replace_connections(&self, _: &HashMap<&str, Arc<dyn Node>>) {
            unimplemented!()
        }
    }

    // 获取健康监控配置，若未指定则使用默认值
    let health_config = config
        .health
        .as_ref()
        .map(|h| h.to_internal())
        .unwrap_or_else(|| llm_gateway_config::HealthConfig::default().to_internal());

    let mut ans = Vec::new();
    let mut nodes: HashMap<&str, Arc<dyn Node>> = HashMap::new();
    for (name, node) in &config.nodes {
        match node {
            ConfigNode::Input(n) => ans.push(Arc::new(InputNode {
                name: name.clone(),
                port: n.port,
                models: RwLock::new(
                    n.models
                        .iter()
                        .map(|name| {
                            let name: Arc<str> = name.as_str().into();
                            (name.clone(), Arc::new(PlaceHolder(name)) as _)
                        })
                        .collect(),
                ),
                alias: n.alias.clone(),
            })),
            ConfigNode::Virtual(n) => match n {
                VirtualNode::Sequence(successors) => {
                    nodes.insert(
                        &**name,
                        Arc::new(SequenceNode {
                            name: name.clone(),
                            successors: RwLock::new(
                                successors
                                    .iter()
                                    .map(|name| Arc::new(PlaceHolder(name.as_str().into())) as _)
                                    .collect(),
                            ),
                        }) as _,
                    );
                }
            },
            ConfigNode::Backend(n) => {
                // 为后端节点创建健康监控器
                let health_monitor = Arc::new(HealthMonitor::new(health_config.clone()));
                nodes.insert(
                    &**name,
                    Arc::new(BackendNode {
                        name: name.clone(),
                        base_url: n.base_url.clone(),
                        api_key: n.api_key.clone(),
                        health: health_monitor,
                    }) as _,
                );
            }
        }
    }

    // 建立节点间的连接关系
    for node in nodes.values() {
        node.replace_connections(&nodes)
    }
    for input in &ans {
        input.replace_connections(&nodes)
    }

    ans
}
