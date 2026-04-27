use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::oneshot;

use super::{
    generate_code_challenge, generate_code_verifier, generate_state, open_browser, save_provider_auth,
    start_callback_server, CallbackResult, OAuthCredentials,
};

const PROVIDER: &str = "openai-codex";
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const CALLBACK_PORT: u16 = 1455;
const REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
const SCOPE: &str = "openid profile email offline_access";
const JWT_CLAIM_PATH: &str = "https://api.openai.com/auth";

#[derive(Debug, Deserialize)]
struct CodexTokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: u64,
}

pub async fn login() -> std::result::Result<OAuthCredentials, String> {
    let verifier = generate_code_verifier();
    let challenge = generate_code_challenge(&verifier);
    let state = generate_state();
    let (rx, server_handle) = start_callback_server(state.clone(), CALLBACK_PORT).await?;
    let auth_url = build_auth_url(&challenge, &state)?;

    eprintln!("\n\x1b[1mOpening browser to sign in with ChatGPT...\x1b[0m\n");
    if let Err(e) = open_browser(&auth_url) {
        eprintln!("Could not open browser automatically: {}", e);
    }
    eprintln!("\x1b[2mIf the browser didn't open, visit this URL:\x1b[0m");
    eprintln!("\x1b[36m{}\x1b[0m\n", auth_url);

    let (manual_tx, manual_rx) = oneshot::channel::<CallbackResult>();
    let manual_state = state.clone();
    let stdin_task = tokio::spawn(async move {
        eprintln!("\x1b[2mOr paste the authorization code or redirect URL here:\x1b[0m");
        let mut line = String::new();
        let result = tokio::task::spawn_blocking(move || {
            std::io::stdin().read_line(&mut line).ok();
            line.trim().to_string()
        })
        .await;

        if let Ok(input) = result {
            if let Some((code, parsed_state)) = parse_authorization_input(&input) {
                let _ = manual_tx.send(CallbackResult {
                    code,
                    state: parsed_state.unwrap_or(manual_state),
                });
            }
        }
    });

    let result = tokio::select! {
        callback = rx => callback.map_err(|_| "Callback channel closed".to_string())?,
        manual = manual_rx => manual.map_err(|_| "Manual input channel closed".to_string())?,
    };
    stdin_task.abort();

    if result.state != state {
        server_handle.shutdown().await;
        return Err("OAuth state mismatch -- possible CSRF attack".to_string());
    }

    eprintln!("\n\x1b[1mExchanging code for tokens...\x1b[0m");
    let creds = exchange_code_for_tokens(&result.code, &verifier).await?;
    server_handle.shutdown().await;
    save_provider_auth(PROVIDER, &creds)?;
    Ok(creds)
}

pub async fn refresh_token(client: &Client, refresh: &str) -> std::result::Result<OAuthCredentials, String> {
    let params = [
        ("grant_type", "refresh_token"),
        ("client_id", CLIENT_ID),
        ("refresh_token", refresh),
    ];
    let token = token_request(client, &params).await?;
    credentials_from_token(token)
}

async fn exchange_code_for_tokens(
    code: &str,
    verifier: &str,
) -> std::result::Result<OAuthCredentials, String> {
    let client = Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;
    let params = [
        ("grant_type", "authorization_code"),
        ("client_id", CLIENT_ID),
        ("code", code),
        ("code_verifier", verifier),
        ("redirect_uri", REDIRECT_URI),
    ];
    let token = token_request(&client, &params).await?;
    credentials_from_token(token)
}

async fn token_request(
    client: &Client,
    params: &[(&str, &str)],
) -> std::result::Result<CodexTokenResponse, String> {
    let resp = client
        .post(TOKEN_URL)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("Token request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Token request failed ({}): {}", status, text));
    }

    resp.json()
        .await
        .map_err(|e| format!("Failed to parse token response: {}", e))
}

fn credentials_from_token(token: CodexTokenResponse) -> std::result::Result<OAuthCredentials, String> {
    let account_id = extract_account_id(&token.access_token)
        .ok_or_else(|| "Failed to extract ChatGPT account id from token".to_string())?;
    Ok(OAuthCredentials {
        auth_type: "oauth".to_string(),
        refresh: token.refresh_token,
        access: token.access_token,
        expires: crate::epoch_millis() + (token.expires_in * 1000) - (5 * 60 * 1000),
        account_id: Some(account_id),
    })
}

fn build_auth_url(challenge: &str, state: &str) -> std::result::Result<String, String> {
    let mut url = url::Url::parse(AUTHORIZE_URL).map_err(|e| e.to_string())?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", CLIENT_ID)
        .append_pair("redirect_uri", REDIRECT_URI)
        .append_pair("scope", SCOPE)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", state)
        .append_pair("id_token_add_organizations", "true")
        .append_pair("codex_cli_simplified_flow", "true")
        .append_pair("originator", "synaps");
    Ok(url.to_string())
}

fn parse_authorization_input(input: &str) -> Option<(String, Option<String>)> {
    let value = input.trim();
    if value.is_empty() {
        return None;
    }
    if let Ok(url) = url::Url::parse(value) {
        let code = url.query_pairs().find(|(k, _)| k == "code").map(|(_, v)| v.to_string())?;
        let state = url.query_pairs().find(|(k, _)| k == "state").map(|(_, v)| v.to_string());
        return Some((code, state));
    }
    if let Some((code, state)) = value.split_once('#') {
        if !code.is_empty() {
            return Some((code.to_string(), Some(state.to_string())));
        }
    }
    if value.contains("code=") {
        let params = url::form_urlencoded::parse(value.as_bytes());
        let mut code = None;
        let mut state = None;
        for (key, val) in params {
            match key.as_ref() {
                "code" => code = Some(val.to_string()),
                "state" => state = Some(val.to_string()),
                _ => {}
            }
        }
        return code.map(|code| (code, state));
    }
    Some((value.to_string(), None))
}

pub fn extract_account_id(access_token: &str) -> Option<String> {
    let payload = access_token.split('.').nth(1)?;
    let decoded = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let json: Value = serde_json::from_slice(&decoded).ok()?;
    json.get(JWT_CLAIM_PATH)?
        .get("chatgpt_account_id")?
        .as_str()
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};

    #[test]
    fn parses_redirect_url() {
        let parsed = parse_authorization_input("http://localhost:1455/auth/callback?code=abc&state=xyz").unwrap();
        assert_eq!(parsed.0, "abc");
        assert_eq!(parsed.1.as_deref(), Some("xyz"));
    }

    #[test]
    fn extracts_account_id_from_jwt() {
        let payload = serde_json::json!({
            JWT_CLAIM_PATH: { "chatgpt_account_id": "acct_123" }
        });
        let token = format!(
            "x.{}.y",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).unwrap())
        );
        assert_eq!(extract_account_id(&token).as_deref(), Some("acct_123"));
    }
}
