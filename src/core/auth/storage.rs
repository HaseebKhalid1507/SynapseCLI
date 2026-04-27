use std::path::PathBuf;

use super::{AuthFile, OAuthCredentials};

/// Get the path to auth.json (~/.synaps-cli/auth.json).
pub fn auth_file_path() -> PathBuf {
    crate::config::resolve_read_path("auth.json")
}

/// Load credentials from auth.json.
pub fn load_auth() -> std::result::Result<Option<AuthFile>, String> {
    let path = auth_file_path();
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
    let auth: AuthFile = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse {}: {}", path.display(), e))?;
    Ok(Some(auth))
}

/// Load one provider's OAuth credential from auth.json.
pub fn load_provider_auth(provider: &str) -> std::result::Result<Option<OAuthCredentials>, String> {
    let path = auth_file_path();
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
    let value: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse {}: {}", path.display(), e))?;
    let Some(raw) = value.get(provider) else {
        return Ok(None);
    };
    let creds: OAuthCredentials = serde_json::from_value(raw.clone())
        .map_err(|e| format!("Failed to parse {} credential: {}", provider, e))?;
    Ok(Some(creds))
}

/// Save credentials to auth.json.
pub fn save_auth(creds: &OAuthCredentials) -> std::result::Result<(), String> {
    save_provider_auth("anthropic", creds)
}

/// Save one provider credential while preserving other auth.json entries.
pub fn save_provider_auth(provider: &str, creds: &OAuthCredentials) -> std::result::Result<(), String> {
    let path = crate::config::resolve_write_path("auth.json");

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create {}: {}", parent.display(), e))?;
    }

    let mut root = if path.exists() {
        let content = std::fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
        serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&content)
            .map_err(|e| format!("Failed to parse {}: {}", path.display(), e))?
    } else {
        serde_json::Map::new()
    };

    root.insert(
        provider.to_string(),
        serde_json::to_value(creds).map_err(|e| format!("Failed to serialize auth: {}", e))?,
    );

    let json = serde_json::to_string_pretty(&root)
        .map_err(|e| format!("Failed to serialize auth: {}", e))?;

    std::fs::write(&path, &json)
        .map_err(|e| format!("Failed to write {}: {}", path.display(), e))?;

    // Set file permissions to 600 (owner read/write only)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(&path, perms)
            .map_err(|e| format!("Failed to set permissions on {}: {}", path.display(), e))?;
    }

    Ok(())
}
