use serde_json::{json, Value};
use crate::{Runtime, Result, RuntimeError, StreamEvent};

/// Compaction configuration — loaded from ~/.synaps-cli/config
#[derive(Debug, Clone)]
pub struct CompactionConfig {
    /// Whether compaction is enabled
    pub enabled: bool,
    /// Model to use for summarization (cheap model recommended)
    pub model: String,
    /// Thinking budget for the summarization model
    pub thinking_budget: u32,
    /// Estimated token count threshold to trigger compaction
    pub threshold: usize,
    /// Number of recent turns to keep intact (not compacted)
    pub keep_recent: usize,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            model: "claude-sonnet-4-20250514".to_string(),
            thinking_budget: 2048,
            threshold: 80_000,
            keep_recent: 6,
        }
    }
}

impl CompactionConfig {
    /// Parse compaction settings from config key-value pairs.
    /// Called during config loading — keys are prefixed with "compaction".
    pub fn apply(&mut self, key: &str, val: &str) {
        match key {
            "compaction" => {
                self.enabled = matches!(val, "true" | "on" | "1" | "yes");
            }
            "compaction_model" => {
                self.model = val.to_string();
            }
            "compaction_thinking" => {
                self.thinking_budget = match val {
                    "low" => 1024,
                    "medium" => 2048,
                    "high" => 4096,
                    _ => val.parse().unwrap_or(2048),
                };
            }
            "compaction_threshold" => {
                self.threshold = val.parse().unwrap_or(80_000);
            }
            "compaction_keep_recent" => {
                self.keep_recent = val.parse().unwrap_or(6);
            }
            _ => {}
        }
    }
}

/// Rough token estimate: ~4 chars per token for English text.
/// Not exact, but good enough for threshold checks.
pub fn estimate_tokens(messages: &[Value]) -> usize {
    let mut total_chars = 0usize;
    for msg in messages {
        if let Some(content) = msg["content"].as_str() {
            total_chars += content.len();
        } else if let Some(arr) = msg["content"].as_array() {
            for block in arr {
                if let Some(text) = block["text"].as_str() {
                    total_chars += text.len();
                }
                if let Some(text) = block["content"].as_str() {
                    total_chars += text.len();
                }
                // Tool inputs
                if let Some(input) = block["input"].as_object() {
                    total_chars += serde_json::to_string(input).unwrap_or_default().len();
                }
            }
        }
    }
    total_chars / 4
}

/// Check if compaction should trigger.
pub fn should_compact(config: &CompactionConfig, messages: &[Value]) -> bool {
    if !config.enabled {
        return false;
    }
    // Need enough messages to compact (at least keep_recent + something to compact)
    if messages.len() < config.keep_recent + 4 {
        return false;
    }
    estimate_tokens(messages) >= config.threshold
}

/// The compaction summary prompt — adapted from Microsoft Memento's STATE-COMPRESSOR.
const COMPACTION_SYSTEM_PROMPT: &str = r#"You are a STATE-COMPRESSOR for AI agent conversation histories.

You receive a segment of a conversation between a user and an AI agent. The conversation includes user messages, agent thinking, agent responses, tool calls (bash, read, write, edit, grep, find, ls), and tool results.

Your ONLY job is to produce an extremely information-dense summary that captures ALL state from this conversation segment.

Think of this as a "compressed state update" — a future agent will see ONLY your summary (not the original conversation) and must be able to continue the work seamlessly.

CORE OBJECTIVE: Minimize tokens while preserving ALL logically relevant information.

"Logically relevant" includes:
- Files read and their key contents/structure
- Files written or edited and what changed
- Commands run and their essential outputs
- Decisions made and reasoning behind them
- Errors encountered and how they were resolved
- Current state of any ongoing task
- User preferences or instructions that affect future work
- Any TODO items or unfinished work

You MUST NOT:
- Omit any fact needed to continue the work
- Invent information not in the conversation
- Include full file contents (summarize what matters)
- Include full command outputs (capture the key result)

STYLE:
- Terse, dense, factual — not literary
- "file: key_finding" format over prose
- Semicolons to chain related facts
- State what WAS DONE, not what was discussed
- Lead with outcomes, not process

OUTPUT FORMAT:
Respond with ONLY the summary text. No headers, no markers, no metadata. Just the dense summary paragraph(s)."#;

