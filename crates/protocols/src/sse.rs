//! SSE (Server-Sent Events) 解析和序列化模块
//!
//! 提供 SSE 流式数据的解析和处理功能

use log::error;
use memchr::memmem::Finder;
use serde_json::Value as Json;
use std::{error, fmt, sync::OnceLock};

/// SSE 错误类型
#[derive(Debug, Clone, PartialEq)]
pub enum SseError {
    /// 流中的无效 UTF-8 序列
    InvalidUtf8,
    /// 未知的 SSE 行类型
    UnknownLineType(String),
    /// 缓冲区处理错误
    BufferError(String),
}

impl fmt::Display for SseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUtf8 => write!(f, "Invalid UTF-8 sequence in SSE stream"),
            Self::UnknownLineType(line) => write!(f, "Unknown SSE line type: {line}"),
            Self::BufferError(msg) => write!(f, "Buffer error: {msg}"),
        }
    }
}

impl error::Error for SseError {}

/// SSE 操作结果类型
pub type SseResult<T> = Result<T, SseError>;

/// 解析后的 SSE 消息
#[derive(Debug, Clone)]
pub struct SseMessage {
    /// 消息数据
    pub data: String,
    /// 事件类型
    pub event: Option<String>,
}

impl SseMessage {
    /// 创建只有数据的 SSE 消息
    pub fn new(data: &Json) -> Self {
        Self {
            data: serde_json::to_string(data).unwrap(),
            event: None,
        }
    }

    /// 创建带事件类型和数据的 SSE 消息
    pub fn with_event(event: impl Into<String>, data: &Json) -> Self {
        Self {
            data: serde_json::to_string(data).unwrap(),
            event: Some(event.into()),
        }
    }

    /// 创建流结束消息
    pub fn done() -> Self {
        Self {
            data: "[DONE]".into(),
            event: None,
        }
    }

    /// 判断是否是结束消息
    pub fn is_done(&self) -> bool {
        self.data == "[DONE]" && self.event.is_none()
    }

    /// 判断是否为空消息
    pub fn is_empty(&self) -> bool {
        self.data.trim().is_empty() && self.event.is_none()
    }
}

impl fmt::Display for SseMessage {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // event: xxx (if present)
        if let Some(event) = &self.event {
            writeln!(f, "event: {event}")?
        }
        // data: {...}
        write!(f, "data: {}\n\n", self.data)
    }
}

/// SSE 流收集器，解析原始字节为 SseMessage 结构
#[derive(Debug, Default)]
pub struct SseCollector {
    /// 不完整 SSE 消息的缓冲区
    buffer: Vec<u8>,
}

impl SseCollector {
    /// 创建新的 SSE 收集器
    pub fn new() -> Self {
        Default::default()
    }

    /// 从 HTTP 流收集字节并解析完整的 SSE 消息
    ///
    /// # 参数
    /// * `bytes` - HTTP 流的原始字节
    ///
    /// # 返回值
    /// 解析后的 `SseMessage` 向量（如果没有完整消息则为空），或解析失败时的错误
    pub fn collect(&mut self, bytes: &[u8]) -> SseResult<Vec<SseMessage>> {
        static FINDER: OnceLock<Finder> = OnceLock::new();
        let finder = FINDER.get_or_init(|| Finder::new(b"\n\n"));

        self.buffer.extend_from_slice(bytes);

        let mut ans = Vec::new();

        // 搜索完整消息（以 \n\n 分隔）
        while let Some(pos) = finder.find(&self.buffer) {
            // 提取一条完整的 SSE 消息
            let tail = self.buffer.split_off(pos + 2);
            let mut msg = std::mem::replace(&mut self.buffer, tail);
            msg.truncate(msg.len() - 2);
            let msg = String::from_utf8(msg).map_err(|_| SseError::InvalidUtf8)?;

            // 解析消息
            if let Some(message) = self.parse_message(&msg)? {
                ans.push(message)
            }
        }

        Ok(ans)
    }

    /// 处理流结束时缓冲区中剩余的数据
    ///
    /// 当 SSE 流完成时应调用此方法，处理可能仍在缓冲区中的不完整消息数据
    pub fn finish(&mut self) -> SseResult<Option<SseMessage>> {
        let msg = std::mem::take(&mut self.buffer);
        if msg.is_empty() {
            return Ok(None);
        }
        let msg = String::from_utf8(msg).map_err(|_| SseError::InvalidUtf8)?;
        self.parse_message(&msg)
    }

    /// 解析单条 SSE 消息（\n\n 分隔的单个块）
    fn parse_message(&self, message: &str) -> SseResult<Option<SseMessage>> {
        let mut ans = SseMessage {
            data: String::new(),
            event: None,
        };

        for line in message.lines() {
            if let Some(data) = line.strip_prefix("data:") {
                if !ans.data.is_empty() {
                    ans.data.push('\n')
                }
                ans.data.push_str(data.trim())
            } else if let Some(event) = line.strip_prefix("event:") {
                ans.event = Some(event.trim().into())
            } else if line.is_empty() {
                // 空行，忽略
            } else if line.starts_with(':') {
                // 注释行，忽略
            } else {
                return Err(SseError::UnknownLineType(line.to_string()));
            }
        }

        Ok(Some(ans).filter(|msg| !msg.is_empty()))
    }
}

impl Drop for SseCollector {
    fn drop(&mut self) {
        if !self.buffer.is_empty() {
            error!(
                "SSE Collector dropped not empty: {}",
                String::from_utf8_lossy(&self.buffer)
            )
        }
    }
}
