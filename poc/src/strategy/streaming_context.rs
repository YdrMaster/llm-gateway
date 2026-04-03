use llm_gateway_protocols::{SseCollector, streaming::StreamingCollector};

/// 为单个请求持有增量 SSE 解析器和协议转换器。
///
/// 每个上游 body 块的处理流水线：
///   原始字节 → SseCollector.collect() → Vec<SseMessage> → StreamingCollector.process() → Vec<SseMessage> → 输出字节
pub struct StreamingTransformCtx {
    /// 增量 SSE 解析器 — 缓冲未完成的行
    pub sse_collector: SseCollector,
    /// 协议转换器（如 AnthropicToOpenai）— 为 None 时表示直通
    pub converter: Option<Box<dyn StreamingCollector>>,
    /// 源协议（用于日志/调试）
    pub source_protocol: String,
    /// 目标协议
    pub target_protocol: String,
}

impl StreamingTransformCtx {
    pub fn new(
        source: String,
        target: String,
        converter: Option<Box<dyn StreamingCollector>>,
    ) -> Self {
        Self {
            sse_collector: SseCollector::default(),
            converter,
            source_protocol: source,
            target_protocol: target,
        }
    }

    /// 处理一块原始上游 body 字节。
    /// 返回要发送到下游的转换后字节（如果数据不足可能为空）。
    pub fn process_chunk(&mut self, chunk: &[u8]) -> Vec<u8> {
        // 将字节块送入 SSE 收集器
        let messages = match self.sse_collector.collect(chunk) {
            Ok(msgs) => msgs,
            Err(e) => {
                log::warn!("SSE collect error: {e}");
                return Vec::new();
            }
        };

        let mut output = Vec::new();

        for msg in messages {
            if let Some(converter) = self.converter.as_mut() {
                match converter.process(msg) {
                    Ok(converted) => {
                        for out_msg in converted {
                            // 格式：event: ...\ndata: ...\n\n （或仅 data: ...\n\n）
                            let formatted = format!("{out_msg}");
                            output.extend_from_slice(formatted.as_bytes());
                        }
                    }
                    Err(e) => log::warn!("Protocol conversion error: {e}"),
                }
            } else {
                // 直通：重新发出原始消息
                let formatted = format!("{msg}");
                output.extend_from_slice(formatted.as_bytes());
            }
        }

        output
    }

    /// 在流结束时排出任何剩余的 SSE 数据。
    pub fn finish(&mut self) -> Vec<u8> {
        let remaining = match self.sse_collector.finish() {
            Ok(Some(msg)) => vec![msg],
            Ok(None) => vec![],
            Err(e) => {
                log::warn!("SSE finish error: {e}");
                vec![]
            }
        };

        let mut output = Vec::new();
        for msg in remaining {
            if let Some(converter) = self.converter.as_mut() {
                if let Ok(converted) = converter.process(msg) {
                    for out_msg in converted {
                        output.extend_from_slice(format!("{out_msg}").as_bytes());
                    }
                }
            } else {
                output.extend_from_slice(format!("{msg}").as_bytes());
            }
        }

        output
    }
}