/// Run compaction on old messages. Returns new message array with old turns replaced by summary.
///
/// Layout after compaction:
/// [system_context_msg (summary of old turns)] + [recent_turns (kept intact)]
pub async fn compact_messages(
    config: &CompactionConfig,
    messages: &[Value],
    tx: Option<&tokio::sync::mpsc::UnboundedSender<StreamEvent>>,
) -> Result<Vec<Value>> {
    let total = messages.len();
    let keep = config.keep_recent.min(total);
    let compact_end = total - keep;

    if compact_end < 2 {
        // Nothing meaningful to compact
        return Ok(messages.to_vec());
    }

    let old_messages = &messages[..compact_end];
    let recent_messages = &messages[compact_end..];

    // Build the conversation text to summarize
    let mut conversation_text = String::new();
    for msg in old_messages {
        let role = msg["role"].as_str().unwrap_or("unknown");
        if let Some(content) = msg["content"].as_str() {
            conversation_text.push_str(&format!("[{}]: {}\n\n", role, content));
        } else if let Some(arr) = msg["content"].as_array() {
            conversation_text.push_str(&format!("[{}]:\n", role));
            for block in arr {
                match block["type"].as_str() {
                    Some("text") => {
                        if let Some(text) = block["text"].as_str() {
                            conversation_text.push_str(&format!("  {}\n", text));
                        }
                    }
                    Some("thinking") => {
                        if let Some(text) = block["thinking"].as_str() {
                            // Truncate thinking to avoid blowing up the summarization context
                            let preview: String = text.chars().take(500).collect();
                            conversation_text.push_str(&format!("  [thinking]: {}...\n", preview));
                        }
                    }
                    Some("tool_use") => {
                        let name = block["name"].as_str().unwrap_or("?");
                        let input = serde_json::to_string(&block["input"]).unwrap_or_default();
                        // Truncate long tool inputs
                        let input_preview: String = input.chars().take(300).collect();
                        conversation_text.push_str(&format!("  [tool_use: {}] {}\n", name, input_preview));
                    }
                    Some("tool_result") => {
                        if let Some(content) = block["content"].as_str() {
                            let preview: String = content.chars().take(500).collect();
                            conversation_text.push_str(&format!("  [tool_result]: {}\n", preview));
                        }
                    }
                    _ => {}
                }
            }
            conversation_text.push('\n');
        }
    }

    let before_tokens = estimate_tokens(messages);
    tracing::info!("Compaction: summarizing {} messages ({} est tokens), keeping {} recent",
        compact_end, before_tokens, keep);

    // Call the summarization model
    let mut summarizer = Runtime::new().await
        .map_err(|e| RuntimeError::Tool(format!("Compaction: failed to create runtime: {}", e)))?;
    summarizer.set_model(config.model.clone());
    summarizer.set_thinking_budget(config.thinking_budget);
    summarizer.set_system_prompt(COMPACTION_SYSTEM_PROMPT.to_string());
    // Summarizer doesn't need tools
    summarizer.set_tools(crate::ToolRegistry::without_subagent());

    let prompt = format!(
        "Summarize the following conversation segment. This is turns 1-{} of a longer session. \
         Capture all state needed for an agent to continue the work.\n\n{}",
        compact_end, conversation_text
    );

    let summary = summarizer.run_single(&prompt).await
        .map_err(|e| RuntimeError::Tool(format!("Compaction: summarization failed: {}", e)))?;

    // Build the compacted message array
    let mut compacted = Vec::new();

    // Insert summary as a user message with clear framing
    compacted.push(json!({
        "role": "user",
        "content": format!(
            "[COMPACTED SESSION CONTEXT — turns 1-{}, summarized to save context space]\n\n{}\n\n[END COMPACTED CONTEXT]",
            compact_end, summary
        )
    }));

    // Placeholder assistant ack so message alternation is valid
    compacted.push(json!({
        "role": "assistant",
        "content": "Understood. I have the full context from the compacted session history and will continue seamlessly."
    }));

    // Append recent messages intact
    compacted.extend_from_slice(recent_messages);

    let after_tokens = estimate_tokens(&compacted);
    let saved = before_tokens.saturating_sub(after_tokens);
    tracing::info!("Compaction complete: {} → {} est tokens (saved ~{})", before_tokens, after_tokens, saved);

    // Notify TUI
    if let Some(tx) = tx {
        let _ = tx.send(StreamEvent::CompactionDone {
            before_tokens,
            after_tokens,
        });
    }

    Ok(compacted)
}
