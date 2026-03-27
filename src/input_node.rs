//! 输入节点模块
//!
//! 实现网关的入口节点，负责接收客户端请求并根据模型名称路由到对应的后端

use crate::{Node, RouteError, RoutePayload, RouteResult};
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

/// 输入节点结构
///
/// 网关的入口点，维护模型名称到下游节点的映射
pub struct InputNode {
    /// 节点名称
    pub(super) name: Arc<str>,
    /// 监听端口
    pub(super) port: u16,
    /// 模型名称到下游节点的映射
    pub(super) models: RwLock<HashMap<Arc<str>, Arc<dyn Node>>>,
}

impl Node for InputNode {
    /// 获取节点名称
    fn name(&self) -> &str {
        &self.name
    }

    /// 根据请求中的模型名称路由到对应的下游节点
    fn route(&self, payload: &RoutePayload) -> RouteResult {
        let model = payload.body.get("model").and_then(Value::as_str).unwrap();
        match self.models.read().unwrap().get(model) {
            Some(node) => match node.route(payload) {
                Ok(mut route) => {
                    route.nodes.push(node.clone());
                    Ok(route)
                }
                Err(e) => Err(e),
            },
            None => Err(RouteError::NoAvailable),
        }
    }

    /// 替换模型名称对应的下游节点引用，建立完整的节点连接
    fn replace_connections(&self, nodes: &HashMap<&str, Arc<dyn Node>>) {
        for node in self.models.write().unwrap().values_mut() {
            *node = nodes
                .get(node.name())
                .unwrap_or_else(|| panic!("{}: successor {} not fount", self.name, node.name()))
                .clone()
        }
    }
}
