//! 后端节点定义
//!
//! 后端节点代表实际的服务实例，支持：
//! - 简单 URL 转发
//! - 协议特定的 URL 映射
//! - API Key 替换

use std::collections::HashMap;

/// 后端节点结构
///
/// # 字段
///
/// * `base_url` - 基础 URL 配置，支持多协议映射
/// * `api_key` - 可选的 API Key，用于替换请求中的密钥
/// * `connect_timeout_ms` - 连接超时（毫秒），默认 500
/// * `request_timeout_ms` - 请求超时（毫秒），预留字段本次不使用
#[derive(Clone, Debug)]
pub struct BackendNode {
    /// 基础 URL 配置
    pub base_url: BaseUrl,
    /// 可选的 API Key
    pub api_key: Option<String>,
    /// 连接超时（毫秒），默认 500
    pub connect_timeout_ms: Option<u64>,
}

/// 基础 URL 配置
///
/// 支持两种配置方式：
/// 1. 单一 URL：所有协议使用同一个 URL
/// 2. 多协议映射：不同协议使用不同的 URL
#[derive(Clone, Debug)]
pub enum BaseUrl {
    /// 多协议映射
    Multi(HashMap<String, String>),
    /// 单一 URL
    AllInOne(String),
}

/// URL 查询结果
pub enum UrlResult<'a> {
    /// 原生协议支持
    Native(&'a str),
    /// 需要协议转换
    Foreign(&'a str, &'a str),
    /// URL 为空
    Empty,
}

impl BaseUrl {
    /// 根据协议获取对应的 URL
    ///
    /// # 参数
    ///
    /// * `protocol` - 协议名称，如 "anthropic", "openai" 等
    ///
    /// # 返回值
    ///
    /// 返回对应的 URL 查询结果
    pub fn get(&self, protocol: &str) -> UrlResult<'_> {
        match self {
            Self::Multi(map) => map
                .get(protocol)
                .map(|s| UrlResult::Native(s.as_str()))
                .or_else(|| {
                    map.iter()
                        .next()
                        .map(|(protocol, url)| UrlResult::Foreign(protocol.as_str(), url.as_str()))
                })
                .unwrap_or(UrlResult::Empty),
            Self::AllInOne(url) => UrlResult::Native(url),
        }
    }
}
