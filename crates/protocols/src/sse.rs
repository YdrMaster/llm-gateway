//! SSE (Server-Sent Events) parsing and serialization
//!
//! This module provides types and utilities for handling SSE streams
//! according to the HTML5 Server-Sent Events specification.

use memchr::memmem::Finder;
use serde_json::Value as Json;
use std::{error, fmt, sync::OnceLock};

/// SSE-specific error type
#[derive(Debug, Clone, PartialEq)]
pub enum SseError {
    /// Invalid UTF-8 sequence in the stream
    InvalidUtf8,
    /// Unknown line type in SSE message
    UnknownLineType(String),
    /// Buffer processing error
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

/// Result type for SSE operations
pub type SseResult<T> = Result<T, SseError>;

/// A parsed SSE message
///
/// According to the SSE spec, a message consists of:
/// - `data`: The message data
/// - `event`: Optional event type (defaults to "message" if not present)
#[derive(Debug, Clone)]
pub struct SseMessage {
    /// Parsed JSON data
    pub data: String,
    /// Event type (e.g., "content_block_delta", "message_start")
    pub event: Option<String>,
}

impl SseMessage {
    /// Create a new SSE message with data only (no event type)
    pub fn new(data: &Json) -> Self {
        Self {
            data: serde_json::to_string(data).unwrap(),
            event: None,
        }
    }

    /// Create a new SSE message with event type and data
    pub fn with_event(event: impl Into<String>, data: &Json) -> Self {
        Self {
            data: serde_json::to_string(data).unwrap(),
            event: Some(event.into()),
        }
    }

    /// Create a new SSE message with event type and data
    pub fn done() -> Self {
        Self {
            data: "[DONE]".into(),
            event: None,
        }
    }

    pub fn is_done(&self) -> bool {
        self.data == "[DONE]" && self.event.is_none()
    }

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

/// SSE stream collector that parses raw bytes into SseMessage structs
#[derive(Debug, Default)]
pub struct SseCollector {
    /// Buffer for incomplete SSE messages
    buffer: Vec<u8>,
}

impl SseCollector {
    /// Create a new SSE collector
    pub fn new() -> Self {
        Default::default()
    }

    /// Collect bytes from HTTP stream and parse complete SSE messages
    ///
    /// # Arguments
    /// * `bytes` - Raw bytes from HTTP stream
    ///
    /// # Returns
    /// Result containing vector of parsed `SseMessage` events (may be empty if no complete messages),
    /// or an error if parsing fails
    pub fn collect(&mut self, bytes: &[u8]) -> SseResult<Vec<SseMessage>> {
        static FINDER: OnceLock<Finder> = OnceLock::new();
        let finder = FINDER.get_or_init(|| Finder::new(b"\n\n"));

        self.buffer.extend_from_slice(bytes);

        let mut ans = Vec::new();

        // 搜索
        while let Some(pos) = finder.find(&self.buffer) {
            // Extract one complete SSE message
            let tail = self.buffer.split_off(pos + 2);
            let mut msg = std::mem::replace(&mut self.buffer, tail);
            msg.truncate(msg.len() - 2);
            let msg = String::from_utf8(msg).map_err(|_| SseError::InvalidUtf8)?;

            // Parse the message
            if let Some(message) = self.parse_message(&msg)? {
                ans.push(message)
            }
        }

        Ok(ans)
    }

    /// Process any remaining data in the buffer when the stream ends
    ///
    /// This should be called when the SSE stream is complete to handle
    /// any incomplete message data that may still be in the buffer.
    ///
    /// # Returns
    /// Result containing the final parsed message (if any), or an error if parsing fails
    pub fn finish(&mut self) -> SseResult<Option<SseMessage>> {
        let msg = std::mem::take(&mut self.buffer);
        if msg.is_empty() {
            return Ok(None);
        }
        let msg = String::from_utf8(msg).map_err(|_| SseError::InvalidUtf8)?;
        self.parse_message(&msg)
    }

    /// Parse a single SSE message (one block between \n\n separators)
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
                // Empty line, ignore
            } else if line.starts_with(':') {
                // Comment line, ignore
            } else {
                return Err(SseError::UnknownLineType(line.to_string()));
            }
        }

        Ok(Some(ans).filter(|msg| !msg.is_empty()))
    }
}
