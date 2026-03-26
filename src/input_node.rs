use crate::{Node, RouteError, RoutePayload, RouteResult};
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

pub struct InputNode {
    pub(super) name: Arc<str>,
    pub(super) port: u16,
    pub(super) models: RwLock<HashMap<Arc<str>, Arc<dyn Node>>>,
}

impl Node for InputNode {
    fn name(&self) -> &Arc<str> {
        &self.name
    }

    fn route(&self, payload: &RoutePayload) -> RouteResult {
        let model = payload.body.get("model").and_then(Value::as_str).unwrap();
        match self.models.read().unwrap().get(model) {
            Some(node) => match node.route(payload) {
                Ok(mut route) => {
                    route.nodes.push(self.name.clone());
                    Ok(route)
                }
                Err(e) => Err(e),
            },
            None => Err(RouteError::NoAvailable),
        }
    }

    fn replace_connections(&self, nodes: &HashMap<&str, Arc<dyn Node>>) {
        for node in self.models.write().unwrap().values_mut() {
            *node = nodes
                .get(&**node.name())
                .unwrap_or_else(|| panic!("{}: successor {} not fount", self.name, node.name()))
                .clone()
        }
    }
}
