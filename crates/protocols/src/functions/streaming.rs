use super::{ProtocolError, ProtocolResult};
use crate::SseMessage;
use serde_json::{Value as Json, json};

/// Streaming collector trait for protocol conversion
pub trait StreamingCollector: Send + Sync {
    /// Process an SSE message and return converted messages (if any)
    fn process(&mut self, msg: SseMessage) -> ProtocolResult<Option<Vec<SseMessage>>>;
}

/// OpenAI to Anthropic streaming converter
#[derive(Default)]
pub struct OpenaiToAnthropic {
    id: Option<String>,
    model: Option<String>,
    created: Option<u64>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    initialized: bool,
    finished: bool,
}

impl StreamingCollector for OpenaiToAnthropic {
    fn process(&mut self, msg: SseMessage) -> ProtocolResult<Option<Vec<SseMessage>>> {
        // Handle [DONE] marker - if already finished, return empty
        if msg.is_done() {
            if self.finished {
                // Already sent end-of-stream events via finish_reason, nothing to do
                return Ok(None);
            }
            // Otherwise generate end-of-stream events (e.g., when OpenAI stream ends without finish_reason)
            let mut ans = Vec::new();

            // Generate content_block_stop
            ans.push(SseMessage::with_event(
                "content_block_stop",
                &json!({
                    "type": "content_block_stop",
                    "index": 0
                }),
            ));

            // Generate message_delta with end_turn
            ans.push(SseMessage::with_event(
                "message_delta",
                &json!({
                    "type": "message_delta",
                    "delta": {
                        "stop_reason": "end_turn",
                        "stop_sequence": null,
                        "usage": {
                            "output_tokens": self.output_tokens.unwrap_or_default()
                        }
                    }
                }),
            ));

            // Generate message_stop
            ans.push(SseMessage::with_event(
                "message_stop",
                &json!({"type": "message_stop"}),
            ));

            self.finished = true;
            return Ok(Some(ans));
        }

        let chunk: Json = serde_json::from_str(&msg.data)?;

        let mut ans = Vec::new();
        let obj = chunk
            .as_object()
            .cloned()
            .ok_or_else(|| ProtocolError::InvalidRequest("Chunk must be an object".to_string()))?;

        // Initialize on first chunk
        if !self.initialized {
            if self.id.is_none()
                && let Some(id) = obj.get("id").and_then(|v| v.as_str())
            {
                self.id = Some(id.to_string());
            }
            if self.model.is_none()
                && let Some(model) = obj.get("model").and_then(|v| v.as_str())
            {
                self.model = Some(model.to_string());
            }
            if self.created.is_none()
                && let Some(created) = obj.get("created").and_then(|v| v.as_u64())
            {
                self.created = Some(created);
            }

            // Generate message_start event
            ans.push(SseMessage::with_event(
                "message_start",
                &self.create_message_start(),
            ));

            // Generate content_block_start event
            ans.push(SseMessage::with_event(
                "content_block_start",
                &self.create_content_block_start(0),
            ));

            self.initialized = true;
        }

        // Process choices
        if let Some(choices) = obj.get("choices").and_then(|v| v.as_array())
            && let Some(choice) = choices.first()
        {
            // Process delta
            if let Some(delta) = choice.get("delta") {
                // Handle content delta
                if let Some(content) = delta.get("content").and_then(|v| v.as_str())
                    && !content.is_empty()
                {
                    ans.push(SseMessage::with_event(
                        "content_block_delta",
                        &self.create_content_block_delta(0, content),
                    ));
                }

                // Handle reasoning content (thinking)
                if let Some(reasoning) = delta.get("reasoning_content").and_then(|v| v.as_str())
                    && !reasoning.is_empty()
                {
                    ans.push(SseMessage::with_event(
                        "content_block_delta",
                        &json!({
                            "type": "content_block_delta",
                            "index": 0,
                            "delta": {
                                "type": "thinking_delta",
                                "thinking": reasoning
                            }
                        }),
                    ));
                }

                // Handle tool calls
                if let Some(tool_calls) = delta.get("tool_calls").and_then(|v| v.as_array()) {
                    for (idx, tool_call) in tool_calls.iter().enumerate() {
                        if let Some(function) = tool_call.get("function") {
                            let tool_id = Self::extract_str(function, "id", "");
                            let name = Self::extract_str(function, "name", "");
                            let arguments = Self::extract_str(function, "arguments", "{}");

                            // Generate tool_use content_block_start
                            ans.push(SseMessage::with_event(
                                "content_block_start",
                                &self.create_tool_use_block_start(1 + idx, &tool_id, &name),
                            ));

                            // Generate tool_use input delta
                            ans.push(SseMessage::with_event(
                                "content_block_delta",
                                &self.create_input_json_delta(1 + idx, &arguments),
                            ));
                        }
                    }
                }
            }

            // Handle finish_reason
            if let Some(finish_reason) = choice.get("finish_reason").and_then(|v| v.as_str())
                && finish_reason != "null"
                && !finish_reason.is_empty()
            {
                // Generate content_block_stop
                ans.push(SseMessage::with_event(
                    "content_block_stop",
                    &json!({
                        "type": "content_block_stop",
                        "index": 0
                    }),
                ));

                // Generate message_delta
                ans.push(SseMessage::with_event(
                    "message_delta",
                    &self.create_message_delta(finish_reason),
                ));

                // Generate message_stop
                ans.push(SseMessage::with_event(
                    "message_stop",
                    &json!({"type": "message_stop"}),
                ));

                // Mark as finished so [DONE] won't generate duplicate events
                self.finished = true;
            }
        }

        // Process usage
        if let Some(usage) = obj.get("usage")
            && let Some(usage_obj) = usage.as_object()
        {
            if let Some(prompt_tokens) = usage_obj.get("prompt_tokens").and_then(|v| v.as_u64()) {
                self.input_tokens = Some(prompt_tokens);
            }
            if let Some(completion_tokens) =
                usage_obj.get("completion_tokens").and_then(|v| v.as_u64())
            {
                self.output_tokens = Some(completion_tokens);
            }
        }

        Ok(Some(ans))
    }
}

