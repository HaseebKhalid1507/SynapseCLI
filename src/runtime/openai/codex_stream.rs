//! Streaming path for the OpenAI Codex Responses API.
//!
//! Used when users authenticate via ChatGPT OAuth — calls go to
//! `https://chatgpt.com/backend-api/codex/responses` using the Responses API
//! wire format (NOT the standard `/v1/chat/completions` format).
//!
//! Supported models via ChatGPT OAuth: `gpt-5.2`, `gpt-5.4`, `gpt-5.5`.
//!
//! # Responses API SSE events we handle
//!
//! | Event type                    | Action                                      |
//! |-------------------------------|---------------------------------------------|
//! | `response.output_text.delta`  | Stream text chunk via `StreamEvent::Llm`    |
//! | `response.output_item.done`   | Capture completed function_call items        |
//! | `response.completed`          | Extract usage stats                          |
//! | `response.failed`             | Surface error to caller                      |
//! | All other events              | Ignored (no-op)                             |

use super::translate;
use crate::runtime::types::{LlmEvent, SessionEvent, StreamEvent};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use bytes::BytesMut;
use futures::StreamExt;
use serde_json::{json, Value};
use tokio::sync::mpsc;

// ── Constants ────────────────────────────────────────────────────────────────

const CODEX_ENDPOINT: &str = "https://chatgpt.com/backend-api/codex/responses";

// ── Public entry point ───────────────────────────────────────────────────────

