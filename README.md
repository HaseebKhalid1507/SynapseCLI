# SynapsCLI

A minimal, terminal-native AI agent runtime built in Rust. It connects to the Anthropic API, streams responses with extended thinking, executes tools in an autonomous loop, and presents everything through a polished TUI.

## Features

- **Streaming SSE** — Real-time token streaming with thinking block display
- **Tool use loop** — Autonomous multi-step tool execution with cancellation support
- **7 built-in tools** — bash, read, write, edit, grep, find, ls
- **TUI interface** — Full terminal UI with markdown rendering, syntax highlighting, and animations
- **Session persistence** — Auto-saved sessions with `--continue` to resume any conversation
- **Extended thinking** — Configurable thinking budgets (low/medium/high/xhigh or custom token count)
- **Cost tracking** — Per-model pricing with session totals shown in the footer
- **API key auth** — Simple `ANTHROPIC_API_KEY` authentication
- **Prefix commands** — Type `/q` instead of `/quit`, unambiguous prefixes resolve automatically
- **Tab completion** — Tab-complete slash commands with longest common prefix matching

## Quick Start

### Prerequisites

- Rust toolchain (1.70+)
- An Anthropic API key

### Setup

```bash
# Clone and build
git clone <repo-url>
cd SynapsCLI
cargo build --release

# Set your API key
export ANTHROPIC_API_KEY="sk-ant-..."

# Or create a config
mkdir -p ~/.SynapsCLI
echo "model = claude-sonnet-4-20250514" > ~/.SynapsCLI/config
echo "thinking = medium" >> ~/.SynapsCLI/config
```

### Run

```bash
# TUI mode (recommended)
cargo run --bin chatui

# Continue the most recent session
cargo run --bin chatui -- --continue

# Continue a specific session (partial ID match)
cargo run --bin chatui -- --continue 20260410-1430

# Plain streaming chat
cargo run --bin chat

# Single prompt (non-streaming)
cargo run --bin SynapsCLI run "explain quicksort"
```

## Architecture

```
src/
├── lib.rs        # Module exports
├── runtime.rs    # Core runtime: API calls, SSE parsing, tool loop, auth
├── tools.rs      # Tool registry and 7 tool implementations
├── session.rs    # Session persistence: save, load, list, find
├── chatui.rs     # TUI binary (ratatui + crossterm)
├── chat.rs       # Plain text streaming chat binary
├── error.rs      # Error types
└── main.rs       # CLI binary with run/chat subcommands
```

### Runtime (`runtime.rs`)

The `Runtime` struct is the core engine. It handles:

- **Authentication** — Reads `ANTHROPIC_API_KEY` from the environment
- **SSE streaming** — Parses `content_block_start`, `content_block_delta`, `content_block_stop` events, accumulating thinking blocks (with signatures), text, and tool use blocks
- **Tool loop** — When the model returns `tool_use` blocks, executes each tool, sends results back, and continues until the model responds with text only. Updated message history is sent after each iteration so the UI stays in sync.
- **Cancellation** — Accepts a `CancellationToken` that can abort between API calls, between tool executions, or mid-tool via `tokio::select!`. Partial message history is preserved on cancellation.

Public API:

```rust
let runtime = Runtime::new().await?;
runtime.set_model("claude-sonnet-4-20250514".to_string());
runtime.set_thinking_budget(16384);
runtime.set_system_prompt("You are a helpful agent.".to_string());

// Streaming with cancellation
let cancel = CancellationToken::new();
let mut stream = runtime.run_stream_with_messages(messages, cancel.clone());
while let Some(event) = stream.next().await {
    match event {
        StreamEvent::Text(t) => print!("{}", t),
        StreamEvent::Thinking(t) => { /* thinking tokens */ },
        StreamEvent::ToolUse { tool_name, tool_id, input } => { /* tool called */ },
        StreamEvent::ToolResult { tool_id, result } => { /* tool finished */ },
        StreamEvent::MessageHistory(msgs) => { /* updated conversation */ },
        StreamEvent::Usage { input_tokens, output_tokens } => { /* token counts */ },
        StreamEvent::Done => break,
        StreamEvent::Error(e) => eprintln!("{}", e),
    }
}

// Non-streaming (blocks until complete, runs tool loop internally)
let response = runtime.run_single("list files in src/").await?;
```