impl OpenaiToAnthropic {
    /// Extract a string field from JSON with default value
    fn extract_str(obj: &Json, key: &str, default: &str) -> String {
        obj.get(key)
            .and_then(|v| v.as_str())
            .unwrap_or(default)
            .into()
    }

    fn create_message_start(&self) -> Json {
        json!({
            "type": "message_start",
            "message": {
                "id": self.id.as_deref().unwrap_or(""),
                "type": "message",
                "role": "assistant",
                "model": self.model.as_deref().unwrap_or(""),
                "content": [],
                "stop_reason": null,
                "stop_sequence": null,
                "usage": {
                    "input_tokens": 0,
                    "output_tokens": 0
                }
            }
        })
    }

    fn create_content_block_start(&self, index: usize) -> Json {
        json!({
            "type": "content_block_start",
            "index": index,
            "content_block": {
                "type": "text",
                "text": ""
            }
        })
    }

    fn create_content_block_delta(&self, index: usize, text: &str) -> Json {
        json!({
            "type": "content_block_delta",
            "index": index,
            "delta": {
                "type": "text_delta",
                "text": text
            }
        })
    }

    fn create_tool_use_block_start(&self, index: usize, tool_id: &str, name: &str) -> Json {
        json!({
            "type": "content_block_start",
            "index": index,
            "content_block": {
                "type": "tool_use",
                "id": tool_id,
                "name": name,
                "input": {}
            }
        })
    }

    fn create_input_json_delta(&self, index: usize, partial_json: &str) -> Json {
        json!({
            "type": "content_block_delta",
            "index": index,
            "delta": {
                "type": "input_json_delta",
                "partial_json": partial_json
            }
        })
    }

    fn create_message_delta(&mut self, finish_reason: &str) -> Json {
        // Convert finish_reason
        let stop_reason = match finish_reason {
            "stop" => "end_turn",
            "length" => "max_tokens",
            "tool_calls" => "tool_use",
            "content_filter" => "refusal",
            _ => "end_turn",
        };

        json!({
            "type": "message_delta",
            "delta": {
                "stop_reason": stop_reason,
                "stop_sequence": null,
                "usage": {
                    "output_tokens": self.output_tokens.unwrap_or_default()
                }
            }
        })
    }
}

/// Anthropic to OpenAI streaming converter
#[derive(Default)]
pub struct AnthropicToOpenai {
    id: Option<String>,
    model: Option<String>,
    created: Option<u64>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    current_block_type: Option<String>,
    tool_id: Option<String>,
    tool_name: Option<String>,
    tool_input: String,
}

