//! 并发控制节点模块
//!
//! 实现并发连接数限制的虚拟节点，仅负责并发控制，将请求转发给单一后继

use crate::{Node, RouteError, RouteGuard, RoutePayload, RouteResult};
use std::collections::HashMap;
use std::sync::RwLock;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

/// 并发控制节点结构
#[derive(Clone)]
pub struct ConcurrencyNode(Arc<Internal>);

/// 并发控制守卫，RAII 模式管理并发计数
pub struct ConcurrencyGuard(ConcurrencyNode);

struct Internal {
    /// 节点名称
    name: String,
    /// 最大并发数阈值
    max: usize,
    /// 当前并发计数（原子操作）
    current: AtomicUsize,
    /// 单一后继节点
    successor: RwLock<Arc<dyn Node>>,
}

impl ConcurrencyGuard {
    fn new(node: &ConcurrencyNode) -> Option<Self> {
        let Internal {
            name, max, current, ..
        } = &*node.0;

        let current_ = current.fetch_add(1, Ordering::AcqRel) + 1;
        if current_ <= *max {
            Some(Self(node.clone()))
        } else {
            warn!("Concurrency limit exceeded ({current_}/{max}) for node '{name}'");
            current.fetch_sub(1, Ordering::Release);
            None
        }
    }
}

impl Drop for ConcurrencyGuard {
    fn drop(&mut self) {
        self.0.0.current.fetch_sub(1, Ordering::Release);
    }
}

impl RouteGuard for ConcurrencyGuard {
    fn node(&self) -> &dyn Node {
        &self.0 as _
    }
}

impl ConcurrencyNode {
    /// 创建新的并发控制节点
    pub fn new(name: String, max: usize, successor: Arc<dyn Node>) -> Self {
        Self(Arc::new(Internal {
            name,
            max,
            current: AtomicUsize::new(0),
            successor: RwLock::new(successor),
        }))
    }
}

impl Node for ConcurrencyNode {
    fn name(&self) -> &str {
        &self.0.name
    }

    fn route(&self, payload: &RoutePayload) -> RouteResult {
        match ConcurrencyGuard::new(self) {
            Some(guard) => {
                self.0
                    .successor
                    .read()
                    .unwrap()
                    .route(payload)
                    .map(|mut route| {
                        // 成功：将 Guard 添加到路由
                        route.guards.push(Box::new(guard));
                        route
                    })
            }
            None => Err(RouteError::OverConcurrency),
        }
    }

    fn replace_connections(&self, nodes: &HashMap<&str, Arc<dyn Node>>) {
        // 先获取后继节点名称（读锁）
        let successor_name = self.0.successor.read().unwrap().name().to_string();
        // 再获取写锁更新后继节点
        if let Some(new_successor) = nodes.get(&*successor_name) {
            *self.0.successor.write().unwrap() = new_successor.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 虚拟节点用于测试
    struct DummyNode;
    impl Node for DummyNode {
        fn name(&self) -> &str {
            "dummy"
        }
        fn route(&self, _: &RoutePayload) -> RouteResult {
            unimplemented!()
        }
        fn replace_connections(&self, _: &HashMap<&str, Arc<dyn Node>>) {}
    }

    /// 测试并发计数正确性
    #[test]
    fn test_concurrent_counting() {
        let dummy = Arc::new(DummyNode);
        let node = ConcurrencyNode::new("test".into(), 2, dummy);

        // 初始状态应该为 0
        assert_eq!(node.0.current.load(Ordering::Relaxed), 0);

        // 手动模拟 acquire 操作
        let mut current = node.0.current.load(Ordering::Relaxed);
        loop {
            match node.0.current.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => current = actual,
            }
        }
        assert_eq!(node.0.current.load(Ordering::Relaxed), 1);

        // 创建 Guard（不增加计数，只负责释放）
        let guard1 = ConcurrencyGuard(node.clone());

        // 再次手动模拟 acquire
        current = node.0.current.load(Ordering::Relaxed);
        loop {
            match node.0.current.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => current = actual,
            }
        }
        assert_eq!(node.0.current.load(Ordering::Relaxed), 2);

        let guard2 = ConcurrencyGuard(node.clone());

        // 释放第一个
        drop(guard1);
        assert_eq!(node.0.current.load(Ordering::Relaxed), 1);

        // 释放第二个
        drop(guard2);
        assert_eq!(node.0.current.load(Ordering::Relaxed), 0);
    }
}