/// Run a single streaming request against `chatgpt.com/backend-api/codex/responses`.
///
/// Translates the Responses API SSE stream back into the Anthropic-shaped
/// `{"role": "assistant", "content": [...]}` value that the outer agent loop
/// expects — identical contract to `call_oai_stream_inner`.
///
/// # Parameters
/// - `access_token` — JWT OAuth access token from ChatGPT login
/// - `model` — bare model id, e.g. `"gpt-5.5"` (without provider prefix)
/// - `client` — shared `reqwest::Client`
/// - `tools_schema` — Anthropic-shaped tool definitions from the tool registry
/// - `system_prompt` — optional system instruction string
/// - `messages` — Anthropic-shaped conversation history
/// - `tx` — unbounded sender for streaming UI events
/// - `cancel` — cooperative cancellation token
pub(crate) async fn call_codex_stream_inner(
    access_token: &str,
    model: &str,
    client: &reqwest::Client,
    tools_schema: &[Value],
    system_prompt: &Option<String>,
    messages: &[Value],
    tx: &mpsc::UnboundedSender<StreamEvent>,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
    // ── 1. Extract chatgpt_account_id from the JWT ───────────────────────────
    let account_id = extract_chatgpt_account_id(access_token).unwrap_or_default();

    // ── 2. Build the request body ────────────────────────────────────────────
    let oai_messages = translate::messages_to_oai(messages, &None);

    // The Responses API `input` field mirrors the chat/completions message list,
    // but without the system message (sent separately as `instructions`).
    let input: Vec<Value> = oai_messages
        .iter()
        .filter(|m| m.role != "system")
        .map(|m| {
            // Build the input message object.
            // tool_call_id messages become "tool" role with a special shape.
            if m.role == "tool" {
                // Tool result — Responses API expects role:"tool" with output field
                json!({
                    "role": "tool",
                    "tool_call_id": m.tool_call_id.as_deref().unwrap_or(""),
                    "output": m.content.as_deref().unwrap_or(""),
                })
            } else if m.role == "assistant" && m.tool_calls.is_some() {
                // Assistant message with tool calls
                let calls = m.tool_calls.as_ref().unwrap();
                let call_values: Vec<Value> = calls
                    .iter()
                    .map(|c| {
                        json!({
                            "type": "function_call",
                            "id": c.id,
                            "name": c.function.name,
                            "arguments": c.function.arguments,
                        })
                    })
                    .collect();
                json!({
                    "role": "assistant",
                    "content": call_values,
                })
            } else {
                // Normal user or assistant text message
                json!({
                    "role": m.role,
                    "content": m.content.as_deref().unwrap_or(""),
                })
            }
        })
        .collect();

    // Extract system prompt text (from chat messages or the dedicated param)
    let instructions: Option<String> = oai_messages
        .iter()
        .find(|m| m.role == "system")
        .and_then(|m| m.content.clone())
        .or_else(|| system_prompt.clone());

    let tools_value = tools_to_responses_api(tools_schema);

    let mut body = json!({
        "model": model,
        "store": false,
        "stream": true,
        "input": input,
        "text": { "verbosity": "medium" },
        "parallel_tool_calls": true,
    });

    if let Some(ref instr) = instructions {
        if !instr.is_empty() {
            body["instructions"] = json!(instr);
        }
    }

    if !tools_value.is_empty() {
        body["tools"] = json!(tools_value);
        body["tool_choice"] = json!("auto");
    }

    tracing::debug!(
        url = %CODEX_ENDPOINT,
        model = %model,
        account_id = %account_id,
        "codex responses stream request"
    );

    // ── 3. Send HTTP request ─────────────────────────────────────────────────
    let mut req = client
        .post(CODEX_ENDPOINT)
        .bearer_auth(access_token)
        .header("content-type", "application/json")
        .header("accept", "text/event-stream")
        .header("originator", "synaps");

    if !account_id.is_empty() {
        req = req.header("chatgpt-account-id", &account_id);
    }

    let resp = req.json(&body).send().await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("codex responses request failed: {status}: {text}").into());
    }

    // ── 4. Stream and parse SSE events ───────────────────────────────────────
    let mut accumulated_text = String::new();
    let mut tool_use_blocks: Vec<Value> = Vec::new();

    // SSE framing state: events are pairs of `event:` + `data:` lines,
    // separated by blank lines. We track the most recent event type so we
    // can route the accompanying data line correctly.
    let mut current_event_type = String::new();
    let mut buf = BytesMut::with_capacity(16 * 1024);
    let mut stream = resp.bytes_stream();

    'stream: while let Some(chunk) = tokio::select! {
        chunk = stream.next() => chunk,
        _ = cancel.cancelled() => {
            return Err("request cancelled".into());
        }
    } {
        let chunk = chunk?;
        buf.extend_from_slice(&chunk);

        // Process all complete newline-terminated lines from the buffer.
        // Using memchr for O(n) scanning (SIMD-accelerated on supported platforms).
        loop {
            match memchr::memchr(b'\n', &buf) {
                None => break, // no complete line yet — wait for more bytes
                Some(nl) => {
                    // Split the line out of the buffer without allocating a copy
                    // of the entire remaining buffer — only the prefix is cloned.
                    let line_bytes = buf.split_to(nl + 1);
                    // Strip the trailing `\n` (and optional `\r`) from the view
                    let end = if nl > 0 && line_bytes[nl - 1] == b'\r' { nl - 1 } else { nl };
                    let line = match std::str::from_utf8(&line_bytes[..end]) {
                        Ok(s) => s,
                        Err(_) => continue, // malformed UTF-8 — skip
                    };

                    if let LoopAction::Break = handle_sse_line(
                        line,
                        &mut current_event_type,
                        &mut accumulated_text,
                        &mut tool_use_blocks,
                        tx,
                    )? {
                        break 'stream;
                    }
                }
            }
        }
    }

    // Flush any remaining bytes (stream ended without a trailing newline)
    if !buf.is_empty() {
        if let Ok(line) = std::str::from_utf8(&buf) {
            let line = line.trim_end_matches(['\r', '\n']);
            let _ = handle_sse_line(
                line,
                &mut current_event_type,
                &mut accumulated_text,
                &mut tool_use_blocks,
                tx,
            );
        }
    }

    // ── 5. Build Anthropic-shaped response ───────────────────────────────────
    let mut content: Vec<Value> = Vec::new();
    if !accumulated_text.is_empty() {
        content.push(json!({ "type": "text", "text": accumulated_text }));
    }
    content.extend(tool_use_blocks);

    Ok(json!({
        "role": "assistant",
        "content": content,
    }))
}

// ── SSE line handler ─────────────────────────────────────────────────────────

/// Signal returned from `handle_sse_line` to indicate whether the outer loop
/// should keep going (`Continue`) or stop early (`Break`).
enum LoopAction {
    Continue,
    Break,
}