impl StreamingCollector for AnthropicToOpenai {
    fn process(&mut self, msg: SseMessage) -> ProtocolResult<Option<Vec<SseMessage>>> {
        let event: Json = serde_json::from_str(&msg.data)?;

        let obj = event.as_object().ok_or_else(|| {
            ProtocolError::InvalidStreamEvent("Event must be an object".to_string())
        })?;

        let event_type = obj
            .get("type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ProtocolError::InvalidStreamEvent("Event missing type".to_string()))?;

        match event_type {
            "ping" => Ok(None),
            "message_start" => self.handle_message_start(obj),
            "content_block_start" => self.handle_content_block_start(obj),
            "content_block_delta" => self.handle_content_block_delta(obj),
            "content_block_stop" => self.handle_content_block_stop(obj),
            "message_delta" => self.handle_message_delta(obj),
            "message_stop" => self.handle_message_stop(),
            _ => Err(ProtocolError::InvalidStreamEvent(format!(
                "Unknown event type: {event_type}"
            ))),
        }
    }
}

impl AnthropicToOpenai {
    fn handle_message_start(
        &mut self,
        obj: &serde_json::Map<String, Json>,
    ) -> ProtocolResult<Option<Vec<SseMessage>>> {
        if let Some(message) = obj.get("message").and_then(|v| v.as_object()) {
            if let Some(id) = message.get("id").and_then(|v| v.as_str()) {
                self.id = Some(id.to_string());
            }
            if let Some(model) = message.get("model").and_then(|v| v.as_str()) {
                self.model = Some(model.to_string());
            }
            // Set created timestamp
            self.created = Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
            );
            if let Some(usage) = message.get("usage").and_then(|v| v.as_object())
                && let Some(input_tokens) = usage.get("input_tokens").and_then(|v| v.as_u64())
            {
                self.input_tokens = Some(input_tokens);
            }
        }
        // No output for message_start
        Ok(None)
    }

    fn handle_content_block_start(
        &mut self,
        obj: &serde_json::Map<String, Json>,
    ) -> ProtocolResult<Option<Vec<SseMessage>>> {
        if let Some(content_block) = obj.get("content_block").and_then(|v| v.as_object()) {
            let block_type = content_block
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("text")
                .to_string();

            self.current_block_type = Some(block_type.clone());

            if block_type == "tool_use" {
                self.tool_id = content_block
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                self.tool_name = content_block
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                self.tool_input = String::new();
            }
        }
        // No output for content_block_start
        Ok(None)
    }

    fn handle_content_block_delta(
        &mut self,
        obj: &serde_json::Map<String, Json>,
    ) -> ProtocolResult<Option<Vec<SseMessage>>> {
        if let Some(delta) = obj.get("delta").and_then(|v| v.as_object()) {
            let delta_type = delta.get("type").and_then(|v| v.as_str()).unwrap_or("");

            match delta_type {
                "text_delta" => {
                    if let Some(text) = delta.get("text").and_then(|v| v.as_str()) {
                        // Generate OpenAI chunk with delta.content
                        return Ok(Some(vec![SseMessage::new(&self.create_chunk(
                            None,
                            Some(text),
                            None,
                            None,
                        ))]));
                    }
                }
                "thinking_delta" => {
                    if let Some(thinking) = delta.get("thinking").and_then(|v| v.as_str()) {
                        // Generate OpenAI chunk with reasoning_content
                        return Ok(Some(vec![SseMessage::new(&self.create_chunk(
                            None,
                            None,
                            None,
                            Some(thinking),
                        ))]));
                    }
                }
                "input_json_delta" => {
                    if let Some(partial_json) = delta.get("partial_json").and_then(|v| v.as_str()) {
                        // Accumulate tool input
                        self.tool_input.push_str(partial_json);
                    }
                    // No output for input_json_delta
                }
                _ => {}
            }
        }
        Ok(None)
    }

