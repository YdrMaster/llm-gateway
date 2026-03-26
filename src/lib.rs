use http::{Request, request};
use http_body_util::BodyExt;
use hyper::body::Incoming;
use llm_gateway_config::{GatewayConfig, VirtualNode};
use llm_gateway_protocols::Protocol;
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

mod backend_node;
mod error;
mod input_node;
mod sequence_node;
mod serve;

pub use error::GatewayError;
pub use input_node::InputNode;
pub use serve::serve;

// 统计模块重新导出
pub use llm_gateway_statistics::{
    AggQuery, AggStats, EventFilter, RoutingEvent, StatisticsConfig, StatsQueryBuilder,
    StatsStoreManager, TimeGranularity,
};

use crate::{backend_node::BackendNode, sequence_node::SequenceNode};

/// 路由负载 - 在路由图中流动的请求上下文
#[derive(Clone)]
pub struct RoutePayload {
    pub parts: request::Parts,
    /// 请求体（已解析为 JSON）
    pub body: Value,
}

impl RoutePayload {
    /// 创建新的 RoutePayload
    pub async fn new(req: Request<Incoming>) -> Result<Self, GatewayError> {
        let (parts, incoming) = req.into_parts();
        let body = incoming.collect().await?;
        Ok(Self {
            parts,
            body: serde_json::from_slice(&body.to_bytes())?,
        })
    }

    pub fn protocol(&self) -> Protocol {
        Protocol::from_path(self.parts.uri.path())
    }
}

/// 路由结果
pub type RouteResult = Result<Route, RouteError>;

pub struct Route {
    nodes: Vec<Arc<dyn Node>>,
    backend: Backend,
}

pub struct Backend {
    protocol: Protocol,
    base_url: String,
    api_key: Option<String>,
}

pub enum RouteError {
    NoAvailable,
}

/// 节点 Trait
pub trait Node: Send + Sync {
    /// 节点名字
    fn name(&self) -> &str;
    /// 执行路由
    fn route(&self, payload: &RoutePayload) -> RouteResult;
    /// 替换连接关系
    fn replace_connections(&self, nodes: &HashMap<&str, Arc<dyn Node>>);
}

pub fn build(config: &GatewayConfig) -> Vec<Arc<InputNode>> {
    use llm_gateway_config::Node as ConfigNode;

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
                nodes.insert(
                    &**name,
                    Arc::new(BackendNode {
                        name: name.clone(),
                        base_url: n.base_url.clone(),
                        api_key: n.api_key.clone(),
                    }) as _,
                );
            }
        }
    }

    for node in nodes.values() {
        node.replace_connections(&nodes)
    }
    for input in &ans {
        input.replace_connections(&nodes)
    }

    ans
}
