//! 协议转换中间件
//!
//! 复用 llm-gateway-protocols  crate 进行协议转换

use llm_gateway_protocols::{
    Protocol, SseMessage,
    streaming::{AnthropicToOpenai, OpenaiToAnthropic, StreamingCollector},
};
use log::debug;

/// 协议转换中间件
pub struct ProtocolConversionMiddleware;

impl ProtocolConversionMiddleware {
    /// 创建新的协议转换中间件
    pub fn new() -> Self {
        Self
    }

    /// 创建转换器
    pub fn create_converter(
        &self,
        from: Protocol,
        to: Protocol,
    ) -> Option<Box<dyn StreamingCollector>> {
        match (from, to) {
            (Protocol::Anthropic, Protocol::OpenAI) => {
                debug!("Creating Anthropic to OpenAI converter");
                Some(Box::new(AnthropicToOpenai::default()))
            }
            (Protocol::OpenAI, Protocol::Anthropic) => {
                debug!("Creating OpenAI to Anthropic converter");
                Some(Box::new(OpenaiToAnthropic::default()))
            }
            _ => {
                debug!("No conversion needed: {from:?} -> {to:?}");
                None
            }
        }
    }

    /// 处理 SSE 消息
    #[allow(dead_code)]
    pub fn process_message(
        &self,
        converter: &mut Box<dyn StreamingCollector>,
        message: SseMessage,
    ) -> Result<Vec<SseMessage>, llm_gateway_protocols::ProtocolError> {
        converter.process(message)
    }

    /// 检查是否需要转换
    #[allow(dead_code)]
    pub fn needs_conversion(&self, from: Protocol, to: Protocol) -> bool {
        from != to
    }
}

impl Default for ProtocolConversionMiddleware {
    fn default() -> Self {
        Self::new()
    }
}
