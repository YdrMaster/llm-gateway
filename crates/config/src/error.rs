//! 配置解析错误类型
//!
//! 提供详细的错误信息，包括错误类型和发生错误的路径，
//! 帮助用户快速定位配置问题。

use std::{error, fmt};

/// 配置解析错误类型
///
/// 包含错误类型和发生错误的路径，帮助用户快速定位问题
///
/// # 变体
///
/// * [`ParseError`](Error::ParseError) - TOML 语法解析错误
/// * [`MissingField`](Error::MissingField) - 缺少必需字段
/// * [`DuplicateName`](Error::DuplicateName) - 节点名称重复
#[derive(Clone, Debug)]
pub enum Error {
    /// TOML 解析错误
    ParseError(String),
    /// 缺少必需字段，包含字段名和路径
    MissingField(String, String),
    /// 重复的节点名称
    DuplicateName(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::ParseError(msg) => write!(f, "TOML parse error: {msg}"),
            Error::MissingField(field, path) => {
                write!(f, "Missing required field '{field}': [{path}]")
            }
            Error::DuplicateName(name) => write!(f, "Duplicate node name: {name}"),
        }
    }
}

impl error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GatewayConfig;
    use std::str::FromStr;

    /// 测试缺少 port 字段时的错误信息
    #[test]
    fn test_error_on_missing_port() {
        let toml_str = r#"
[input.service]
models = ["model1"]
"#;
        let result = GatewayConfig::from_str(toml_str);
        assert!(result.is_err());

        match result.unwrap_err() {
            Error::MissingField(field, path) => {
                assert_eq!(field, "port");
                assert!(path.contains("input"));
            }
            _ => panic!("Expected MissingField error"),
        }
    }

    /// 测试缺少 base-url 字段时的错误信息
    #[test]
    fn test_error_on_missing_base_url() {
        let toml_str = r#"
[backend.test]
api-key = "test-key"
"#;
        let result = GatewayConfig::from_str(toml_str);
        assert!(result.is_err());

        match result.unwrap_err() {
            Error::MissingField(field, path) => {
                assert_eq!(field, "base-url");
                assert!(path.contains("backend"));
            }
            _ => panic!("Expected MissingField error"),
        }
    }

    /// 测试无效 TOML 语法错误
    #[test]
    fn test_error_on_invalid_toml() {
        let toml_str = "invalid [ toml";
        let result = GatewayConfig::from_str(toml_str);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), Error::ParseError(_)));
    }

    /// 测试跨类型重名检测（input 和 backend 同名）
    #[test]
    fn test_error_on_duplicate_input_name() {
        let toml_str = r#"
[input.service]
port = 8000
models = ["model1"]

[backend.service]
base-url = "http://test.com"
"#;
        let result = GatewayConfig::from_str(toml_str);
        assert!(result.is_err());

        match result.unwrap_err() {
            Error::DuplicateName(name) => {
                assert_eq!(name, "service");
            }
            _ => panic!("Expected DuplicateName error"),
        }
    }

    /// 测试跨类型重名检测（node 和 backend 同名）
    #[test]
    fn test_error_on_duplicate_node_name() {
        let toml_str = r#"
[node."test"]
sequence = ["a", "b"]

[backend.test]
base-url = "http://test.com"
"#;
        let result = GatewayConfig::from_str(toml_str);
        assert!(result.is_err());

        match result.unwrap_err() {
            Error::DuplicateName(name) => {
                assert_eq!(name, "test");
            }
            _ => panic!("Expected DuplicateName error"),
        }
    }

    /// 测试两个不同的简单后端名字（验证正常解析）
    #[test]
    fn test_error_on_duplicate_backend_name() {
        let toml_str = r#"
[backend]
"test-1" = "http://test1.com"
"test-2" = "http://test2.com"
"#;
        let result = GatewayConfig::from_str(toml_str);
        assert!(result.is_ok());
        let config = result.unwrap();

        // 验证两个后端都被解析
        assert!(config.nodes.contains_key("test-1"));
        assert!(config.nodes.contains_key("test-2"));
    }

    /// 测试跨类型重名检测（通用测试）
    #[test]
    fn test_error_on_cross_type_duplicate_name() {
        let toml_str = r#"
[input.service]
port = 8000
models = ["model1"]

[backend.service]
base-url = "http://test.com"
"#;
        let result = GatewayConfig::from_str(toml_str);
        assert!(result.is_err());

        match result.unwrap_err() {
            Error::DuplicateName(name) => {
                assert_eq!(name, "service");
            }
            _ => panic!("Expected DuplicateName error"),
        }
    }
}