    fn handle_content_block_stop(
        &mut self,
        _obj: &serde_json::Map<String, Json>,
    ) -> ProtocolResult<Option<Vec<SseMessage>>> {
        // If this was a tool_use block, generate tool_calls chunk
        if self.current_block_type.as_deref() == Some("tool_use") {
            let tool_id = self.tool_id.take().unwrap_or_default();
            let tool_name = self.tool_name.take().unwrap_or_default();
            let tool_input = std::mem::take(&mut self.tool_input);

            let tool_calls_chunk = self.create_chunk(
                None,
                None,
                Some((tool_id.as_str(), tool_name.as_str(), tool_input.as_str())),
                None,
            );
            return Ok(Some(vec![SseMessage::new(&tool_calls_chunk)]));
        }
        self.current_block_type = None;
        Ok(None)
    }

    fn handle_message_delta(
        &mut self,
        obj: &serde_json::Map<String, Json>,
    ) -> ProtocolResult<Option<Vec<SseMessage>>> {
        // Extract stop_reason
        let stop_reason = obj
            .get("delta")
            .and_then(|v| v.as_object())
            .and_then(|d| d.get("stop_reason"))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        // Convert stop_reason
        let finish_reason = match stop_reason {
            "end_turn" | "stop_sequence" => "stop",
            "max_tokens" => "length",
            "tool_use" => "tool_calls",
            "pause_turn" => "tool_calls",
            "refusal" => "stop",
            _ => "stop",
        };

        // Extract usage
        if let Some(usage) = obj.get("usage").and_then(|v| v.as_object()) {
            if let Some(input_tokens) = usage.get("input_tokens").and_then(|v| v.as_u64()) {
                self.input_tokens = Some(input_tokens);
            }
            if let Some(output_tokens) = usage.get("output_tokens").and_then(|v| v.as_u64()) {
                self.output_tokens = Some(output_tokens);
            }
        }

        // Generate OpenAI chunk with finish_reason and usage
        Ok(Some(vec![SseMessage::new(&self.create_chunk(
            Some(finish_reason),
            None,
            None,
            None,
        ))]))
    }

    fn handle_message_stop(&mut self) -> ProtocolResult<Option<Vec<SseMessage>>> {
        // Generate [DONE] marker
        Ok(Some(vec![SseMessage::done()]))
    }

