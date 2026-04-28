//! LLM Gateway 核心库
//!
//! 提供请求路由、协议转换和统计功能的核心模块

mod api;
mod backend_node;
mod concurrency_node;
mod error;
mod health_monitor;
mod input_node;
mod sequence_node;
mod serve;

#[macro_use]
extern crate log;

pub use api::admin::AdminServer;
pub use concurrency_node::{ConcurrencyGuard, ConcurrencyNode};
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
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper_rustls::HttpsConnector;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use llm_gateway_config::{GatewayConfig, HealthConfig, VirtualNode};
use llm_gateway_protocols::Protocol;
use sequence_node::SequenceNode;
use serde_json::Value as Json;
use std::collections::HashMap;
use std::sync::Arc;

use bytes::Bytes;
use serve::{DEFAULT_CONNECT_TIMEOUT_MS, create_client};

/// HTTPS 客户端的类型别名
pub(crate) type HttpsClient = Client<HttpsConnector<HttpConnector>, Full<Bytes>>;

/// 路由负载 - 在路由图中流动的请求上下文
#[derive(Clone)]
pub struct RoutePayload {
    /// 请求输入时使用的协议
    pub protocol: Protocol,
    /// 别名解析后，请求中实际使用的模型名称
    pub model: String,
    /// Http 信息
    pub parts: request::Parts,
    /// 请求体（已解析为 JSON）
    pub body: Json,
}

impl RoutePayload {
    /// 从 HTTP 请求创建 RoutePayload，解析请求体为 JSON
    pub async fn new(req: Request<Incoming>) -> Result<Self, GatewayError> {
        let (parts, incoming) = req.into_parts();
        let body = incoming.collect().await?;
        let body: Json = serde_json::from_slice(&body.to_bytes())?;

        let protocol =
            Protocol::from_path(parts.uri.path()).ok_or(GatewayError::UnknownProtocol)?;
        let model = body
            .get("model")
            .and_then(Json::as_str)
            .ok_or(GatewayError::MissingModelField)?
            .to_string();

        Ok(Self {
            protocol,
            model,
            parts,
            body,
        })
    }

    /// 根据请求路径判断使用的协议
    const fn protocol(&self) -> Protocol {
        self.protocol
    }

    /// 获取模型名称
    const fn model(&self) -> &str {
        self.model.as_str()
    }
}

/// 路由结果
pub type RouteResult = Result<Route, RouteError>;

/// 路由结果，包含路由路径和后端信息
pub struct Route {
    /// 路由路径上的节点守卫列表
    /// 倒序：backend -> #n -> #n-1 -> ... -> model
    pub guards: Vec<Box<dyn RouteGuard>>,
    pub backend: Backend,
}

impl Route {
    pub fn model_name(&self) -> &str {
        self.guards
            .last()
            .map(|n| n.node().name())
            .unwrap_or("no model")
    }

    pub fn backend_name(&self) -> &str {
        self.guards
            .first()
            .map(|n| n.node().name())
            .unwrap_or("no backend")
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
    /// 独立的 HTTP 客户端（带连接超时）
    client: HttpsClient,
}

/// 路由错误类型
#[derive(Debug)]
pub enum RouteError {
    /// 没有可用的后端
    NoAvailable,
    /// 并行度超限
    OverConcurrency,
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

/// 节点守卫 trait
///
/// 用于在 Route 中携带节点引用，同时通过 Drop 实现资源清理
/// 注意：此 trait 不继承 Clone，因此是对象安全的
pub trait RouteGuard: Send + Sync {
    /// 获取底层节点引用
    fn node(&self) -> &dyn Node;
}

/// 简单守卫，用于包装不需要特殊清理的节点
pub struct SimpleGuard<T>(T);

impl<T: Node> RouteGuard for SimpleGuard<T> {
    fn node(&self) -> &dyn Node {
        &self.0
    }
}

/// 根据配置构建网关节点图
///
/// 从配置文件中读取节点定义，创建节点实例并建立节点间的连接关系
pub fn build(config: &GatewayConfig) -> Vec<Arc<InputNode>> {
    use llm_gateway_config::Node as ConfigNode;
    use log::info;

    info!("Building node graph with {} nodes...", config.nodes.len());

    /// 占位节点，用于在连接建立前保存节点名称引用
    struct PlaceHolder(String);

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
        .map(HealthConfig::to_internal)
        .unwrap_or_else(|| HealthConfig::default().to_internal());

    let mut ans = Vec::new();
    let mut nodes: HashMap<&str, Arc<dyn Node>> = HashMap::new();
    // 存储需要设置后继的 ConcurrencyNode: (节点名称, 后继名称)
    for (name, node) in &config.nodes {
        match node {
            ConfigNode::Input(n) => {
                let models: HashMap<String, Arc<dyn Node>> = n
                    .models
                    .iter()
                    .map(|model_name| {
                        (
                            model_name.clone(),
                            Arc::new(PlaceHolder(model_name.clone())) as _,
                        )
                    })
                    .collect();
                ans.push(Arc::new(InputNode::new(
                    name.clone(),
                    n.port,
                    models,
                    n.alias.clone(),
                )))
            }
            ConfigNode::Virtual(n) => match n {
                VirtualNode::Sequence(successors) => {
                    let successors: Vec<Arc<dyn Node>> = successors
                        .iter()
                        .map(|name| Arc::new(PlaceHolder(name.clone())) as _)
                        .collect();
                    nodes.insert(
                        &**name,
                        Arc::new(SequenceNode::new(name.clone(), successors)) as _,
                    );
                }
                VirtualNode::Concurrency { max, successor } => {
                    let successor = PlaceHolder(successor.clone());
                    let node = ConcurrencyNode::new(name.clone(), *max, Arc::new(successor));
                    nodes.insert(&**name, Arc::new(node) as _);
                }
            },
            ConfigNode::Backend(n) => {
                // 为后端节点创建健康监控器
                let connect_timeout_ms = n.connect_timeout_ms.unwrap_or(DEFAULT_CONNECT_TIMEOUT_MS);
                let client = create_client(connect_timeout_ms);
                nodes.insert(
                    &**name,
                    Arc::new(BackendNode::new(
                        name.clone(),
                        n.base_url.clone(),
                        n.api_key.clone(),
                        Arc::new(HealthMonitor::new(health_config.clone())),
                        client,
                    )) as _,
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
