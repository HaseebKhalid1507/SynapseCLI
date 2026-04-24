use rand::Rng;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use sha2::{Sha256, Digest};

use super::{AUTHORIZE_URL, CLIENT_ID, SCOPES};
use super::{OPENAI_AUTHORIZE_URL, OPENAI_CLIENT_ID, OPENAI_SCOPES, OPENAI_CALLBACK_PATH};

/// Generate a cryptographically random code verifier (43-128 chars, base64url).
pub fn generate_code_verifier() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Compute S256 code challenge from verifier.
pub fn generate_code_challenge(verifier: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let hash = hasher.finalize();
    URL_SAFE_NO_PAD.encode(hash)
}

/// Generate a random state parameter.
pub fn generate_state() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Build the full authorize URL for the browser.
pub fn build_auth_url(challenge: &str, state: &str, port: u16) -> String {
    let redirect_uri = format!("http://localhost:{}/callback", port);
    let params = [
        ("code", "true"),
        ("client_id", CLIENT_ID),
        ("response_type", "code"),
        ("redirect_uri", &redirect_uri),
        ("scope", SCOPES),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
        ("state", state),
    ];

    let query: String = params
        .iter()
        .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
        .collect::<Vec<_>>()
        .join("&");

    format!("{}?{}", AUTHORIZE_URL, query)
}

/// Build the full authorize URL for OpenAI (Sign in with ChatGPT).
pub fn build_openai_auth_url(challenge: &str, state: &str, port: u16) -> String {
    let redirect_uri = format!("http://localhost:{}{}", port, OPENAI_CALLBACK_PATH);
    let params = [
        ("response_type", "code"),
        ("client_id", OPENAI_CLIENT_ID),
        ("redirect_uri", &redirect_uri),
        ("scope", OPENAI_SCOPES),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
        ("state", state),
        ("id_token_add_organizations", "true"),
        ("codex_cli_simplified_flow", "true"),
    ];

    let query: String = params
        .iter()
        .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
        .collect::<Vec<_>>()
        .join("&");

    format!("{}?{}", OPENAI_AUTHORIZE_URL, query)
}
