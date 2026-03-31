//! LLM Gateway 协议转换模块
//!
//! 提供 OpenAI 和 Anthropic 协议之间的双向转换功能

mod functions;
mod sse;

pub use functions::request;
pub use functions::response;
pub use functions::streaming;
pub use functions::{ProtocolError, ProtocolResult};
pub use sse::{SseCollector, SseError, SseMessage, SseResult};

/// 支持的协议类型
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Protocol {
    /// OpenAI 协议
    OpenAI,
    /// Anthropic 协议
    Anthropic,
}

impl Protocol {
    /// 从协议名称创建
    pub fn from_name(name: &str) -> Self {
        match name {
            "openai" => Self::OpenAI,
            "anthropic" => Self::Anthropic,
            _ => panic!("Unknown protocol name {name}"),
        }
    }

    /// 获取协议名称
    pub const fn name(&self) -> &str {
        match self {
            Self::OpenAI => "openai",
            Self::Anthropic => "anthropic",
        }
    }

    /// 从请求路径创建
    pub fn from_path(path: &str) -> Option<Self> {
        match path {
            "/v1/chat/completions" => Some(Self::OpenAI),
            "/v1/messages" => Some(Self::Anthropic),
            _ => None,
        }
    }

    /// 获取请求路径
    pub const fn path(&self) -> &str {
        match self {
            Self::OpenAI => "/v1/chat/completions",
            Self::Anthropic => "/v1/messages",
        }
    }
}
