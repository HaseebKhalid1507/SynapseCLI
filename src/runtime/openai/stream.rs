//! Streaming path for OpenAI-compatible providers.
//!
//! Mirrors `ApiMethods::call_api_stream_inner` but speaks OpenAI chat/completions
//! and translates back to Anthropic-shaped events for the rest of the runtime.

use super::translate;
use super::types::{ChatMessage, ChatRequest, OaiEvent, ProviderConfig, StreamOptions, ToolCall};
use super::wire::StreamDecoder;
use crate::runtime::types::StreamEvent;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use futures::StreamExt;
use serde_json::{json, Value};
use tokio::sync::mpsc;

/// Run a single streaming request against an OpenAI-compatible endpoint.
///
/// Returns the final assistant response as an Anthropic-shaped content Value
/// (`{"content": [..text.., ..tool_use..]}`) so the outer agent loop can keep
/// using the same handling as the native Anthropic path.
pub(crate) async fn call_oai_stream_inner(
    cfg: &ProviderConfig,
    client: &reqwest::Client,
    tools_schema: &[Value],
    system_prompt: &Option<String>,
    messages: &[Value],
    tx: &mpsc::UnboundedSender<StreamEvent>,
    temperature: Option<f32>,
    max_tokens: Option<u32>,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
    let oai_messages = translate::messages_to_oai(messages, system_prompt);
    let oai_tools = translate::tools_to_oai(tools_schema);
    let tools_opt = if oai_tools.is_empty() { None } else { Some(oai_tools) };

    // Google's OpenAI-compat endpoint rejects stream_options
    let stream_options = if cfg.base_url.contains("googleapis.com") {
        None
    } else {
        Some(StreamOptions { include_usage: true })
    };

    let body = ChatRequest {
        model: cfg.model.clone(),
        messages: oai_messages,
        stream: true,
        stream_options,
        max_tokens,
        temperature,
        tools: tools_opt,
        tool_choice: None,
    };

    let url = format!("{}/chat/completions", cfg.base_url.trim_end_matches('/'));

    tracing::debug!(url=%url, model=%cfg.model, "openai stream request");

    let resp = match client
        .post(&url)
        .bearer_auth(&cfg.api_key)
        .header("content-type", "application/json")
        .header("accept", "text/event-stream")
        .json(&body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            if e.is_connect() && url.contains("localhost") {
                return Err(format!(
                    "Can't reach local endpoint at {} — is Ollama/LM Studio running?",
                    url
                ).into());
            }
            return Err(e.into());
        }
    };

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("openai request failed: {status}: {text}").into());
    }

    let mut decoder = StreamDecoder::new();
    let mut accumulated_text = String::new();
    let mut tool_use_blocks: Vec<Value> = Vec::new();
    let mut buf = bytes::BytesMut::with_capacity(8 * 1024);
    let mut sink: Vec<OaiEvent> = Vec::with_capacity(4);
    let mut stream = resp.bytes_stream();

    while let Some(chunk) = tokio::select! {
        chunk = stream.next() => chunk,
        _ = cancel.cancelled() => {
            return Err("request cancelled".into());
        }
    } {
        let chunk = chunk?;
        buf.extend_from_slice(&chunk);

        // Scan for newline-delimited SSE lines (SIMD-accelerated via memchr)
        while let Some(nl) = memchr::memchr(b'\n', &buf) {
            let line_bytes = buf.split_to(nl + 1); // O(1) — ref-counted split
            let line = std::str::from_utf8(&line_bytes[..nl]).unwrap_or("");

            sink.clear();
            decoder.push_line(line, &mut sink);
            handle_events(&sink, tx, &mut accumulated_text, &mut tool_use_blocks);
        }
    }

    // Flush any remaining buffered line + final Done
    if !buf.is_empty() {
        let line = std::str::from_utf8(&buf).unwrap_or("");
        sink.clear();
        decoder.push_line(line, &mut sink);
        handle_events(&sink, tx, &mut accumulated_text, &mut tool_use_blocks);
    }
    sink.clear();
    decoder.finish(&mut sink);
    handle_events(&sink, tx, &mut accumulated_text, &mut tool_use_blocks);

    // Build Anthropic-shaped final response
    let mut content: Vec<Value> = Vec::new();
    if !accumulated_text.is_empty() {
        content.push(json!({"type": "text", "text": accumulated_text}));
    }
    content.extend(tool_use_blocks);

    Ok(json!({
        "role": "assistant",
        "content": content,
    }))
}

