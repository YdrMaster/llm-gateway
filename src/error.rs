//! 网关错误类型定义
//!
//! 包含网关运行过程中可能出现的各种错误类型

use thiserror::Error;

#[derive(Debug, Error)]
pub enum GatewayError {
    /// 未知协议
    #[error("Unknown protocol")]
    UnknownProtocol,

    /// 模型未找到
    #[error("Model not found: {0}")]
    ModelNotFound(String),

    /// 节点未找到
    #[error("Node not found: {0}")]
    NodeNotFound(String),

    /// 没有可用的后端
    #[error("No available backend")]
    NoAvailableBackend,

    /// 后端请求失败
    #[error("Backend request failed: {0}")]
    BackendRequestFailed(String),

    /// 协议转换失败
    #[error("Protocol conversion failed: {0}")]
    ProtocolConversionFailed(String),

    /// HTTP 错误
    #[error("HTTP error: {0}")]
    HttpError(#[from] hyper::Error),

    /// JSON 解析错误
    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),

    /// IO 错误
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = GatewayError::UnknownProtocol;
        assert_eq!(format!("{err}"), "Unknown protocol");

        let err = GatewayError::ModelNotFound("gpt-4".to_string());
        assert_eq!(format!("{err}"), "Model not found: gpt-4");
    }
}