#### SSE Event Handling

The streaming parser processes these SSE event types:

| Event Type | Description |
|------------|-------------|
| `content_block_start` | Marks beginning of a thinking, text, or tool_use block |
| `content_block_delta` | Incremental content: `text_delta`, `thinking_delta`, `signature_delta`, `input_json_delta` |
| `content_block_stop` | Finalizes the block, flushes accumulated content |
| `message_start` | Message envelope with initial usage data |
| `message_delta` | Token usage updates |
| `message_stop` | Final message marker |

Thinking blocks accumulate both the thinking text and a `signature` field across separate deltas. Both are stored and echoed back in tool loop continuations.

API requests use `max_tokens: 128000` and `thinking.display: "summarized"`.

### Tools (`tools.rs`)

Seven tools are registered by default:

| Tool | Description |
|------|-------------|
| **bash** | Execute shell commands via `/bin/bash -c`. Configurable `timeout` (default 30s, max 300s). Captures stdout + stderr. |
| **read** | Read file contents with line numbers. Supports `offset` (starting line, 0-indexed) and `limit` (max lines). Reports remaining lines when truncated. |
| **write** | Create or overwrite files. Atomic writes via temp file (`.agent-tmp`) + rename. Auto-creates parent directories. Reports line and byte counts. |
| **edit** | Surgical find-and-replace. `old_string` must match exactly once (errors on zero or multiple matches). Atomic write. |
| **grep** | Regex search via `grep -rn`. Supports `include` glob filter (e.g. `*.rs`), `context` lines. Excludes `.git`/`node_modules`/`target`. 15s timeout, 50KB output cap. |
| **find** | Glob-based file search via `find`. Supports `type` filter (`f` for files, `d` for directories). Excludes noise directories. 10s timeout. |
| **ls** | Directory listing via `ls -lah`. Shows permissions, size, and dates. |

All tools expand `~` to `$HOME`. Tool results are streamed back to the TUI as they complete.

#### Tool Parameters

<details>
<summary>Full parameter reference</summary>

**bash**
| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `command` | string | yes | Shell command to execute |
| `timeout` | integer | no | Timeout in seconds (default: 30, max: 300) |

**read**
| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `path` | string | yes | File path to read |
| `offset` | integer | no | Line number to start from (0-indexed) |
| `limit` | integer | no | Maximum number of lines to read |

**write**
| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `path` | string | yes | File path to write |
| `content` | string | yes | File content |

**edit**
| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `path` | string | yes | File path to edit |
| `old_string` | string | yes | Exact text to find (must match once) |
| `new_string` | string | yes | Replacement text |

**grep**
| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `pattern` | string | yes | Regex pattern |
| `path` | string | no | Search directory (default: `.`) |
| `include` | string | no | Glob filter (e.g. `*.rs`) |
| `context` | integer | no | Context lines before and after matches |

**find**
| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `pattern` | string | yes | Glob pattern to match |
| `path` | string | no | Search directory (default: `.`) |
| `type` | string | no | `f` for files, `d` for directories |

**ls**
| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `path` | string | no | Directory to list (default: `.`) |

</details>

### Sessions (`session.rs`)

Every conversation is automatically saved to `~/.SynapsCLI/sessions/`. Session files are JSON containing the full API message history, model settings, token counts, and cost. Empty sessions (no user messages) are not saved.

Session ID format: `YYYYMMDD-HHMMSS-XXXX` (4-character UUID suffix).

Sessions are automatically titled from the first 80 characters of the first user message.

```bash
# Continue the most recent session (no ID needed)
cargo run --bin chatui -- --continue

# Continue a specific session (partial ID match)
cargo run --bin chatui -- --continue 20260410-1430
```

Session functions:

- `Session::new()` — Create with current model/thinking/system prompt
- `Session::save()` / `Session::load()` — Persist to / read from disk
- `Session::auto_title()` — Set title from first user message
- `list_sessions()` — All sessions sorted by last updated
- `latest_session()` — Most recently active session
- `find_session("partial_id")` — Exact match first, then substring match. Errors on ambiguous matches.

### TUI (`chatui.rs`)

The terminal interface built with ratatui, crossterm, tachyonfx, and syntect.

**Keyboard shortcuts:**