    fn create_chunk(
        &self,
        finish_reason: Option<&str>,
        content: Option<&str>,
        tool_call: Option<(&str, &str, &str)>,
        reasoning_content: Option<&str>,
    ) -> Json {
        let mut delta = json!({});

        if let Some(content) = content {
            delta["content"] = json!(content);
        }

        if let Some(reasoning) = reasoning_content {
            delta["reasoning_content"] = json!(reasoning);
        }

        if let Some((tool_id, tool_name, tool_input)) = tool_call {
            delta["tool_calls"] = json!([{
                "id": tool_id,
                "type": "function",
                "function": {
                    "name": tool_name,
                    "arguments": tool_input
                }
            }]);
        }

        let choice = json!({
            "index": 0,
            "delta": delta,
            "finish_reason": finish_reason.map(Json::from).unwrap_or(Json::Null)
        });

        let mut result = json!({
            "id": self.id.as_deref().unwrap_or(""),
            "object": "chat.completion.chunk",
            "created": self.created.unwrap_or_default(),
            "model": self.model.as_deref().unwrap_or(""),
            "choices": [choice]
        });

        // Add usage if we have it and this is the final chunk
        if finish_reason.is_some() {
            // Only include usage if we have at least one token count
            if self.input_tokens.is_some() || self.output_tokens.is_some() {
                let mut usage = json!({});
                if let Some(input_tokens) = self.input_tokens {
                    usage["prompt_tokens"] = json!(input_tokens);
                }
                if let Some(output_tokens) = self.output_tokens {
                    usage["completion_tokens"] = json!(output_tokens);
                }
                if let (Some(input), Some(output)) = (self.input_tokens, self.output_tokens) {
                    usage["total_tokens"] = json!(input + output);
                }
                result["usage"] = usage;
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ============================================================================
    // OpenAI → Anthropic 转换测试
    // ============================================================================

    #[test]
    fn test_openai_message_start_conversion() {
        let mut converter = OpenaiToAnthropic::default();
        let chunk = json!({
            "id": "chatcmpl-abc",
            "object": "chat.completion.chunk",
            "created": 1234567890,
            "model": "gpt-4",
            "choices": [{
                "index": 0,
                "delta": { "role": "assistant" },
                "finish_reason": null
            }]
        });

        let events = converter.process(SseMessage::new(&chunk)).unwrap();

        // Should generate message_start and content_block_start
        assert!(events.is_some());
        let events = events.unwrap();
        assert!(
            events
                .iter()
                .any(|e| e.event.as_deref() == Some("message_start"))
        );
        assert!(
            events
                .iter()
                .any(|e| e.event.as_deref() == Some("content_block_start"))
        );
    }

    #[test]
    fn test_openai_content_delta_conversion() {
        let mut converter = OpenaiToAnthropic::default();

        // First establish state
        let _ = converter.process(SseMessage::new(&json!({
            "id": "chatcmpl-abc",
            "model": "gpt-4",
            "choices": [{ "delta": { "role": "assistant" } }]
        })));

        // Then send content
        let chunk = json!({
            "choices": [{
                "delta": { "content": "Hello" },
                "finish_reason": null
            }]
        });

        let events = converter.process(SseMessage::new(&chunk)).unwrap();

        // Should generate content_block_delta with text_delta
        assert!(events.is_some());
        let events = events.unwrap();
        assert!(
            events
                .iter()
                .any(|e| e.event.as_deref() == Some("content_block_delta"))
        );
    }

    #[test]
    fn test_openai_finish_reason_conversion() {
        let mut converter = OpenaiToAnthropic::default();

        // Establish state
        let _ = converter.process(SseMessage::new(&json!({
            "choices": [{ "delta": { "role": "assistant" } }]
        })));

        // Send finish
        let chunk = json!({
            "choices": [{
                "delta": {},
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 8,
                "total_tokens": 18
            }
        });

        let events = converter.process(SseMessage::new(&chunk)).unwrap();

        // Should generate message_delta with stop_reason: end_turn
        assert!(events.is_some());
        let events = events.unwrap();
        assert!(
            events
                .iter()
                .any(|e| e.event.as_deref() == Some("message_delta"))
        );
        assert!(
            events
                .iter()
                .any(|e| e.event.as_deref() == Some("message_stop"))
        );
    }

    #[test]
    fn test_openai_to_anthropic_handles_done_marker() {
        let mut converter = OpenaiToAnthropic::default();

        // Initialize converter state first
        let _ = converter.process(SseMessage::new(&json!({
            "id": "chatcmpl-abc",
            "model": "gpt-4",
            "choices": [{ "delta": { "role": "assistant" } }]
        })));

        // Set output_tokens to verify it's included in message_delta
        converter.output_tokens = Some(42);

        // Send [DONE] marker - should generate exactly the three Anthropic ending events
        let result = converter.process(SseMessage::done()).unwrap();

        assert!(result.is_some());
        let events = result.unwrap();

        // Must generate exactly 3 events
        assert_eq!(events.len(), 3, "Should generate exactly 3 events");

        // Event 1: content_block_stop
        assert_eq!(events[0].event.as_deref(), Some("content_block_stop"));
        let data1: Json = serde_json::from_str(&events[0].data).unwrap();
        assert_eq!(
            data1,
            json!({
                "type": "content_block_stop",
                "index": 0
            })
        );

        // Event 2: message_delta - per Anthropic spec, usage only has output_tokens
        assert_eq!(events[1].event.as_deref(), Some("message_delta"));
        let data2: Json = serde_json::from_str(&events[1].data).unwrap();
        assert_eq!(
            data2,
            json!({
                "type": "message_delta",
                "delta": {
                    "stop_reason": "end_turn",
                    "stop_sequence": null,
                    "usage": {
                        "output_tokens": 42
                    }
                }
            })
        );

        // Event 3: message_stop
        assert_eq!(events[2].event.as_deref(), Some("message_stop"));
        let data3: Json = serde_json::from_str(&events[2].data).unwrap();
        assert_eq!(data3, json!({"type": "message_stop"}));
    }

    #[test]
    fn test_openai_done_after_finish_reason_generates_no_extra_events() {
        let mut converter = OpenaiToAnthropic::default();

        // Initialize converter
        let _ = converter.process(SseMessage::new(&json!({
            "id": "chatcmpl-abc",
            "model": "gpt-4",
            "choices": [{ "delta": { "role": "assistant" } }]
        })));

        // Send chunk with finish_reason - this generates the 3 end events
        let result = converter
            .process(SseMessage::new(&json!({
                "choices": [{
                    "delta": {},
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 10,
                    "completion_tokens": 5,
                    "total_tokens": 15
                }
            })))
            .unwrap();

        assert!(result.is_some());
        let events = result.unwrap();
        assert_eq!(
            events.len(),
            3,
            "finish_reason should generate 3 end events"
        );

        // Now send [DONE] - should NOT generate duplicate events
        let result = converter.process(SseMessage::done()).unwrap();
        assert!(
            result.is_none(),
            "[DONE] after finish_reason should not generate extra events"
        );
    }

    // ============================================================================
    // Anthropic → OpenAI 转换测试
    // ============================================================================

    #[test]
    fn test_anthropic_message_start_accumulation() {
        let mut converter = AnthropicToOpenai::default();

        // message_start should accumulate state but not produce output
        let event = json!({
            "type": "message_start",
            "message": {
                "id": "msg_abc",
                "type": "message",
                "role": "assistant",
                "model": "claude-sonnet-4-5-20250929",
                "usage": {
                    "input_tokens": 10,
                    "output_tokens": 0
                }
            }
        });

        let chunk = converter.process(SseMessage::new(&event)).unwrap();

        // Should return None (no output yet)
        assert!(chunk.is_none());
    }

    #[test]
    fn test_anthropic_content_delta_conversion() {
        let mut converter = AnthropicToOpenai::default();

        // Establish state
        let _ = converter.process(SseMessage::new(&json!({
            "type": "message_start",
            "message": {
                "id": "msg_abc",
                "model": "claude-sonnet-4-5-20250929",
                "usage": { "input_tokens": 10, "output_tokens": 0 }
            }
        })));

        // Send content delta
        let event = json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": {
                "type": "text_delta",
                "text": "Hello"
            }
        });

        let chunk = converter.process(SseMessage::new(&event)).unwrap();

        // Should generate OpenAI chunk with delta.content
        assert!(chunk.is_some());
        let lines = chunk.unwrap();
        assert!(lines.iter().any(|e| {
            let data: Json = serde_json::from_str(&e.data).unwrap();
            data["choices"][0]["delta"]["content"] == "Hello"
        }));
    }

    #[test]
    fn test_anthropic_message_delta_finish() {
        let mut converter = AnthropicToOpenai::default();

        // Establish state
        let _ = converter.process(SseMessage::new(&json!({
            "type": "message_start",
            "message": {
                "id": "msg_abc",
                "model": "claude-sonnet-4-5-20250929",
                "usage": { "input_tokens": 10, "output_tokens": 0 }
            }
        })));

        // Send message_delta
        let event = json!({
            "type": "message_delta",
            "delta": {
                "stop_reason": "end_turn"
            },
            "usage": {
                "output_tokens": 8
            }
        });

        let chunk = converter.process(SseMessage::new(&event)).unwrap();

        // Should generate OpenAI chunk with finish_reason: stop
        assert!(chunk.is_some());
        let lines = chunk.unwrap();
        assert!(lines.iter().any(|e| {
            let data: Json = serde_json::from_str(&e.data).unwrap();
            data["choices"][0]["finish_reason"] == "stop"
        }));
    }

    // ============================================================================
    // Edge cases and tool call tests
    // ============================================================================

    #[test]
    fn test_openai_to_anthropic_tool_calls() {
        let mut converter = OpenaiToAnthropic::default();

        // First establish state
        let _ = converter.process(SseMessage::new(&json!({
            "id": "chatcmpl-tool",
            "model": "gpt-4",
            "choices": [{ "delta": { "role": "assistant" } }]
        })));

        // Send tool call
        let chunk = json!({
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "id": "call_abc",
                        "type": "function",
                        "function": {
                            "name": "get_weather",
                            "arguments": "{\"location\": \"Beijing\"}"
                        }
                    }]
                }
            }]
        });

        let events = converter.process(SseMessage::new(&chunk)).unwrap();

        // Should generate tool_use content_block_start and input_json_delta
        assert!(events.is_some());
        let events = events.unwrap();
        assert!(
            events
                .iter()
                .any(|e| e.event.as_deref() == Some("content_block_start"))
        );
        assert!(
            events
                .iter()
                .any(|e| e.event.as_deref() == Some("content_block_delta"))
        );
    }

    #[test]
    fn test_anthropic_to_openai_tool_use() {
        let mut converter = AnthropicToOpenai::default();

        // Establish state
        let _ = converter.process(SseMessage::new(&json!({
            "type": "message_start",
            "message": {
                "id": "msg_tool",
                "model": "claude-sonnet-4-5-20250929",
                "usage": { "input_tokens": 10, "output_tokens": 0 }
            }
        })));

        // tool_use content_block_start
        let _ = converter.process(SseMessage::new(&json!({
            "type": "content_block_start",
            "index": 0,
            "content_block": {
                "type": "tool_use",
                "id": "tool_abc",
                "name": "search",
                "input": {}
            }
        })));

        // input_json_delta
        let _ = converter.process(SseMessage::new(&json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": {
                "type": "input_json_delta",
                "partial_json": "{\"query\": \"weather\"}"
            }
        })));

        // content_block_stop - should generate tool_calls chunk
        let chunk = converter
            .process(SseMessage::new(&json!({
                "type": "content_block_stop",
                "index": 0
            })))
            .unwrap();

        // Should have tool_calls in chunk
        assert!(chunk.is_some());
        let lines = chunk.unwrap();
        assert!(
            lines
                .iter()
                .any(|e| e.data.to_string().contains("\"tool_calls\""))
        );
    }

    #[test]
    fn test_invalid_event_handling() {
        let mut converter = AnthropicToOpenai::default();

        // Invalid event type
        let result = converter.process(SseMessage::new(&json!({
            "type": "invalid_event_type"
        })));

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, ProtocolError::InvalidStreamEvent(_)));
    }

    #[test]
    fn test_sse_line_done_variant() {
        // Test [DONE] marker handling
        let mut converter = AnthropicToOpenai::default();

        // Establish state
        let _ = converter.process(SseMessage::new(&json!({
            "type": "message_start",
            "message": { "id": "msg", "model": "claude", "usage": { "input_tokens": 0, "output_tokens": 0 } }
        })));

        // message_stop should return [DONE] marker
        let result = converter
            .process(SseMessage::new(&json!({
                "type": "message_stop"
            })))
            .unwrap();

        assert!(result.is_some());
        let lines = result.unwrap();
        assert!(lines.iter().any(|e| e.is_done()));
    }

    // ============================================================================
    // Protocol compliance tests
    // ============================================================================

    #[test]
    fn test_finish_reason_is_json_null_when_not_finished() {
        // Test that finish_reason is JSON null (not string "null") when not finished
        let mut converter = AnthropicToOpenai::default();

        // Establish state
        let _ = converter.process(SseMessage::new(&json!({
            "type": "message_start",
            "message": {
                "id": "msg_abc",
                "model": "claude-test",
                "usage": { "input_tokens": 10, "output_tokens": 0 }
            }
        })));

        // Send content delta - should generate chunk with finish_reason: null (JSON null)
        let event = json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": {
                "type": "text_delta",
                "text": "Hello"
            }
        });

        let chunk = converter.process(SseMessage::new(&event)).unwrap();

        assert!(chunk.is_some());
        let lines = chunk.unwrap();

        // Verify finish_reason is JSON null, not string "null"
        let data_line = lines
            .iter()
            .find(|e| !serde_json::from_str::<Json>(&e.data).unwrap().is_null())
            .map(|e| &e.data)
            .unwrap();

        let data_line: Json = serde_json::from_str(data_line).unwrap();
        let finish_reason = &data_line["choices"][0]["finish_reason"];
        // JSON null should have as_str() return None
        assert!(
            finish_reason.as_str().is_none(),
            "finish_reason should be JSON null, not a string"
        );
        assert!(finish_reason.is_null(), "finish_reason should be JSON null");
    }

    #[test]
    fn test_message_delta_usage_inside_delta() {
        // Test that message_delta has usage inside delta object (per Anthropic spec)
        let mut converter = OpenaiToAnthropic::default();

        // Establish state
        let _ = converter.process(SseMessage::new(&json!({
            "choices": [{ "delta": { "role": "assistant" } }]
        })));

        // Send finish with usage
        let chunk = json!({
            "choices": [{
                "delta": {},
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 8,
                "total_tokens": 18
            }
        });

        let events = converter.process(SseMessage::new(&chunk)).unwrap();
        assert!(events.is_some());

        // Find message_delta event and verify usage is inside delta
        let message_delta_data = events.unwrap().iter().find_map(|e| {
            let data: Json = serde_json::from_str(&e.data).unwrap();
            if data.get("type").and_then(|v| v.as_str()) == Some("message_delta") {
                Some(e.data.clone())
            } else {
                None
            }
        });

        assert!(
            message_delta_data.is_some(),
            "Should have message_delta event"
        );
        let data = message_delta_data.unwrap();
        let data: Json = serde_json::from_str(&data).unwrap();

        // usage should be inside delta object (per Anthropic spec)
        assert!(
            data.get("delta").and_then(|d| d.get("usage")).is_some(),
            "usage should be inside delta object"
        );
    }

    #[test]
    fn test_created_timestamp_is_set() {
        // Test that created timestamp is set (not default 0)
        let mut converter = AnthropicToOpenai::default();

        // Establish state
        let _ = converter.process(SseMessage::new(&json!({
            "type": "message_start",
            "message": {
                "id": "msg_ts_test",
                "model": "claude-test",
                "usage": { "input_tokens": 5, "output_tokens": 0 }
            }
        })));

        // Send content delta
        let event = json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": {
                "type": "text_delta",
                "text": "Test"
            }
        });

        let chunk = converter.process(SseMessage::new(&event)).unwrap();

        assert!(chunk.is_some());
        let lines = chunk.unwrap();

        // Verify created field is not 0
        let data_line = lines
            .iter()
            .find(|e| !serde_json::from_str::<Json>(&e.data).unwrap().is_null())
            .map(|e| &e.data)
            .unwrap();
        let data_line: Json = serde_json::from_str(data_line).unwrap();
        let created = data_line["created"].as_u64();
        assert!(created.is_some(), "created field should be a number");
        assert!(
            created.unwrap() > 0,
            "created timestamp should be > 0, got {:?}",
            created
        );
    }

    #[test]
    fn test_anthropic_to_openai_thinking_content() {
        let mut converter = AnthropicToOpenai::default();

        // Establish state with message_start
        let _ = converter.process(SseMessage::new(&json!({
            "type": "message_start",
            "message": {
                "id": "msg_abc",
                "model": "claude-sonnet-4-5-20250929",
                "usage": { "input_tokens": 10, "output_tokens": 0 }
            }
        })));

        // Start thinking block
        let _ = converter.process(SseMessage::new(&json!({
            "type": "content_block_start",
            "index": 0,
            "content_block": {
                "type": "thinking",
                "thinking": ""
            }
        })));

        // Send thinking delta
        let event = json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": {
                "type": "thinking_delta",
                "thinking": "Let me analyze this step by step..."
            }
        });

        let chunk = converter.process(SseMessage::new(&event)).unwrap();

        // Should generate OpenAI chunk with reasoning_content
        assert!(chunk.is_some());
        let lines = chunk.unwrap();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].event.is_none()); // OpenAI chunks don't have event field

        let data: Json = serde_json::from_str(&lines[0].data).unwrap();
        assert_eq!(
            data["choices"][0]["delta"]["reasoning_content"],
            "Let me analyze this step by step..."
        );
    }

    #[test]
    fn test_openai_to_anthropic_reasoning_content() {
        let mut converter = OpenaiToAnthropic::default();

        // First establish state
        let _ = converter.process(SseMessage::new(&json!({
            "id": "chatcmpl-abc",
            "model": "gpt-4",
            "choices": [{ "delta": { "role": "assistant" } }]
        })));

        // Send reasoning content delta
        let chunk = json!({
            "choices": [{
                "delta": { "reasoning_content": "Let me think about this..." },
                "finish_reason": null
            }]
        });

        let events = converter.process(SseMessage::new(&chunk)).unwrap();

        // Should generate thinking content_block_delta
        assert!(events.is_some());
        let events = events.unwrap();
        assert_eq!(events.len(), 1);

        // Check event type
        assert_eq!(events[0].event.as_deref(), Some("content_block_delta"));

        // Check data content
        let data: Json = serde_json::from_str(&events[0].data).unwrap();
        assert_eq!(data["type"], "content_block_delta");
        assert_eq!(data["index"], 0);
        assert_eq!(data["delta"]["type"], "thinking_delta");
        assert_eq!(data["delta"]["thinking"], "Let me think about this...");
    }
}
