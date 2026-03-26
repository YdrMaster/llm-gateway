use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

/// 路由事件，记录每一次请求的路由过程
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RoutingEvent {
    /// 毫秒级 Unix 时间戳 (i64 for rusqlite)
    pub timestamp: i64,
    /// 请求来源 IPv4 地址 (u32 网络字节序)
    pub remote_addr: u32,
    /// 请求来源端口
    pub remote_port: u16,
    /// HTTP 方法
    pub method: String,
    /// 请求路径
    pub path: String,
    /// 输入节点端口
    pub input_port: u16,
    /// 请求的模型名称
    pub model: String,
    /// 路由路径，如 "input->qwen3.5-35b->sglang"
    pub routing_path: String,
    /// 最终后端节点
    pub backend: String,
    /// 是否成功
    pub success: bool,
    /// 总耗时（毫秒）
    pub duration_ms: i64,
    /// 错误类型（失败时）
    pub error_type: Option<String>,
    /// 请求体大小（字节）
    pub request_size: Option<i64>,
    /// 响应体大小（字节）
    pub response_size: Option<i64>,
}

/// 构建 `RoutingEvent` 的 Builder
///
/// # 示例
///
/// ```
/// use llm_gateway_statistics::RoutingEventBuilder;
///
/// let event = RoutingEventBuilder::new(1234567890000, 9000)
///     .remote_addr("192.168.1.1:12345".parse().unwrap())
///     .method("POST")
///     .path("/v1/chat/completions")
///     .model("qwen3.5-35b")
///     .routing_path("input->qwen3.5-35b->sglang")
///     .backend("sglang")
///     .success(true)
///     .duration_ms(150)
///     .build();
/// ```
pub struct RoutingEventBuilder(RoutingEvent);

impl RoutingEventBuilder {
    /// 创建新的 Builder，必需参数为时间戳和输入端口
    pub fn new(timestamp: i64, input_port: u16) -> Self {
        Self(RoutingEvent {
            timestamp,
            input_port,
            ..Default::default()
        })
    }

    /// 设置远程地址和端口（从 SocketAddr）
    pub fn remote_addr(mut self, addr: SocketAddr) -> Self {
        match addr {
            SocketAddr::V4(v4) => {
                self.0.remote_addr = u32::from_be_bytes(v4.ip().octets());
            }
            SocketAddr::V6(v6) => {
                self.0.remote_addr = v6
                    .ip()
                    .to_ipv4()
                    .map_or(0, |ipv4| u32::from_be_bytes(ipv4.octets()));
            }
        }
        self.0.remote_port = addr.port();
        self
    }

    /// 设置远程地址和端口（从 u32 和 u16）
    pub fn remote_addr_raw(mut self, addr: u32, port: u16) -> Self {
        self.0.remote_addr = addr;
        self.0.remote_port = port;
        self
    }

    /// 设置 HTTP 方法
    pub fn method(mut self, method: impl Into<String>) -> Self {
        self.0.method = method.into();
        self
    }

    /// 设置请求路径
    pub fn path(mut self, path: impl Into<String>) -> Self {
        self.0.path = path.into();
        self
    }

    /// 设置模型名称
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.0.model = model.into();
        self
    }

    /// 设置路由路径
    pub fn routing_path(mut self, path: impl Into<String>) -> Self {
        self.0.routing_path = path.into();
        self
    }

    /// 设置后端名称
    pub fn backend(mut self, backend: impl Into<String>) -> Self {
        self.0.backend = backend.into();
        self
    }

    /// 设置是否成功
    pub fn success(mut self, success: bool) -> Self {
        self.0.success = success;
        self
    }

    /// 设置持续时间（毫秒）
    pub fn duration_ms(mut self, duration_ms: i64) -> Self {
        self.0.duration_ms = duration_ms;
        self
    }

    /// 设置错误类型
    pub fn error_type(mut self, error: impl Into<String>) -> Self {
        self.0.error_type = Some(error.into());
        self
    }

    /// 设置请求和响应大小
    pub fn sizes(mut self, request: i64, response: i64) -> Self {
        self.0.request_size = Some(request);
        self.0.response_size = Some(response);
        self
    }

    /// 构建 `RoutingEvent`
    pub fn build(self) -> RoutingEvent {
        self.0
    }
}

impl RoutingEvent {
    /// 创建新的 Builder
    pub fn builder(timestamp: i64, input_port: u16) -> RoutingEventBuilder {
        RoutingEventBuilder::new(timestamp, input_port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_routing_event_builder() {
        let event = RoutingEvent::builder(1234567890000, 9000)
            .remote_addr_raw(0xC0A80101, 12345)
            .method("POST")
            .path("/v1/chat/completions")
            .model("qwen3.5-35b")
            .routing_path("input->qwen3.5-35b->sglang")
            .backend("sglang")
            .success(true)
            .duration_ms(150)
            .build();

        assert_eq!(event.timestamp, 1234567890000);
        assert_eq!(event.remote_addr, 0xC0A80101);
        assert_eq!(event.remote_port, 12345);
        assert_eq!(event.method, "POST");
        assert_eq!(event.path, "/v1/chat/completions");
        assert_eq!(event.input_port, 9000);
        assert_eq!(event.model, "qwen3.5-35b");
        assert_eq!(event.routing_path, "input->qwen3.5-35b->sglang");
        assert_eq!(event.backend, "sglang");
        assert!(event.success);
        assert_eq!(event.duration_ms, 150);
        assert!(event.error_type.is_none());
    }

    #[test]
    fn test_routing_event_from_addr() {
        let addr: std::net::SocketAddr = "192.168.1.1:12345".parse().unwrap();
        let event = RoutingEvent::builder(1234567890000, 9000)
            .remote_addr(addr)
            .method("POST")
            .path("/v1/chat/completions")
            .model("qwen3.5-35b")
            .routing_path("input->qwen3.5-35b")
            .backend("aliyun")
            .success(false)
            .duration_ms(5000)
            .build();

        assert_eq!(event.remote_addr, 0xC0A80101); // 192.168.1.1
        assert_eq!(event.remote_port, 12345);
        assert!(!event.success);
    }

    #[test]
    fn test_routing_event_with_error() {
        let event = RoutingEvent::builder(1234567890000, 9000)
            .remote_addr_raw(0xC0A80101, 12345)
            .method("POST")
            .path("/v1/chat/completions")
            .model("qwen3.5-35b")
            .routing_path("input->qwen3.5-35b")
            .backend("aliyun")
            .success(false)
            .duration_ms(5000)
            .error_type("Backend timeout")
            .build();

        assert!(!event.success);
        assert_eq!(event.error_type, Some("Backend timeout".to_string()));
    }
}
