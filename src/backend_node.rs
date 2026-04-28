//! 后端节点

use crate::{
    Backend, HttpsClient, Node, Route, RouteError, RoutePayload, RouteResult, SimpleGuard,
    health_monitor::HealthMonitor,
};
use llm_gateway_config::{BaseUrl, UrlResult};
use llm_gateway_protocols::Protocol;
use std::{collections::HashMap, sync::Arc};

#[derive(Clone)]
pub(crate) struct BackendNode(Arc<Internal>);

struct Internal {
    name: String,
    base_url: BaseUrl,
    api_key: Option<String>,
    health: Arc<HealthMonitor>,
    client: HttpsClient,
}

impl Node for BackendNode {
    fn name(&self) -> &str {
        &self.0.name
    }

    fn route(&self, payload: &RoutePayload) -> RouteResult {
        // Check health availability first
        if !self.0.health.is_available() {
            return Err(RouteError::NoAvailable);
        }

        let protocol = payload.protocol();
        match self.0.base_url.get(protocol.name()) {
            UrlResult::Native(url) => Ok(Route {
                guards: vec![Box::new(SimpleGuard(self.clone()))],
                backend: Backend {
                    protocol,
                    base_url: url.into(),
                    api_key: self.0.api_key.clone(),
                    client: self.0.client.clone(),
                },
            }),
            UrlResult::Foreign(protocol, url) => Ok(Route {
                guards: vec![Box::new(SimpleGuard(self.clone()))],
                backend: Backend {
                    protocol: match protocol {
                        "openai" => Protocol::OpenAI,
                        "anthropic" => Protocol::Anthropic,
                        _ => unreachable!(),
                    },
                    base_url: url.into(),
                    api_key: self.0.api_key.clone(),
                    client: self.0.client.clone(),
                },
            }),
            UrlResult::Empty => Err(RouteError::NoAvailable),
        }
    }

    fn replace_connections(&self, _: &HashMap<&str, Arc<dyn Node>>) {}

    fn health(&self) -> Option<&Arc<HealthMonitor>> {
        Some(&self.0.health)
    }
}

impl BackendNode {
    /// 创建新的后端节点
    pub(crate) fn new(
        name: String,
        base_url: BaseUrl,
        api_key: Option<String>,
        health: Arc<HealthMonitor>,
        client: HttpsClient,
    ) -> Self {
        Self(Arc::new(Internal {
            name,
            base_url,
            api_key,
            health,
            client,
        }))
    }
}