/// Process a single SSE line, mutating accumulated state and sending events to
/// the UI channel as appropriate.
///
/// Returns `Ok(LoopAction::Break)` if the caller should stop reading (e.g. on
/// a `response.failed` or `response.completed` that should end the stream).
fn handle_sse_line(
    line: &str,
    current_event_type: &mut String,
    accumulated_text: &mut String,
    tool_use_blocks: &mut Vec<Value>,
    tx: &mpsc::UnboundedSender<StreamEvent>,
) -> Result<LoopAction, Box<dyn std::error::Error + Send + Sync>> {
    // Blank line = end of one SSE event frame; reset event-type tracking
    if line.is_empty() {
        current_event_type.clear();
        return Ok(LoopAction::Continue);
    }

    // Comment line — ignore
    if line.starts_with(':') {
        return Ok(LoopAction::Continue);
    }

    // `event:` line — store the event type for the upcoming `data:` line
    if let Some(event_name) = line.strip_prefix("event:") {
        *current_event_type = event_name.trim().to_string();
        return Ok(LoopAction::Continue);
    }

    // `data:` line — the actual payload
    if let Some(raw) = line.strip_prefix("data:") {
        let payload = raw.trim_start();

        // Guard against empty or sentinel payloads
        if payload.is_empty() || payload == "[DONE]" {
            return Ok(LoopAction::Continue);
        }

        let data: Value = match serde_json::from_str(payload) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    event = %current_event_type,
                    error = %e,
                    "codex: failed to parse SSE data payload"
                );
                return Ok(LoopAction::Continue);
            }
        };

        // Prefer the event type from the `event:` line; fall back to the
        // `type` field embedded in the JSON payload itself (some implementations
        // only send one or the other).
        let event_type: &str = if !current_event_type.is_empty() {
            current_event_type.as_str()
        } else {
            data.get("type").and_then(|v| v.as_str()).unwrap_or("")
        };

        match event_type {
            // ── Text streaming ───────────────────────────────────────────────
            "response.output_text.delta" => {
                // The delta field contains the incremental text chunk.
                if let Some(delta) = data.get("delta").and_then(|v| v.as_str()) {
                    if !delta.is_empty() {
                        accumulated_text.push_str(delta);
                        let _ = tx.send(StreamEvent::Llm(LlmEvent::Text(delta.to_string())));
                    }
                }
            }

            // ── Tool call completion ─────────────────────────────────────────
            //
            // `response.output_item.done` fires when any output item (text or
            // function_call) is fully assembled. We only act on function_call items.
            "response.output_item.done" => {
                let item = data.get("item").unwrap_or(&data);
                if item.get("type").and_then(|v| v.as_str()) == Some("function_call") {
                    if let Some(block) = parse_function_call_item(item) {
                        // Emit tool-use start + complete events for the UI
                        if let Some(name) = block.get("name").and_then(|v| v.as_str()) {
                            let _ = tx.send(StreamEvent::Llm(LlmEvent::ToolUseStart(name.to_string())));
                        }
                        let tool_id = block.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let tool_name = block.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let input = block.get("input").cloned().unwrap_or_else(|| json!({}));

                        let _ = tx.send(StreamEvent::Llm(LlmEvent::ToolUse {
                            tool_name: tool_name.clone(),
                            tool_id: tool_id.clone(),
                            input: input.clone(),
                        }));

                        // Accumulate as an Anthropic-shaped tool_use content block
                        tool_use_blocks.push(json!({
                            "type": "tool_use",
                            "id": tool_id,
                            "name": tool_name,
                            "input": input,
                        }));
                    }
                }
            }

            // ── Usage stats from the completed response ──────────────────────
            "response.completed" => {
                if let Some(usage) = data.get("response").and_then(|r| r.get("usage"))
                    .or_else(|| data.get("usage"))
                {
                    let input_tokens = usage
                        .get("input_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let output_tokens = usage
                        .get("output_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);

                    if input_tokens > 0 || output_tokens > 0 {
                        tracing::debug!(
                            input_tokens = %input_tokens,
                            output_tokens = %output_tokens,
                            "codex: usage"
                        );
                        let _ = tx.send(StreamEvent::Session(SessionEvent::Usage {
                            input_tokens,
                            output_tokens,
                            cache_read_input_tokens: 0,
                            cache_creation_input_tokens: 0,
                            model: None,
                        }));
                    }
                }
                // The stream is logically complete; stop reading to avoid
                // blocking on any trailing keepalive bytes.
                return Ok(LoopAction::Break);
            }

            // ── Errors ───────────────────────────────────────────────────────
            "response.failed" => {
                let error_msg = data
                    .get("response")
                    .and_then(|r| r.get("error"))
                    .and_then(|e| e.get("message"))
                    .and_then(|m| m.as_str())
                    .or_else(|| data.get("error").and_then(|e| e.as_str()))
                    .unwrap_or("unknown error");

                return Err(format!("codex response failed: {}", error_msg).into());
            }

            // ── Informational / lifecycle events — no action needed ───────────
            //
            // `response.created`        — response object created server-side
            // `response.in_progress`    — status update
            // `response.output_item.added`  — output item slot opened
            // `response.content_part.added` — content part slot opened
            // `response.output_text.done`   — text output finished (no new data)
            // `response.content_part.done`  — content part finished
            _ => {
                tracing::trace!(event_type = %event_type, "codex: unhandled SSE event");
            }
        }
    }

    Ok(LoopAction::Continue)
}