pub(crate) async fn call_codex_stream_inner(
    cfg: &ProviderConfig,
    client: &reqwest::Client,
    tools_schema: &[Value],
    system_prompt: &Option<String>,
    messages: &[Value],
    tx: &mpsc::UnboundedSender<StreamEvent>,
    temperature: Option<f32>,
    max_tokens: Option<u32>,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
    let creds = if cfg.api_key.is_empty() {
        crate::auth::ensure_fresh_provider_token(client, "openai-codex").await?
    } else {
        crate::auth::OAuthCredentials {
            auth_type: "oauth".to_string(),
            refresh: String::new(),
            access: cfg.api_key.clone(),
            expires: 0,
            account_id: None,
        }
    };
    let account_id = creds
        .account_id
        .clone()
        .or_else(|| extract_codex_account_id(&creds.access))
        .ok_or("Failed to extract ChatGPT account id from Codex token")?;

    let oai_messages = translate::messages_to_oai(messages, system_prompt);
    let oai_tools = translate::tools_to_oai(tools_schema);
    let tools: Vec<Value> = oai_tools
        .into_iter()
        .map(|tool| {
            json!({
                "type": "function",
                "name": tool.function.name,
                "description": tool.function.description.unwrap_or_default(),
                "parameters": tool.function.parameters,
            })
        })
        .collect();

    let mut body = json!({
        "model": cfg.model,
        "store": false,
        "stream": true,
        "instructions": system_prompt.clone().unwrap_or_default(),
        "input": codex_input_messages(oai_messages),
        "tool_choice": "auto",
        "parallel_tool_calls": true,
        "include": ["reasoning.encrypted_content"],
        "text": { "verbosity": "medium" },
    });
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools);
    }
    if let Some(temp) = temperature {
        body["temperature"] = json!(temp);
    }
    if let Some(max) = max_tokens {
        body["max_output_tokens"] = json!(max);
    }

    let url = format!(
        "{}/codex/responses",
        cfg.base_url.trim_end_matches('/').trim_end_matches("/codex")
    );
    tracing::debug!(url=%url, model=%cfg.model, "codex stream request");

    let resp = client
        .post(&url)
        .bearer_auth(&creds.access)
        .header("chatgpt-account-id", account_id)
        .header("originator", "synaps")
        .header("OpenAI-Beta", "responses=experimental")
        .header("content-type", "application/json")
        .header("accept", "text/event-stream")
        .json(&body)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("codex request failed: {status}: {text}").into());
    }

    let mut accumulated_text = String::new();
    let mut parser = CodexSseDecoder::default();
    let mut buf = bytes::BytesMut::with_capacity(8 * 1024);
    let mut stream = resp.bytes_stream();

    while let Some(chunk) = tokio::select! {
        chunk = stream.next() => chunk,
        _ = cancel.cancelled() => {
            return Err("request cancelled".into());
        }
    } {
        let chunk = chunk?;
        buf.extend_from_slice(&chunk);
        while let Some(nl) = memchr::memchr(b'\n', &buf) {
            let line_bytes = buf.split_to(nl + 1);
            let line = std::str::from_utf8(&line_bytes[..nl]).unwrap_or("");
            parser.push_line(line, tx, &mut accumulated_text);
        }
    }
    if !buf.is_empty() {
        let line = std::str::from_utf8(&buf).unwrap_or("");
        parser.push_line(line, tx, &mut accumulated_text);
    }
    parser.finish();

    let mut content: Vec<Value> = Vec::new();
    if !accumulated_text.is_empty() {
        content.push(json!({"type": "text", "text": accumulated_text}));
    }
    content.extend(translate::tool_calls_to_content_blocks(&parser.completed_tools));

    Ok(json!({
        "role": "assistant",
        "content": content,
    }))
}

