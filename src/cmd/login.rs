use synaps_cli::auth;

pub async fn run(profile: Option<String>, provider: Option<String>) {
    if let Some(ref prof) = profile {
        synaps_cli::config::set_profile(Some(prof.clone()));
    }

    let _log_guard = synaps_cli::logging::init_logging();

    // If a specific provider was requested, skip the dashboard
    if let Some(ref p) = provider {
        match p.as_str() {
            "anthropic" | "claude" => {
                run_anthropic_login().await;
                return;
            }
            "openai" | "chatgpt" => {
                run_openai_login().await;
                return;
            }
            _ => {
                eprintln!("\x1b[31m✗ Unknown provider: {}\x1b[0m", p);
                eprintln!("  Available: anthropic, openai");
                std::process::exit(1);
            }
        }
    }

    // Show the provider dashboard
    show_dashboard().await;
}

fn show_oauth_status(_name: &str, creds: Option<&auth::OAuthCredentials>) -> String {
    match creds {
        Some(c) if c.auth_type == "oauth" && !c.access.is_empty() => {
            if auth::is_token_expired(c) {
                format!("\x1b[33m⚠ Token expired\x1b[0m")
            } else {
                format!("\x1b[32m✓ Logged in\x1b[0m — expires {}", format_expiry(c.expires))
            }
        }
        _ => format!("\x1b[31m✗ Not logged in\x1b[0m"),
    }
}

async fn show_dashboard() {
    // Load auth state
    let auth_file = auth::load_auth().ok().flatten();
    let anthropic_creds = auth_file.as_ref().map(|a| &a.anthropic);
    let openai_creds = auth_file.as_ref().and_then(|a| a.openai.as_ref());

    // Load API key provider status
    let provider_keys = synaps_cli::config::get_provider_keys();
    let providers = synaps_cli::runtime::openai::registry::list_providers(&provider_keys);

    eprintln!();
    eprintln!("  \x1b[1m╔══════════════════════════════════════════════════╗\x1b[0m");
    eprintln!("  \x1b[1m║              SynapsCLI — Login                   ║\x1b[0m");
    eprintln!("  \x1b[1m╠══════════════════════════════════════════════════╣\x1b[0m");
    eprintln!("  \x1b[1m║\x1b[0m                                                  \x1b[1m║\x1b[0m");
    eprintln!("  \x1b[1m║\x1b[0m  \x1b[1;4mOAuth Providers\x1b[0m                                 \x1b[1m║\x1b[0m");
    eprintln!("  \x1b[1m║\x1b[0m                                                  \x1b[1m║\x1b[0m");
    eprintln!("  \x1b[1m║\x1b[0m   \x1b[1m1.\x1b[0m Anthropic (Claude)                           \x1b[1m║\x1b[0m");
    eprintln!("  \x1b[1m║\x1b[0m      {}   \x1b[1m║\x1b[0m", show_oauth_status("Anthropic", anthropic_creds));
    eprintln!("  \x1b[1m║\x1b[0m                                                  \x1b[1m║\x1b[0m");
    eprintln!("  \x1b[1m║\x1b[0m   \x1b[1m2.\x1b[0m OpenAI (ChatGPT)                             \x1b[1m║\x1b[0m");
    eprintln!("  \x1b[1m║\x1b[0m      {}   \x1b[1m║\x1b[0m", show_oauth_status("OpenAI", openai_creds));
    eprintln!("  \x1b[1m║\x1b[0m                                                  \x1b[1m║\x1b[0m");

    // Show API key providers
    let configured: Vec<_> = providers.iter().filter(|(key, _, has_key, _)| *has_key && *key != "openai").collect();
    let unconfigured: Vec<_> = providers.iter().filter(|(key, _, has_key, _)| !*has_key && *key != "openai").collect();

    if !configured.is_empty() || !unconfigured.is_empty() {
        eprintln!("  \x1b[1m║\x1b[0m  \x1b[1;4mAPI Key Providers\x1b[0m                               \x1b[1m║\x1b[0m");
        for (key, name, _, _) in &configured {
            eprintln!("  \x1b[1m║\x1b[0m   \x1b[32m✓\x1b[0m {:<16} ({})  \x1b[1m║\x1b[0m", name, key);
        }
        // Show first few unconfigured
        for (_key, name, _, _) in unconfigured.iter().take(3) {
            eprintln!("  \x1b[1m║\x1b[0m   \x1b[2m✗ {:<16}\x1b[0m                            \x1b[1m║\x1b[0m", name);
        }
        if unconfigured.len() > 3 {
            eprintln!("  \x1b[1m║\x1b[0m   \x1b[2m  ... and {} more\x1b[0m                          \x1b[1m║\x1b[0m", unconfigured.len() - 3);
        }
        eprintln!("  \x1b[1m║\x1b[0m                                                  \x1b[1m║\x1b[0m");
    }

    eprintln!("  \x1b[1m╚══════════════════════════════════════════════════╝\x1b[0m");
    eprintln!();
    eprint!("  Select a provider to log in (\x1b[1m1\x1b[0m-\x1b[1m2\x1b[0m), or press Enter to cancel: ");

    // Read user input
    let mut input = String::new();
    if std::io::stdin().read_line(&mut input).is_err() {
        return;
    }

    match input.trim() {
        "1" => run_anthropic_login().await,
        "2" => run_openai_login().await,
        "" => {
            eprintln!("  Cancelled.");
        }
        _ => {
            eprintln!("  \x1b[31mInvalid selection.\x1b[0m");
        }
    }
}