| Key | Action |
|-----|--------|
| `Enter` | Send message |
| `Esc` | Abort streaming (cancels tool execution too) |
| `Ctrl+C` | Quit (with dissolve animation) |
| `Ctrl+A` / `Home` | Move cursor to start |
| `Ctrl+E` / `End` | Move cursor to end |
| `Ctrl+W` | Delete word backward |
| `Ctrl+U` | Delete to start of line |
| `Alt+Left` / `Alt+Right` | Move cursor by word |
| `Alt+Backspace` | Delete word backward |
| `Up` / `Down` | Input history navigation |
| `Shift+Up` / `Shift+Down` | Scroll message history |
| `Mouse wheel` | Scroll messages (3 lines per tick) |
| `Tab` | Autocomplete slash commands (longest common prefix) |

**Slash commands:**

Commands support unambiguous prefix matching — `/q` resolves to `/quit`, `/s` is ambiguous (sessions/system) so it won't resolve, but `/se` resolves to `/sessions`.

| Command | Description |
|---------|-------------|
| `/clear` | Save current session and start a new one |
| `/model [name]` | Show or set model |
| `/system <prompt\|show\|save>` | Manage system prompt |
| `/thinking [low\|medium\|high\|xhigh]` | Set thinking budget |
| `/sessions` | List saved sessions (up to 20, with active marker) |
| `/resume <id>` | Save current session and switch to another |
| `/help` | Show available commands |
| `/quit` | Exit |
| `/exit` | Exit (alias for `/quit`) |

**Rendering:**

- Markdown: headings, bold, italic, inline code, blockquotes, ordered/unordered lists
- Code blocks: syntax highlighted via syntect (base16-ocean.dark theme)
- Tool calls: per-tool icons (❯ bash, ▸ read, ◂ write, Δ edit, ⌕ grep, ○ find, ≡ ls)
- Tool results: compact "└─ ok (N lines)" summary with indented output, errors in red
- Thinking: dimmed with │ left border, truncated to 6 lines
- Animations: fade-in on boot (300ms, QuadOut), dissolve on exit (800ms, QuadIn) via tachyonfx

**Theme:**

Dark base (RGB 12,14,18) with teal accents (RGB 80,200,160). User messages have a subtle background highlight. The color palette is designed for high contrast on dark terminals.

## Configuration

### Config file

`~/.SynapsCLI/config` — key=value format, supports `#` comments:

```
# Model selection
model = claude-opus-4-6

# Thinking budget (named level or raw token count)
thinking = xhigh
# thinking = 8192
```

**Thinking levels:**

| Level | Budget tokens |
|-------|--------------|
| `low` | 2,048 |
| `medium` | 4,096 |
| `high` | 16,384 |
| `xhigh` | 32,768 |
| `<number>` | Custom token count |

### System prompt

`~/.SynapsCLI/system.md` — loaded on startup. Can also be set at runtime with `/system`.

Default (when no file exists):
> You are a helpful AI agent running in a terminal. You have access to bash, read, and write tools. Be concise and direct. Use tools when the user asks you to interact with the filesystem or run commands.

### Authentication

Set `ANTHROPIC_API_KEY` in your environment. The runtime sends it via the `x-api-key` header with `anthropic-version: 2023-06-01`.

## Cost Tracking

Session cost is calculated per API call using current Anthropic pricing:

| Model | Input (per MTok) | Output (per MTok) |
|-------|-----------------|-------------------|
| Opus | $15.00 | $75.00 |
| Sonnet | $3.00 | $15.00 |
| Haiku | $0.80 | $4.00 |

Model matching is substring-based (e.g. any model ID containing "opus" uses Opus pricing). Unknown models default to Sonnet pricing.

The running total is displayed in the footer and persisted with each session.

## Dependencies

| Crate | Purpose |
|-------|---------|
| tokio | Async runtime |
| reqwest | HTTP client with streaming |
| serde / serde_json | Serialization |
| clap | CLI argument parsing |
| ratatui | Terminal UI framework |
| crossterm | Terminal backend + input events |
| tachyonfx | Terminal animations (fade-in, dissolve) |
| syntect | Syntax highlighting for code blocks |
| chrono | Timestamps |
| uuid | Session ID generation |
| tokio-util | CancellationToken for streaming abort |
| thiserror | Error derive macros |

## License

MIT
