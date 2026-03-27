//! 序列节点模块
//!
//! 实现顺序路由策略，按顺序尝试每个后端直到成功

use crate::{Node, RouteError, RoutePayload, RouteResult};
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

/// 序列节点结构
///
/// 按顺序尝试多个后端节点，直到找到一个可用的
pub(crate) struct SequenceNode {
    /// 节点名称
    pub(super) name: Arc<str>,
    /// 后继节点列表
    pub(super) successors: RwLock<Vec<Arc<dyn Node>>>,
}

impl Node for SequenceNode {
    /// 获取节点名称
    fn name(&self) -> &str {
        &self.name
    }

    /// 顺序尝试每个后继节点，返回第一个成功的路由结果
    fn route(&self, payload: &RoutePayload) -> RouteResult {
        for successor in &*self.successors.read().unwrap() {
            match successor.route(payload) {
                Ok(mut route) => {
                    route.nodes.push(successor.clone());
                    return Ok(route);
                }
                Err(e) => match e {
                    RouteError::NoAvailable => {}
                },
            }
        }
        Err(RouteError::NoAvailable)
    }

    /// 替换后继节点的引用，建立完整的节点连接
    fn replace_connections(&self, nodes: &HashMap<&str, Arc<dyn Node>>) {
        for node in &mut *self.successors.write().unwrap() {
            *node = nodes.get(node.name()).unwrap().clone()
        }
    }
}
