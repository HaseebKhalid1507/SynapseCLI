use synaps_cli::{Runtime, Result};
use serde_json::json;
use std::time::Instant;

fn load_agent_prompt(name: &str) -> std::result::Result<String, String> {
    synaps_cli::tools::resolve_agent_prompt(name)
}

pub async fn run(prompt: String, agent: Option<String>, system: Option<String>, model: Option<String>, print: bool) -> Result<()> {
    let _log_guard = synaps_cli::logging::init_logging();
    let mut runtime = Runtime::new().await?;

    // Override model if specified
    if let Some(ref m) = model {
        runtime.set_model(m.clone());
    }

    if let Some(ref agent_name) = agent {
        match load_agent_prompt(agent_name) {
            Ok(p) => {
                if !print {
                    eprintln!("🎭 Agent: {}", agent_name);
                }
                runtime.set_system_prompt(p);
            }
            Err(e) => {
                eprintln!("❌ {}", e);
                std::process::exit(1);
            }
        }
    } else if let Some(ref val) = system {
        let prompt = synaps_cli::config::resolve_system_prompt(Some(val));
        if !print {
            eprintln!("📋 System prompt loaded");
        }
        runtime.set_system_prompt(prompt);
    }

    if !print {
        println!("🤖 Calling {}...", runtime.model());
    }

    let start = Instant::now();
    let response = runtime.run_single(&prompt).await?;
    let elapsed_ms = start.elapsed().as_millis() as u64;

    if print {
        // Structured JSON output for benchmarks
        let output = json!({
            "type": "result",
            "model": runtime.model(),
            "response": response,
            "elapsed_ms": elapsed_ms,
        });
        println!("{}", serde_json::to_string(&output).unwrap());
    } else {
        println!("{}", response);
    }

    Ok(())
}