// ── Tool schema translation ──────────────────────────────────────────────────

/// Convert Anthropic-shaped tool definitions to the Responses API `tools` format.
///
/// Anthropic shape:
/// ```json
/// {"name": "...", "description": "...", "input_schema": {...}}
/// ```
///
/// Responses API shape:
/// ```json
/// {"type": "function", "name": "...", "description": "...", "parameters": {...}, "strict": null}
/// ```
fn tools_to_responses_api(schema: &[Value]) -> Vec<Value> {
    schema
        .iter()
        .filter_map(|t| {
            let name = t.get("name")?.as_str()?;
            // Skip empty names and internal-only pseudo-tools that have no
            // corresponding server-side implementation.
            if name.is_empty()
                || name == "respond"
                || name == "send_channel"
                || name == "watcher_exit"
            {
                return None;
            }

            let description = t
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or("")
                .to_string();

            let parameters = t
                .get("input_schema")
                .cloned()
                .unwrap_or_else(|| json!({"type": "object", "properties": {}}));

            Some(json!({
                "type": "function",
                "name": name,
                "description": description,
                "parameters": parameters,
                "strict": null,
            }))
        })
        .collect()
}

// ── Tool call item parsing ───────────────────────────────────────────────────

/// Parse a Responses API `function_call` output item into an Anthropic-shaped
/// `tool_use` block.
///
/// Expected input shape (from `response.output_item.done`):
/// ```json
/// {
///   "type": "function_call",
///   "id": "call_abc123",
///   "call_id": "call_abc123",
///   "name": "tool_name",
///   "arguments": "{\"key\": \"value\"}"
/// }
/// ```
///
/// Output shape (Anthropic `tool_use` content block):
/// ```json
/// {"type": "tool_use", "id": "...", "name": "...", "input": {...}}
/// ```
fn parse_function_call_item(item: &Value) -> Option<Value> {
    let name = item.get("name").and_then(|v| v.as_str())?;

    // The id may appear as `"id"` or `"call_id"` depending on the server version
    let id = item
        .get("id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .or_else(|| item.get("call_id").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string();

    // `arguments` is a JSON-encoded string; parse it back into a Value so we
    // match the Anthropic `input` field shape. Fall back to `{}` on error.
    let arguments_str = item
        .get("arguments")
        .and_then(|v| v.as_str())
        .unwrap_or("{}");

    let input: Value = serde_json::from_str(arguments_str).unwrap_or_else(|e| {
        tracing::warn!(
            tool = %name,
            error = %e,
            raw = %arguments_str,
            "codex: failed to parse tool arguments JSON"
        );
        json!({})
    });

    Some(json!({
        "type": "tool_use",
        "id": id,
        "name": name,
        "input": input,
    }))
}

// ── JWT account-id extraction ────────────────────────────────────────────────

/// Extract `chatgpt_account_id` from the JWT access token.
///
/// The OpenAI OAuth JWT carries a custom claim:
/// ```json
/// {
///   "https://api.openai.com/auth": {
///     "chatgpt_account_id": "user-abc123"
///   }
/// }
/// ```
///
/// JWTs are three base64url-encoded segments separated by `.`.  We only need
/// the middle segment (the payload).  The padding is optional per RFC 7515 so
/// we use `URL_SAFE_NO_PAD`; if that fails we try adding padding and re-decoding.
///
/// Returns `None` if the token is malformed, the claim is absent, or the value
/// is empty.  The caller should treat `None` as "omit the header" rather than
/// as a hard error.
fn extract_chatgpt_account_id(access_token: &str) -> Option<String> {
    // Split "header.payload.signature"
    let mut parts = access_token.splitn(3, '.');
    let _header = parts.next()?;
    let payload_b64 = parts.next()?;

    // Decode base64url, tolerating missing padding
    let decoded = URL_SAFE_NO_PAD
        .decode(payload_b64)
        .or_else(|_| {
            // Add padding characters and retry
            let padded = match payload_b64.len() % 4 {
                2 => format!("{}==", payload_b64),
                3 => format!("{}=", payload_b64),
                _ => payload_b64.to_string(),
            };
            URL_SAFE_NO_PAD.decode(&padded)
        })
        .ok()?;

    let claims: Value = serde_json::from_slice(&decoded).ok()?;

    // Navigate: claims["https://api.openai.com/auth"]["chatgpt_account_id"]
    let account_id = claims
        .get("https://api.openai.com/auth")?
        .get("chatgpt_account_id")?
        .as_str()?;

    if account_id.is_empty() {
        return None;
    }

    Some(account_id.to_string())
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // Build a minimal JWT with the given claims payload (no real signature —
    // we only care about parsing, not verification).
    fn make_jwt(payload: &Value) -> String {
        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"RS256","typ":"JWT"}"#);
        let payload_enc = URL_SAFE_NO_PAD.encode(serde_json::to_string(payload).unwrap());
        format!("{}.{}.fakesig", header, payload_enc)
    }

    #[test]
    fn extracts_account_id_from_valid_jwt() {
        let claims = json!({
            "sub": "user-xyz",
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "user-abc123"
            }
        });
        let token = make_jwt(&claims);
        assert_eq!(
            extract_chatgpt_account_id(&token),
            Some("user-abc123".to_string())
        );
    }

    #[test]
    fn returns_none_when_claim_missing() {
        let claims = json!({ "sub": "user-xyz" });
        let token = make_jwt(&claims);
        assert_eq!(extract_chatgpt_account_id(&token), None);
    }

    #[test]
    fn returns_none_for_malformed_token() {
        assert_eq!(extract_chatgpt_account_id("not.a.valid.jwt.at.all"), None);
        assert_eq!(extract_chatgpt_account_id(""), None);
        assert_eq!(extract_chatgpt_account_id("onlyone"), None);
    }

    #[test]
    fn returns_none_when_account_id_empty() {
        let claims = json!({
            "https://api.openai.com/auth": {
                "chatgpt_account_id": ""
            }
        });
        let token = make_jwt(&claims);
        assert_eq!(extract_chatgpt_account_id(&token), None);
    }

    #[test]
    fn tools_to_responses_api_converts_correctly() {
        let schema = vec![
            json!({
                "name": "bash",
                "description": "Run a shell command",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "command": { "type": "string" }
                    },
                    "required": ["command"]
                }
            }),
            // Internal tool — should be filtered out
            json!({ "name": "respond", "description": "Internal", "input_schema": {} }),
        ];

        let result = tools_to_responses_api(&schema);
        assert_eq!(result.len(), 1, "respond tool should be filtered out");
        assert_eq!(result[0]["type"], "function");
        assert_eq!(result[0]["name"], "bash");
        assert_eq!(result[0]["description"], "Run a shell command");
        assert!(result[0]["parameters"].is_object());
        assert!(result[0]["strict"].is_null());
    }

    #[test]
    fn tools_to_responses_api_uses_empty_schema_as_fallback() {
        let schema = vec![json!({ "name": "my_tool" })]; // no input_schema
        let result = tools_to_responses_api(&schema);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["parameters"]["type"], "object");
    }

    #[test]
    fn parse_function_call_item_valid() {
        let item = json!({
            "type": "function_call",
            "id": "call_001",
            "name": "bash",
            "arguments": r#"{"command": "ls -la"}"#
        });
        let block = parse_function_call_item(&item).unwrap();
        assert_eq!(block["type"], "tool_use");
        assert_eq!(block["id"], "call_001");
        assert_eq!(block["name"], "bash");
        assert_eq!(block["input"]["command"], "ls -la");
    }

    #[test]
    fn parse_function_call_item_uses_call_id_fallback() {
        let item = json!({
            "type": "function_call",
            "call_id": "call_fallback",
            "name": "grep",
            "arguments": "{}"
        });
        let block = parse_function_call_item(&item).unwrap();
        assert_eq!(block["id"], "call_fallback");
    }

    #[test]
    fn parse_function_call_item_returns_empty_input_on_bad_json() {
        let item = json!({
            "type": "function_call",
            "id": "call_bad",
            "name": "bash",
            "arguments": "not json at all"
        });
        let block = parse_function_call_item(&item).unwrap();
        // Should degrade gracefully to {}
        assert!(block["input"].is_object());
        assert_eq!(block["input"].as_object().unwrap().len(), 0);
    }

    #[test]
    fn parse_function_call_item_none_without_name() {
        let item = json!({ "type": "function_call", "id": "call_no_name" });
        assert!(parse_function_call_item(&item).is_none());
    }
}