fn codex_input_messages(messages: Vec<ChatMessage>) -> Vec<Value> {
    let mut out = Vec::new();
    for msg in messages {
        if let Some(tool_calls) = msg.tool_calls {
            for call in tool_calls {
                out.push(json!({
                    "type": "function_call",
                    "id": call.id,
                    "call_id": call.id,
                    "name": call.function.name,
                    "arguments": call.function.arguments,
                }));
            }
            continue;
        }
        if msg.role == "tool" {
            out.push(json!({
                "type": "function_call_output",
                "call_id": msg.tool_call_id.unwrap_or_default(),
                "output": msg.content.unwrap_or_default(),
            }));
            continue;
        }
        out.push(json!({
            "role": msg.role,
            "content": msg.content.unwrap_or_default(),
        }));
    }
    out
}

#[derive(Default)]
struct CodexSseDecoder {
    buffer: String,
    active_tools: Vec<CodexToolAccumulator>,
    completed_tools: Vec<ToolCall>,
}

#[derive(Default)]
struct CodexToolAccumulator {
    id: String,
    name: String,
    arguments: String,
    started: bool,
}

impl CodexSseDecoder {
    fn push_line(
        &mut self,
        line: &str,
        tx: &mpsc::UnboundedSender<StreamEvent>,
        text_acc: &mut String,
    ) {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            if !self.buffer.is_empty() {
                let payload = std::mem::take(&mut self.buffer);
                self.push_payload(&payload, tx, text_acc);
            }
            return;
        }
        let Some(data) = line.strip_prefix("data:").map(str::trim_start) else {
            return;
        };
        if data == "[DONE]" {
            self.finish();
            return;
        }
        self.buffer.push_str(data);
    }

    fn push_payload(
        &mut self,
        payload: &str,
        tx: &mpsc::UnboundedSender<StreamEvent>,
        text_acc: &mut String,
    ) {
        let Ok(event) = serde_json::from_str::<Value>(payload) else {
            return;
        };
        let event_type = event.get("type").and_then(Value::as_str).unwrap_or_default();
        match event_type {
            "response.output_text.delta" => {
                if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                    text_acc.push_str(delta);
                    let _ = tx.send(StreamEvent::Llm(crate::runtime::types::LlmEvent::Text(
                        delta.to_string(),
                    )));
                }
            }
            "response.output_item.added" => {
                if let Some(item) = event.get("item") {
                    let idx = event.get("output_index").and_then(Value::as_u64).unwrap_or(0) as usize;
                    self.add_tool_from_item(idx, item, tx);
                }
            }
            "response.function_call_arguments.delta" => {
                let idx = event.get("output_index").and_then(Value::as_u64).unwrap_or(0) as usize;
                let delta = event.get("delta").and_then(Value::as_str).unwrap_or_default();
                if !delta.is_empty() {
                    let tool = self.ensure_tool(idx);
                    tool.arguments.push_str(delta);
                    let _ = tx.send(StreamEvent::Llm(
                        crate::runtime::types::LlmEvent::ToolUseDelta(delta.to_string()),
                    ));
                }
            }
            "response.output_item.done" => {
                if let Some(item) = event.get("item") {
                    let idx = event.get("output_index").and_then(Value::as_u64).unwrap_or(0) as usize;
                    self.complete_tool_from_item(idx, item, tx);
                }
            }
            "response.completed" | "response.done" => {
                self.push_usage(&event, tx);
                self.finish();
            }
            _ => {}
        }
    }

    fn ensure_tool(&mut self, idx: usize) -> &mut CodexToolAccumulator {
        while self.active_tools.len() <= idx {
            self.active_tools.push(CodexToolAccumulator::default());
        }
        &mut self.active_tools[idx]
    }

    fn add_tool_from_item(
        &mut self,
        idx: usize,
        item: &Value,
        tx: &mpsc::UnboundedSender<StreamEvent>,
    ) {
        if item.get("type").and_then(Value::as_str) != Some("function_call") {
            return;
        }
        let tool = self.ensure_tool(idx);
        if let Some(id) = item
            .get("call_id")
            .or_else(|| item.get("id"))
            .and_then(Value::as_str)
        {
            tool.id = id.to_string();
        }
        if let Some(name) = item.get("name").and_then(Value::as_str) {
            tool.name = name.to_string();
        }
        if !tool.started && !tool.name.is_empty() {
            tool.started = true;
            let _ = tx.send(StreamEvent::Llm(
                crate::runtime::types::LlmEvent::ToolUseStart(tool.name.clone()),
            ));
        }
    }

    fn complete_tool_from_item(
        &mut self,
        idx: usize,
        item: &Value,
        tx: &mpsc::UnboundedSender<StreamEvent>,
    ) {
        if item.get("type").and_then(Value::as_str) != Some("function_call") {
            return;
        }
        let tool = self.ensure_tool(idx);
        if let Some(id) = item
            .get("call_id")
            .or_else(|| item.get("id"))
            .and_then(Value::as_str)
        {
            tool.id = id.to_string();
        }
        if let Some(name) = item.get("name").and_then(Value::as_str) {
            tool.name = name.to_string();
        }
        if let Some(arguments) = item.get("arguments").and_then(Value::as_str) {
            tool.arguments = arguments.to_string();
        }
        if !tool.started && !tool.name.is_empty() {
            tool.started = true;
            let _ = tx.send(StreamEvent::Llm(
                crate::runtime::types::LlmEvent::ToolUseStart(tool.name.clone()),
            ));
        }
        let completed = if !tool.id.is_empty() && !tool.name.is_empty() {
            Some(ToolCall {
                id: tool.id.clone(),
                kind: "function".to_string(),
                function: super::types::FunctionCall {
                    name: tool.name.clone(),
                    arguments: tool.arguments.clone(),
                },
            })
        } else {
            None
        };
        if let Some(call) = completed {
            if self.completed_tools.iter().any(|done| done.id == call.id) {
                return;
            }
            self.completed_tools.push(ToolCall {
                id: call.id,
                kind: call.kind,
                function: call.function,
            });
        }
    }

    fn push_usage(&self, event: &Value, tx: &mpsc::UnboundedSender<StreamEvent>) {
        let usage = event
            .get("response")
            .and_then(|r| r.get("usage"))
            .or_else(|| event.get("usage"));
        let input = usage
            .and_then(|u| u.get("input_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let output = usage
            .and_then(|u| u.get("output_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if input > 0 || output > 0 {
            let _ = tx.send(StreamEvent::Session(crate::runtime::types::SessionEvent::Usage {
                input_tokens: input,
                output_tokens: output,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
                model: None,
            }));
        }
    }

    fn finish(&mut self) {
        for tool in &self.active_tools {
            if !tool.id.is_empty()
                && !tool.name.is_empty()
                && !self.completed_tools.iter().any(|done| done.id == tool.id)
            {
                self.completed_tools.push(ToolCall {
                    id: tool.id.clone(),
                    kind: "function".to_string(),
                    function: super::types::FunctionCall {
                        name: tool.name.clone(),
                        arguments: tool.arguments.clone(),
                    },
                });
            }
        }
    }
}

fn extract_codex_account_id(token: &str) -> Option<String> {
    const JWT_CLAIM_PATH: &str = "https://api.openai.com/auth";
    let payload = token.split('.').nth(1)?;
    let decoded = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let json: Value = serde_json::from_slice(&decoded).ok()?;
    json.get(JWT_CLAIM_PATH)?
        .get("chatgpt_account_id")?
        .as_str()
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}

fn handle_events(
    events: &[OaiEvent],
    tx: &mpsc::UnboundedSender<StreamEvent>,
    text_acc: &mut String,
    tool_blocks: &mut Vec<Value>,
) {
    for ev in events {
        if let OaiEvent::TextDelta(t) = ev {
            text_acc.push_str(t);
        }
        if let OaiEvent::ToolCallsComplete { calls, .. } = ev {
            tool_blocks.extend(translate::tool_calls_to_content_blocks(calls));
        }
        if let Some(se) = translate::oai_event_to_llm(ev) {
            let _ = tx.send(se);
        }
    }
}
