//! OpenAI-compatible provider engine.
//!
//! Ported from the `openai-runtime` prototype. Translates between Anthropic-shaped
//! messages/tools/content-blocks (the internal synaps representation) and
//! OpenAI `chat/completions` SSE wire.
//!
//! For ChatGPT OAuth users, routes through the Codex Responses API
//! (`chatgpt.com/backend-api/codex/responses`) instead of `/v1/chat/completions`.

use std::collections::BTreeMap;

pub mod types;
pub mod wire;
pub mod registry;
pub mod translate;
pub mod stream;
pub mod codex_stream;
pub mod ping;

pub use types::{
    ChatMessage, ChatOptions, ChatRequest, FunctionCall, FunctionDefinition,
    OaiEvent, ProviderConfig, StreamOptions, ToolCall, ToolChoice, ToolDefinition,
};
pub use wire::StreamDecoder;

/// Routing decision for a given model id.
#[derive(Debug, Clone)]
pub enum Provider {
    /// Native Anthropic path (default, backward-compatible).
    Anthropic,
    /// OpenAI-compatible provider with a resolved config.
    OpenAi(ProviderConfig),
    /// ChatGPT OAuth — uses Codex Responses API at chatgpt.com/backend-api.
    ChatGptOAuth { access_token: String, model: String },
    /// Known provider prefix but no API key configured.
    MissingKey(String),
}

/// Decide which backend a model should route to.
///
/// - `provider/model` shorthand where `provider` matches a known provider key → `OpenAi`
/// - `claude-*` → `Anthropic`
/// - anything else → `Anthropic` (backward compat)
pub fn resolve_route(model: &str, provider_keys: &BTreeMap<String, String>) -> Provider {
    if let Some((prefix, rest)) = model.split_once('/') {
        // Special handling for "openai/" prefix: check if using ChatGPT OAuth
        if prefix == "openai" {
            // Check for an explicit API key first (config or env var)
            let has_api_key = provider_keys.get("openai").map_or(false, |v| !v.is_empty())
                || std::env::var("OPENAI_API_KEY").ok().map_or(false, |v| !v.is_empty());

            if !has_api_key {
                // No API key — check for ChatGPT OAuth credentials
                if let Ok(Some(creds)) = crate::auth::load_openai_auth() {
                    if creds.auth_type == "oauth" && !creds.access.is_empty() {
                        return Provider::ChatGptOAuth {
                            access_token: creds.access,
                            model: rest.to_string(),
                        };
                    }
                }
            }
        }

        if prefix == "local" || registry::providers().iter().any(|s| s.key == prefix) {
            if let Some(cfg) = registry::resolve_shorthand(model, provider_keys) {
                return Provider::OpenAi(cfg);
            }
            // Known provider but no key
            return Provider::MissingKey(prefix.to_string());
        }
    }
    Provider::Anthropic
}

/// Try to route a request through an OpenAI-compatible provider.
///
/// Returns `Some(Ok(value))` if the model resolved to an OpenAI provider and the
/// request completed (successfully or with error). Returns `None` if the model
/// should be handled by the Anthropic path.
///
/// This is the single routing entry point — both streaming and non-streaming
/// callers in `api.rs` use this instead of duplicating the routing logic.
pub async fn try_route(
    model: &str,
    client: &reqwest::Client,
    tools_schema: &std::sync::Arc<Vec<serde_json::Value>>,
    system_prompt: &Option<String>,
    messages: &[serde_json::Value],
    tx: &tokio::sync::mpsc::UnboundedSender<crate::runtime::types::StreamEvent>,
    temperature: Option<f32>,
    max_tokens: Option<u32>,
    cancel: &tokio_util::sync::CancellationToken,
) -> Option<Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>>> {
    let provider_keys = crate::core::config::get_provider_keys();
    match resolve_route(model, &provider_keys) {
        Provider::ChatGptOAuth { mut access_token, model: model_id } => {
            // Refresh the OAuth token if needed before making the call
            if let Ok(fresh_creds) = crate::auth::ensure_fresh_openai_token(client).await {
                access_token = fresh_creds.access;
            }

            let result = codex_stream::call_codex_stream_inner(
                &access_token,
                &model_id,
                client,
                tools_schema,
                system_prompt,
                messages,
                tx,
                cancel,
            ).await;
            Some(result)
        }
        Provider::OpenAi(cfg) => {
            // For OpenAI with API key: just use chat/completions as normal
            let result = stream::call_oai_stream_inner(
                &cfg, client, tools_schema, system_prompt, messages, tx,
                temperature, max_tokens, cancel,
            ).await;
            Some(result)
        }
        Provider::Anthropic => None,
        Provider::MissingKey(provider) => {
            Some(Err(format!(
                "No API key for '{}'. Set provider.{} in ~/.synaps-cli/config or the corresponding env var.",
                provider, provider
            ).into()))
        }
    }
}
