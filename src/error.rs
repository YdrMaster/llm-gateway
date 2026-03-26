use thiserror::Error;

#[derive(Debug, Error)]
pub enum GatewayError {
    #[error("Unknown protocol")]
    UnknownProtocol,

    #[error("Model not found: {0}")]
    ModelNotFound(String),

    #[error("Node not found: {0}")]
    NodeNotFound(String),

    #[error("No available backend")]
    NoAvailableBackend,

    #[error("Backend request failed: {0}")]
    BackendRequestFailed(String),

    #[error("Protocol conversion failed: {0}")]
    ProtocolConversionFailed(String),

    #[error("HTTP error: {0}")]
    HttpError(#[from] hyper::Error),

    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),

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