async fn run_anthropic_login() {
    eprintln!("\n  \x1b[1mSign in with your Claude account\x1b[0m");
    eprintln!("  (Pro, Max, Team, or Enterprise)\n");

    if let Ok(Some(existing)) = auth::load_auth() {
        if !auth::is_token_expired(&existing.anthropic) {
            eprintln!("  \x1b[33m⚠ Already logged in with a valid token.\x1b[0m");
            eprintln!("    Expires: {}", format_expiry(existing.anthropic.expires));
            eprintln!("    Continuing will replace your current credentials.\n");
        } else {
            eprintln!("  \x1b[33m⚠ Existing token is expired. Logging in fresh.\x1b[0m\n");
        }
    }

    match auth::login().await {
        Ok(creds) => {
            eprintln!("\n  \x1b[32m✓ Anthropic login successful!\x1b[0m");
            eprintln!("    Token saved to: {}", auth::auth_file_path().display());
            eprintln!("    Expires: {}\n", format_expiry(creds.expires));
        }
        Err(e) => {
            eprintln!("\n  \x1b[31m✗ Login failed: {}\x1b[0m\n", e);
            std::process::exit(1);
        }
    }
}

async fn run_openai_login() {
    eprintln!("\n  \x1b[1mSign in with your ChatGPT account\x1b[0m");
    eprintln!("  (Plus, Pro, Team, or Enterprise)\n");

    if let Ok(Some(existing)) = auth::load_openai_auth() {
        if !auth::is_token_expired(&existing) {
            eprintln!("  \x1b[33m⚠ Already logged in with a valid OpenAI token.\x1b[0m");
            eprintln!("    Expires: {}", format_expiry(existing.expires));
            eprintln!("    Continuing will replace your current credentials.\n");
        } else {
            eprintln!("  \x1b[33m⚠ Existing OpenAI token is expired. Logging in fresh.\x1b[0m\n");
        }
    }

    match auth::login_openai().await {
        Ok(creds) => {
            eprintln!("\n  \x1b[32m✓ OpenAI login successful!\x1b[0m");
            eprintln!("    Token saved to: {}", auth::auth_file_path().display());
            eprintln!("    Expires: {}", format_expiry(creds.expires));
            eprintln!("    Use with: \x1b[1m/model openai/gpt-4.1\x1b[0m\n");
        }
        Err(e) => {
            eprintln!("\n  \x1b[31m✗ Login failed: {}\x1b[0m\n", e);
            std::process::exit(1);
        }
    }
}

fn format_expiry(expires_millis: u64) -> String {
    let secs = expires_millis / 1000;
    let now = synaps_cli::epoch_secs();

    if secs <= now {
        return "expired".to_string();
    }

    let remaining = secs - now;
    let days = remaining / 86400;
    let hours = (remaining % 86400) / 3600;

    if days > 0 {
        format!("in {} days, {} hours", days, hours)
    } else if hours > 0 {
        format!("in {} hours", hours)
    } else {
        let mins = remaining / 60;
        format!("in {} minutes", mins)
    }
}
