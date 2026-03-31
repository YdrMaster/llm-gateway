//! 后端节点

use crate::{
    Backend, Node, Route, RouteError, RoutePayload, RouteResult, health_monitor::HealthMonitor,
};
use llm_gateway_config::{BaseUrl, UrlResult};
use llm_gateway_protocols::Protocol;
use std::{collections::HashMap, sync::Arc};

pub(crate) struct BackendNode {
    pub(super) name: Arc<str>,
    pub(super) base_url: BaseUrl,
    pub(super) api_key: Option<String>,
    pub(super) health: Arc<HealthMonitor>,
}

impl Node for BackendNode {
    fn name(&self) -> &str {
        &self.name
    }

    fn route(&self, payload: &RoutePayload) -> RouteResult {
        // Check health availability first
        if !self.health.is_available() {
            return Err(RouteError::NoAvailable);
        }

        let protocol = payload.protocol();
        match self.base_url.get(protocol.name()) {
            UrlResult::Native(url) => Ok(Route {
                nodes: vec![],
                backend: Backend {
                    protocol,
                    base_url: url.into(),
                    api_key: self.api_key.clone(),
                },
            }),
            UrlResult::Foreign(protocol, url) => Ok(Route {
                nodes: vec![],
                backend: Backend {
                    protocol: match protocol {
                        "openai" => Protocol::OpenAI,
                        "anthropic" => Protocol::Anthropic,
                        _ => unreachable!(),
                    },
                    base_url: url.into(),
                    api_key: self.api_key.clone(),
                },
            }),
            UrlResult::Empty => Err(RouteError::NoAvailable),
        }
    }

    fn replace_connections(&self, _: &HashMap<&str, Arc<dyn Node>>) {}

    fn health(&self) -> Option<&Arc<HealthMonitor>> {
        Some(&self.health)
    }
}
