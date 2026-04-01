//! 序列节点模块
//!
//! 实现顺序路由策略，按顺序尝试每个后端直到成功

use crate::{Node, RouteError, RoutePayload, RouteResult, SimpleGuard};
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

/// 序列节点结构
///
/// 按顺序尝试多个后端节点，直到找到一个可用的
#[derive(Clone)]
pub(crate) struct SequenceNode(Arc<Internal>);

struct Internal {
    /// 节点名称
    name: String,
    /// 后继节点列表
    successors: RwLock<Vec<Arc<dyn Node>>>,
}

impl Node for SequenceNode {
    /// 获取节点名称
    fn name(&self) -> &str {
        &self.0.name
    }

    /// 顺序尝试每个后继节点，返回第一个成功的路由结果
    fn route(&self, payload: &RoutePayload) -> RouteResult {
        for successor in &*self.0.successors.read().unwrap() {
            match successor.route(payload) {
                Ok(mut route) => {
                    route.guards.push(Box::new(SimpleGuard(self.clone())));
                    return Ok(route);
                }
                Err(e) => match e {
                    RouteError::NoAvailable => {}
                    RouteError::OverConcurrency => {}
                },
            }
        }
        Err(RouteError::NoAvailable)
    }

    /// 替换后继节点的引用，建立完整的节点连接
    fn replace_connections(&self, nodes: &HashMap<&str, Arc<dyn Node>>) {
        let mut successors = self.0.successors.write().unwrap();
        for node in successors.iter_mut() {
            let name = node.name();
            *node = nodes.get(name).unwrap().clone();
        }
    }
}

impl SequenceNode {
    /// 创建新的序列节点
    pub(crate) fn new(name: String, successors: Vec<Arc<dyn Node>>) -> Self {
        Self(Arc::new(Internal {
            name,
            successors: RwLock::new(successors),
        }))
    }
}
