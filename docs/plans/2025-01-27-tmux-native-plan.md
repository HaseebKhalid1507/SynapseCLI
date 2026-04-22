# tmux Native Enhancements — Implementation Plan

**Goal:** Add tmux native integration to Synaps via control mode, with new agent tools, configurable layouts, and subagent display modes.
**Architecture:** TmuxController manages a persistent control mode connection; 6 new tools give the agent direct tmux control; config supports system/project-level tmux settings.
**Design Doc:** `docs/plans/2025-01-27-tmux-native-design.md`
**Estimated Tasks:** 42 tasks across 10 batches
**Complexity:** Large

---

## Batch 1: Foundation — Config & Detection (Tasks 1–5)

### Task 1: Add TmuxConfig struct

**Files:**
- Create: `src/tmux/config.rs`
- Create: `src/tmux/mod.rs`
- Modify: `src/lib.rs`

**Step 1: Write failing test**
```rust
// src/tmux/config.rs — at the bottom
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tmux_config_default() {
        let config = TmuxConfig::default();
        assert!(!config.enabled);
        assert!(config.session_name.is_none());
        assert_eq!(config.default_layout, LayoutPreset::Split);
        assert_eq!(config.subagent_display, SubagentDisplay::Pane);
        assert!(config.mouse);
        assert_eq!(config.split_ratio, 60);
    }
}
```

**Step 2: Verify it fails**
Run: `cargo test tmux::config::tests::test_tmux_config_default -- --nocapture 2>&1 | tail -5`
Expected: FAIL — module `tmux` not found

**Step 3: Implement**
```rust
// src/tmux/config.rs
//! tmux integration configuration.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq)]
pub enum LayoutPreset {
    Split,
    Fullscreen,
    Tiled,
    Custom(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum SubagentDisplay {
    Window,
    Pane,
}

#[derive(Debug, Clone)]
pub struct TmuxConfig {
    pub enabled: bool,
    pub session_name: Option<String>,
    pub default_layout: LayoutPreset,
    pub subagent_display: SubagentDisplay,
    pub mouse: bool,
    pub split_ratio: u32,
    pub shell_position: String,
}

impl Default for TmuxConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            session_name: None,
            default_layout: LayoutPreset::Split,
            subagent_display: SubagentDisplay::Pane,
            mouse: true,
            split_ratio: 60,
            shell_position: "right".to_string(),
        }
    }
}
```

```rust
// src/tmux/mod.rs
//! tmux native integration — control mode, layouts, tools.

pub mod config;

pub use config::TmuxConfig;
```

Add to `src/lib.rs`:
```rust
pub mod tmux;
```

**Step 4: Verify it passes**
Run: `cargo test tmux::config::tests::test_tmux_config_default -- --nocapture 2>&1 | tail -5`
Expected: PASS

**Step 5: Commit**
```bash
git add -A && git commit -m "feat(tmux): add TmuxConfig struct with defaults"
```

---

### Task 2: Wire TmuxConfig into SynapsConfig

**Files:**
- Modify: `src/core/config.rs`

**Step 1: Write failing test**
```rust
// Add to src/core/config.rs #[cfg(test)] mod tests
#[test]
fn test_synaps_config_has_tmux() {
    let config = SynapsConfig::default();
    assert!(!config.tmux.enabled);
    assert!(config.tmux.session_name.is_none());
}
```

**Step 2: Verify it fails**
Run: `cargo test core::config::tests::test_synaps_config_has_tmux -- --nocapture 2>&1 | tail -5`
Expected: FAIL — no field `tmux` on type `SynapsConfig`

**Step 3: Implement**

Add to `SynapsConfig` struct (after `pub shell: ShellConfig`):
```rust
    pub tmux: crate::tmux::TmuxConfig,
```

Add to `impl Default for SynapsConfig` (after `shell: ShellConfig::default()`):
```rust
    tmux: crate::tmux::TmuxConfig::default(),
```

**Step 4: Verify it passes**
Run: `cargo test core::config::tests::test_synaps_config_has_tmux -- --nocapture 2>&1 | tail -5`
Expected: PASS

**Step 5: Commit**
```bash
git add -A && git commit -m "feat(tmux): wire TmuxConfig into SynapsConfig"
```

---

### Task 3: Parse tmux config keys from config file

**Files:**
- Modify: `src/core/config.rs`

**Step 1: Write failing test**
```rust
// Add to src/core/config.rs #[cfg(test)] mod tests
#[test]
fn test_parse_tmux_config_keys() {
    use crate::tmux::config::{LayoutPreset, SubagentDisplay};
    let mut tmux_config = crate::tmux::TmuxConfig::default();
    
    parse_tmux_config_key(&mut tmux_config, "tmux.enabled", "true");
    assert!(tmux_config.enabled);
    
    parse_tmux_config_key(&mut tmux_config, "tmux.session_name", "my-project");
    assert_eq!(tmux_config.session_name, Some("my-project".to_string()));
    
    parse_tmux_config_key(&mut tmux_config, "tmux.default_layout", "fullscreen");
    assert_eq!(tmux_config.default_layout, LayoutPreset::Fullscreen);
    
    parse_tmux_config_key(&mut tmux_config, "tmux.subagent_display", "window");
    assert_eq!(tmux_config.subagent_display, SubagentDisplay::Window);
    
    parse_tmux_config_key(&mut tmux_config, "tmux.mouse", "false");
    assert!(!tmux_config.mouse);
    
    parse_tmux_config_key(&mut tmux_config, "tmux.split_ratio", "70");
    assert_eq!(tmux_config.split_ratio, 70);
}
```

**Step 2: Verify it fails**
Run: `cargo test core::config::tests::test_parse_tmux_config_keys -- --nocapture 2>&1 | tail -5`
Expected: FAIL — `parse_tmux_config_key` not found

**Step 3: Implement**

Add function in `src/core/config.rs` (next to `parse_shell_config_key`):
```rust
fn parse_tmux_config_key(config: &mut crate::tmux::TmuxConfig, key: &str, val: &str) {
    let sub_key = key.strip_prefix("tmux.").unwrap_or(key);
    match sub_key {
        "enabled" => config.enabled = val.eq_ignore_ascii_case("true"),
        "session_name" => config.session_name = Some(val.to_string()),
        "default_layout" => {
            config.default_layout = match val.to_lowercase().as_str() {
                "split" => crate::tmux::config::LayoutPreset::Split,
                "fullscreen" => crate::tmux::config::LayoutPreset::Fullscreen,
                "tiled" => crate::tmux::config::LayoutPreset::Tiled,
                other => crate::tmux::config::LayoutPreset::Custom(other.to_string()),
            };
        }
        "subagent_display" => {
            config.subagent_display = match val.to_lowercase().as_str() {
                "window" => crate::tmux::config::SubagentDisplay::Window,
                _ => crate::tmux::config::SubagentDisplay::Pane,
            };
        }
        "mouse" => config.mouse = val.eq_ignore_ascii_case("true"),
        "split_ratio" => {
            if let Ok(ratio) = val.parse::<u32>() {
                if ratio > 0 && ratio < 100 {
                    config.split_ratio = ratio;
                }
            }
        }
        "shell_position" => {
            config.shell_position = match val.to_lowercase().as_str() {
                "bottom" => "bottom".to_string(),
                _ => "right".to_string(),
            };
        }
        _ => {} // silently ignore unknown tmux keys
    }
}
```

Add dispatch arm in `load_config()` match block (in the `_ =>` fallthrough, alongside `shell.*`):
```rust
            _ => {
                if key.starts_with("shell.") {
                    parse_shell_config_key(&mut config.shell, key, val);
                } else if key.starts_with("tmux.") {
                    parse_tmux_config_key(&mut config.tmux, key, val);
                }
            }
```

**Step 4: Verify it passes**
Run: `cargo test core::config::tests::test_parse_tmux_config_keys -- --nocapture 2>&1 | tail -5`
Expected: PASS

**Step 5: Commit**
```bash
git add -A && git commit -m "feat(tmux): parse tmux.* config keys"
```

---

### Task 4: Add --tmux CLI flag

**Files:**
- Modify: `src/main.rs`

**Step 1: Write failing test**
```rust
// src/tmux/mod.rs — add test
#[cfg(test)]
mod tests {
    #[test]
    fn test_auto_session_name_in_git_repo() {
        // Test the auto-naming function
        let name = super::auto_session_name();
        // Should return synaps-<something> — either repo name or dir name
        assert!(name.starts_with("synaps-"));
    }
}
```

**Step 2: Verify it fails**
Run: `cargo test tmux::tests::test_auto_session_name_in_git_repo -- --nocapture 2>&1 | tail -5`
Expected: FAIL — `auto_session_name` not found

**Step 3: Implement**

Add to `src/tmux/mod.rs`:
```rust
use std::process::Command as StdCommand;

/// Generate an auto session name based on git repo or directory name.
pub fn auto_session_name() -> String {
    // Try git repo name first
    if let Ok(output) = StdCommand::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
    {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if let Some(name) = std::path::Path::new(&path).file_name() {
                return format!("synaps-{}", name.to_string_lossy());
            }
        }
    }
    // Fall back to current directory name
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(name) = cwd.file_name() {
            return format!("synaps-{}", name.to_string_lossy());
        }
    }
    // Last resort
    format!("synaps-{}", std::process::id())
}

/// Check if tmux is available in PATH.
pub fn find_tmux() -> Option<String> {
    StdCommand::new("which")
        .arg("tmux")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}
```

Add to `src/main.rs` Cli struct:
```rust
    /// Launch in tmux mode with optional session name.
    #[arg(long = "tmux", value_name = "SESSION_NAME")]
    tmux: Option<Option<String>>,
```

**Step 4: Verify it passes**
Run: `cargo test tmux::tests::test_auto_session_name_in_git_repo -- --nocapture 2>&1 | tail -5`
Expected: PASS

**Step 5: Commit**
```bash
git add -A && git commit -m "feat(tmux): add --tmux CLI flag and auto session naming"
```

---

### Task 5: tmux detection and install offer

**Files:**
- Create: `src/tmux/install.rs`
- Modify: `src/tmux/mod.rs`

**Step 1: Write failing test**
```rust
// src/tmux/install.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_install_commands_are_valid() {
        let cmds = install_commands();
        assert_eq!(cmds.len(), 5);
        assert!(cmds[0].contains("git clone"));
        assert!(cmds[1].contains("cd tmux"));
        assert!(cmds[2].contains("autogen.sh"));
        assert!(cmds[3].contains("configure"));
        assert!(cmds[4].contains("make install"));
    }
}
```

**Step 2: Verify it fails**
Run: `cargo test tmux::install::tests::test_install_commands_are_valid -- --nocapture 2>&1 | tail -5`
Expected: FAIL — module `install` not found

**Step 3: Implement**
```rust
// src/tmux/install.rs
//! tmux installation flow when tmux is not found.

use std::io::{self, Write};

/// Return the sequence of shell commands to install tmux from source.
pub fn install_commands() -> Vec<String> {
    vec![
        "git clone https://github.com/tmux/tmux.git".to_string(),
        "cd tmux".to_string(),
        "sh autogen.sh".to_string(),
        "./configure && make".to_string(),
        "sudo make install".to_string(),
    ]
}

/// Print the install prompt and return true if user accepts.
pub fn prompt_install() -> bool {
    eprintln!("\x1b[1;31mError:\x1b[0m tmux is not installed.\n");
    eprintln!("tmux mode requires tmux to be available in your PATH.\n");
    eprintln!("Would you like to install tmux from source? [y/N]\n");
    eprintln!("This will run:");
    for cmd in install_commands() {
        eprintln!("  {}", cmd);
    }
    eprint!("\n> ");
    io::stderr().flush().ok();

    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_ok() {
        return input.trim().eq_ignore_ascii_case("y");
    }
    false
}

/// Execute the install commands. Returns Ok(()) on success.
pub async fn run_install() -> Result<(), String> {
    use tokio::process::Command;

    let tmpdir = std::env::temp_dir().join("synaps-tmux-install");
    let _ = std::fs::remove_dir_all(&tmpdir);
    std::fs::create_dir_all(&tmpdir)
        .map_err(|e| format!("Failed to create temp dir: {}", e))?;

    eprintln!("\nCloning tmux...");
    let status = Command::new("git")
        .args(["clone", "https://github.com/tmux/tmux.git"])
        .current_dir(&tmpdir)
        .status()
        .await
        .map_err(|e| format!("git clone failed: {}", e))?;
    if !status.success() {
        return Err("git clone failed".to_string());
    }

    let tmux_dir = tmpdir.join("tmux");

    eprintln!("Running autogen.sh...");
    let status = Command::new("sh")
        .arg("autogen.sh")
        .current_dir(&tmux_dir)
        .status()
        .await
        .map_err(|e| format!("autogen.sh failed: {}", e))?;
    if !status.success() {
        return Err("autogen.sh failed. You may need: automake, autoconf, pkg-config".to_string());
    }

    eprintln!("Configuring and building...");
    let status = Command::new("sh")
        .args(["-c", "./configure && make"])
        .current_dir(&tmux_dir)
        .status()
        .await
        .map_err(|e| format!("configure/make failed: {}", e))?;
    if !status.success() {
        return Err("Build failed. You may need: libevent-dev, ncurses-dev (libncurses-dev)".to_string());
    }

    eprintln!("Installing (may require sudo password)...");
    let status = Command::new("sudo")
        .args(["make", "install"])
        .current_dir(&tmux_dir)
        .status()
        .await
        .map_err(|e| format!("make install failed: {}", e))?;
    if !status.success() {
        return Err("make install failed".to_string());
    }

    // Cleanup
    let _ = std::fs::remove_dir_all(&tmpdir);

    // Verify
    if crate::tmux::find_tmux().is_some() {
        eprintln!("\n\x1b[1;32m✓\x1b[0m tmux installed successfully!");
        Ok(())
    } else {
        Err("tmux installed but not found in PATH".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_install_commands_are_valid() {
        let cmds = install_commands();
        assert_eq!(cmds.len(), 5);
        assert!(cmds[0].contains("git clone"));
        assert!(cmds[1].contains("cd tmux"));
        assert!(cmds[2].contains("autogen.sh"));
        assert!(cmds[3].contains("configure"));
        assert!(cmds[4].contains("make install"));
    }
}
```

Add to `src/tmux/mod.rs`:
```rust
pub mod install;
```

**Step 4: Verify it passes**
Run: `cargo test tmux::install::tests::test_install_commands_are_valid -- --nocapture 2>&1 | tail -5`
Expected: PASS

**Step 5: Commit**
```bash
git add -A && git commit -m "feat(tmux): add detection and install-from-source flow"
```

---

## Batch 2: Control Mode Protocol (Tasks 6–10)

### Task 6: Define TmuxEvent enum and protocol types

**Files:**
- Create: `src/tmux/protocol.rs`
- Modify: `src/tmux/mod.rs`

**Step 1: Write failing test**
```rust
// src/tmux/protocol.rs — at bottom
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_output_notification() {
        let line = "%output %0 hello world";
        let event = TmuxEvent::parse(line);
        assert!(matches!(event, Some(TmuxEvent::Output { pane_id, data }) 
            if pane_id == "%0" && data == "hello world"));
    }

    #[test]
    fn test_parse_window_add_notification() {
        let line = "%window-add @5";
        let event = TmuxEvent::parse(line);
        assert!(matches!(event, Some(TmuxEvent::WindowAdd { window_id })
            if window_id == "@5"));
    }

    #[test]
    fn test_parse_layout_change() {
        let line = "%layout-change @0 ab12,200x50,0,0{100x50,0,0,0,99x50,101,0,1}";
        let event = TmuxEvent::parse(line);
        assert!(matches!(event, Some(TmuxEvent::LayoutChange { window_id, layout })
            if window_id == "@0"));
    }

    #[test]
    fn test_parse_begin_end() {
        let begin = "%begin 1234567890 42 1";
        let event = TmuxEvent::parse(begin);
        assert!(matches!(event, Some(TmuxEvent::Begin { command_num, .. })
            if command_num == 42));

        let end = "%end 1234567890 42 1";
        let event = TmuxEvent::parse(end);
        assert!(matches!(event, Some(TmuxEvent::End { command_num, .. })
            if command_num == 42));
    }

    #[test]
    fn test_parse_unknown_line() {
        let line = "some random output";
        let event = TmuxEvent::parse(line);
        assert!(matches!(event, Some(TmuxEvent::Data(_))));
    }

    #[test]
    fn test_parse_pane_exited() {
        let line = "%pane-exited %3";
        let event = TmuxEvent::parse(line);
        assert!(matches!(event, Some(TmuxEvent::PaneExited { pane_id })
            if pane_id == "%3"));
    }

    #[test]
    fn test_parse_session_changed() {
        let line = "%session-changed $1 my-session";
        let event = TmuxEvent::parse(line);
        assert!(matches!(event, Some(TmuxEvent::SessionChanged { session_id, name })
            if session_id == "$1" && name == "my-session"));
    }
}
```

**Step 2: Verify it fails**
Run: `cargo test tmux::protocol::tests -- --nocapture 2>&1 | tail -5`
Expected: FAIL — module `protocol` not found

**Step 3: Implement**
```rust
// src/tmux/protocol.rs
//! Control mode protocol parser for tmux notifications and command responses.

/// Events received from tmux control mode.
#[derive(Debug, Clone)]
pub enum TmuxEvent {
    /// %begin <time> <command_num> <flags>
    Begin { time: u64, command_num: u64, flags: u32 },
    /// %end <time> <command_num> <flags>
    End { time: u64, command_num: u64, flags: u32 },
    /// %error <time> <command_num> <flags>
    Error { time: u64, command_num: u64, flags: u32 },
    /// %output %<pane_id> <data>
    Output { pane_id: String, data: String },
    /// %window-add @<window_id>
    WindowAdd { window_id: String },
    /// %window-close @<window_id>
    WindowClose { window_id: String },
    /// %window-renamed @<window_id> <name>
    WindowRenamed { window_id: String, name: String },
    /// %session-changed $<session_id> <name>
    SessionChanged { session_id: String, name: String },
    /// %layout-change @<window_id> <layout_string>
    LayoutChange { window_id: String, layout: String },
    /// %pane-mode-changed %<pane_id>
    PaneModeChanged { pane_id: String },
    /// %pane-exited %<pane_id>
    PaneExited { pane_id: String },
    /// Non-notification data (command response lines between %begin/%end)
    Data(String),
}

impl TmuxEvent {
    /// Parse a single line from tmux control mode output.
    pub fn parse(line: &str) -> Option<TmuxEvent> {
        if line.starts_with("%begin ") {
            return Self::parse_triple(line, "%begin ").map(|(t, n, f)| TmuxEvent::Begin {
                time: t, command_num: n, flags: f,
            });
        }
        if line.starts_with("%end ") {
            return Self::parse_triple(line, "%end ").map(|(t, n, f)| TmuxEvent::End {
                time: t, command_num: n, flags: f,
            });
        }
        if line.starts_with("%error ") {
            return Self::parse_triple(line, "%error ").map(|(t, n, f)| TmuxEvent::Error {
                time: t, command_num: n, flags: f,
            });
        }
        if let Some(rest) = line.strip_prefix("%output ") {
            let (pane_id, data) = rest.split_once(' ').unwrap_or((rest, ""));
            return Some(TmuxEvent::Output {
                pane_id: pane_id.to_string(),
                data: data.to_string(),
            });
        }
        if let Some(rest) = line.strip_prefix("%window-add ") {
            return Some(TmuxEvent::WindowAdd { window_id: rest.trim().to_string() });
        }
        if let Some(rest) = line.strip_prefix("%window-close ") {
            return Some(TmuxEvent::WindowClose { window_id: rest.trim().to_string() });
        }
        if let Some(rest) = line.strip_prefix("%window-renamed ") {
            let (id, name) = rest.split_once(' ').unwrap_or((rest, ""));
            return Some(TmuxEvent::WindowRenamed {
                window_id: id.to_string(),
                name: name.to_string(),
            });
        }
        if let Some(rest) = line.strip_prefix("%session-changed ") {
            let (id, name) = rest.split_once(' ').unwrap_or((rest, ""));
            return Some(TmuxEvent::SessionChanged {
                session_id: id.to_string(),
                name: name.to_string(),
            });
        }
        if let Some(rest) = line.strip_prefix("%layout-change ") {
            let (id, layout) = rest.split_once(' ').unwrap_or((rest, ""));
            return Some(TmuxEvent::LayoutChange {
                window_id: id.to_string(),
                layout: layout.to_string(),
            });
        }
        if let Some(rest) = line.strip_prefix("%pane-mode-changed ") {
            return Some(TmuxEvent::PaneModeChanged { pane_id: rest.trim().to_string() });
        }
        if let Some(rest) = line.strip_prefix("%pane-exited ") {
            return Some(TmuxEvent::PaneExited { pane_id: rest.trim().to_string() });
        }
        // Non-notification line = data (command response body)
        Some(TmuxEvent::Data(line.to_string()))
    }

    fn parse_triple(line: &str, prefix: &str) -> Option<(u64, u64, u32)> {
        let rest = line.strip_prefix(prefix)?;
        let parts: Vec<&str> = rest.split_whitespace().collect();
        if parts.len() >= 3 {
            let time = parts[0].parse().ok()?;
            let num = parts[1].parse().ok()?;
            let flags = parts[2].parse().ok()?;
            Some((time, num, flags))
        } else {
            None
        }
    }
}
```

Add to `src/tmux/mod.rs`:
```rust
pub mod protocol;
```

**Step 4: Verify it passes**
Run: `cargo test tmux::protocol::tests -- --nocapture 2>&1 | tail -5`
Expected: PASS — all 7 tests pass

**Step 5: Commit**
```bash
git add -A && git commit -m "feat(tmux): control mode protocol parser"
```

---

### Task 7: Define TmuxState

**Files:**
- Create: `src/tmux/state.rs`
- Modify: `src/tmux/mod.rs`

**Step 1: Write failing test**
```rust
// src/tmux/state.rs — at bottom
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_new() {
        let state = TmuxState::new("$0", "my-session");
        assert_eq!(state.session_id, "$0");
        assert_eq!(state.session_name, "my-session");
        assert!(state.windows.is_empty());
        assert!(state.panes.is_empty());
    }

    #[test]
    fn test_add_and_get_pane() {
        let mut state = TmuxState::new("$0", "test");
        state.add_pane(TmuxPane {
            id: "%0".to_string(),
            window_id: "@0".to_string(),
            title: "main".to_string(),
            width: 200,
            height: 50,
            active: true,
            role: PaneRole::SynapsTui,
        });
        assert_eq!(state.panes.len(), 1);
        assert!(state.pane("%0").is_some());
        assert_eq!(state.pane("%0").unwrap().role, PaneRole::SynapsTui);
    }

    #[test]
    fn test_remove_pane() {
        let mut state = TmuxState::new("$0", "test");
        state.add_pane(TmuxPane {
            id: "%1".to_string(),
            window_id: "@0".to_string(),
            title: "shell".to_string(),
            width: 80,
            height: 24,
            active: false,
            role: PaneRole::AgentShell { session_id: "shell_01".to_string() },
        });
        assert_eq!(state.panes.len(), 1);
        state.remove_pane("%1");
        assert_eq!(state.panes.len(), 0);
    }

    #[test]
    fn test_panes_by_role() {
        let mut state = TmuxState::new("$0", "test");
        state.add_pane(TmuxPane {
            id: "%0".to_string(), window_id: "@0".to_string(),
            title: "tui".to_string(), width: 100, height: 50,
            active: true, role: PaneRole::SynapsTui,
        });
        state.add_pane(TmuxPane {
            id: "%1".to_string(), window_id: "@0".to_string(),
            title: "sh1".to_string(), width: 100, height: 25,
            active: false, role: PaneRole::AgentShell { session_id: "shell_01".to_string() },
        });
        state.add_pane(TmuxPane {
            id: "%2".to_string(), window_id: "@0".to_string(),
            title: "sa1".to_string(), width: 100, height: 25,
            active: false, role: PaneRole::Subagent { handle_id: "sa_1".to_string() },
        });

        let shells: Vec<_> = state.panes_by_role_filter(|r| matches!(r, PaneRole::AgentShell { .. }));
        assert_eq!(shells.len(), 1);
        assert_eq!(shells[0].id, "%1");
    }
}
```

**Step 2: Verify it fails**
Run: `cargo test tmux::state::tests -- --nocapture 2>&1 | tail -5`
Expected: FAIL — module `state` not found

**Step 3: Implement**
```rust
// src/tmux/state.rs
//! Tracked state of tmux objects (sessions, windows, panes).

use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum PaneRole {
    SynapsTui,
    AgentShell { session_id: String },
    Subagent { handle_id: String },
    User,
}

#[derive(Debug, Clone)]
pub struct TmuxPane {
    pub id: String,
    pub window_id: String,
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub active: bool,
    pub role: PaneRole,
}

#[derive(Debug, Clone)]
pub struct TmuxWindow {
    pub id: String,
    pub name: String,
    pub index: u32,
    pub layout: String,
}

#[derive(Debug)]
pub struct TmuxState {
    pub session_id: String,
    pub session_name: String,
    pub windows: HashMap<String, TmuxWindow>,
    pub panes: HashMap<String, TmuxPane>,
    pub self_pane: Option<String>,
}

impl TmuxState {
    pub fn new(session_id: &str, session_name: &str) -> Self {
        Self {
            session_id: session_id.to_string(),
            session_name: session_name.to_string(),
            windows: HashMap::new(),
            panes: HashMap::new(),
            self_pane: None,
        }
    }

    pub fn add_pane(&mut self, pane: TmuxPane) {
        self.panes.insert(pane.id.clone(), pane);
    }

    pub fn remove_pane(&mut self, pane_id: &str) {
        self.panes.remove(pane_id);
    }

    pub fn pane(&self, pane_id: &str) -> Option<&TmuxPane> {
        self.panes.get(pane_id)
    }

    pub fn pane_mut(&mut self, pane_id: &str) -> Option<&mut TmuxPane> {
        self.panes.get_mut(pane_id)
    }

    pub fn add_window(&mut self, window: TmuxWindow) {
        self.windows.insert(window.id.clone(), window);
    }

    pub fn remove_window(&mut self, window_id: &str) {
        self.windows.remove(window_id);
    }

    pub fn panes_by_role_filter<F>(&self, f: F) -> Vec<&TmuxPane>
    where
        F: Fn(&PaneRole) -> bool,
    {
        self.panes.values().filter(|p| f(&p.role)).collect()
    }
}
```

Add to `src/tmux/mod.rs`:
```rust
pub mod state;
```

**Step 4: Verify it passes**
Run: `cargo test tmux::state::tests -- --nocapture 2>&1 | tail -5`
Expected: PASS — all 4 tests pass

**Step 5: Commit**
```bash
git add -A && git commit -m "feat(tmux): add TmuxState with pane/window tracking"
```

---

### Task 8: TmuxController — struct and command sending

**Files:**
- Create: `src/tmux/controller.rs`
- Modify: `src/tmux/mod.rs`

**Step 1: Write failing test**
```rust
// src/tmux/controller.rs — at bottom
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_command() {
        let cmd = TmuxController::format_command("split-window", &["-h", "-P", "-F", "#{pane_id}"]);
        assert_eq!(cmd, "split-window -h -P -F #{pane_id}\n");
    }

    #[test]
    fn test_format_command_no_args() {
        let cmd = TmuxController::format_command("list-windows", &[]);
        assert_eq!(cmd, "list-windows\n");
    }
}
```

**Step 2: Verify it fails**
Run: `cargo test tmux::controller::tests -- --nocapture 2>&1 | tail -5`
Expected: FAIL — module `controller` not found

**Step 3: Implement**
```rust
// src/tmux/controller.rs
//! TmuxController — manages the control mode connection to tmux.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, RwLock};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use std::sync::atomic::{AtomicU64, Ordering};

use super::protocol::TmuxEvent;
use super::state::TmuxState;

/// Result of a command sent to tmux control mode.
#[derive(Debug)]
pub struct CommandResult {
    pub lines: Vec<String>,
    pub success: bool,
}

/// The main tmux controller. Owns the control mode process and state.
pub struct TmuxController {
    /// Child process handle for the control mode client
    child: Option<Child>,
    /// Writer to control mode stdin
    writer: Option<tokio::process::ChildStdin>,
    /// Tracked state of all tmux objects
    state: Arc<RwLock<TmuxState>>,
    /// Channel for incoming parsed events
    event_tx: mpsc::UnboundedSender<TmuxEvent>,
    /// Receiver for events (consumed by event loop)
    event_rx: Option<mpsc::UnboundedReceiver<TmuxEvent>>,
    /// Pending command responses: command_num -> sender
    pending: Arc<tokio::sync::Mutex<HashMap<u64, oneshot::Sender<CommandResult>>>>,
    /// Next command number
    next_cmd: AtomicU64,
    /// Session name
    pub session_name: String,
}

impl TmuxController {
    /// Format a tmux command string for control mode.
    pub fn format_command(cmd: &str, args: &[&str]) -> String {
        if args.is_empty() {
            format!("{}\n", cmd)
        } else {
            format!("{} {}\n", cmd, args.join(" "))
        }
    }

    /// Create a new controller (does not start the connection yet).
    pub fn new(session_name: String) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        Self {
            child: None,
            writer: None,
            state: Arc::new(RwLock::new(TmuxState::new("", &session_name))),
            event_tx,
            event_rx: Some(event_rx),
            pending: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            next_cmd: AtomicU64::new(0),
            session_name,
        }
    }

    /// Get a handle to the shared state.
    pub fn state(&self) -> Arc<RwLock<TmuxState>> {
        Arc::clone(&self.state)
    }

    /// Send a command to tmux and wait for the response.
    pub async fn send_command(&self, cmd: &str, args: &[&str]) -> Result<CommandResult, String> {
        let writer = self.writer.as_ref()
            .ok_or_else(|| "Control mode not connected".to_string())?;

        let formatted = Self::format_command(cmd, args);

        // We need mutable access to write — use unsafe transmute or restructure
        // For now, we'll clone the stdin handle approach
        // This will be refined in the event loop task
        let _ = formatted; // placeholder
        let _ = writer;

        // TODO: Wire up actual send in event loop task
        Err("Not yet connected".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_command() {
        let cmd = TmuxController::format_command("split-window", &["-h", "-P", "-F", "#{pane_id}"]);
        assert_eq!(cmd, "split-window -h -P -F #{pane_id}\n");
    }

    #[test]
    fn test_format_command_no_args() {
        let cmd = TmuxController::format_command("list-windows", &[]);
        assert_eq!(cmd, "list-windows\n");
    }
}
```

Add to `src/tmux/mod.rs`:
```rust
pub mod controller;
pub use controller::TmuxController;
```

**Step 4: Verify it passes**
Run: `cargo test tmux::controller::tests -- --nocapture 2>&1 | tail -5`
Expected: PASS

**Step 5: Commit**
```bash
git add -A && git commit -m "feat(tmux): TmuxController struct with command formatting"
```

---

### Task 9: TmuxController — start control mode session

**Files:**
- Modify: `src/tmux/controller.rs`

**Step 1: Write failing test**
```rust
// Add to src/tmux/controller.rs tests
#[tokio::test]
async fn test_start_requires_tmux() {
    // This test verifies the start method exists and handles missing tmux
    let mut controller = TmuxController::new("test-session".to_string());
    // On CI or systems without tmux, start should return an error
    if crate::tmux::find_tmux().is_none() {
        let result = controller.start().await;
        assert!(result.is_err());
    }
    // If tmux IS available, we test the full flow
    // (covered by integration tests)
}
```

**Step 2: Verify it fails**
Run: `cargo test tmux::controller::tests::test_start_requires_tmux -- --nocapture 2>&1 | tail -5`
Expected: FAIL — no method named `start`

**Step 3: Implement**

Add to `impl TmuxController`:
```rust
    /// Start a tmux session and connect via control mode.
    /// Creates a new detached session, then attaches in control mode.
    pub async fn start(&mut self) -> Result<(), String> {
        // 1. Check tmux exists
        let tmux_path = crate::tmux::find_tmux()
            .ok_or_else(|| "tmux not found in PATH".to_string())?;

        // 2. Get terminal size
        let (cols, rows) = crossterm::terminal::size()
            .unwrap_or((200, 50));

        // 3. Create detached session
        let create_status = Command::new(&tmux_path)
            .args([
                "new-session", "-d",
                "-s", &self.session_name,
                "-x", &cols.to_string(),
                "-y", &rows.to_string(),
            ])
            .status()
            .await
            .map_err(|e| format!("Failed to create tmux session: {}", e))?;

        if !create_status.success() {
            return Err("Failed to create tmux session".to_string());
        }

        // 4. Attach in control mode
        let mut child = Command::new(&tmux_path)
            .args(["-CC", "attach-session", "-t", &self.session_name])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("Failed to start control mode: {}", e))?;

        let stdout = child.stdout.take()
            .ok_or_else(|| "Failed to capture stdout".to_string())?;
        let stdin = child.stdin.take()
            .ok_or_else(|| "Failed to capture stdin".to_string())?;

        self.writer = Some(stdin);
        self.child = Some(child);

        // 5. Start reader task
        let event_tx = self.event_tx.clone();
        let pending = Arc::clone(&self.pending);
        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            let mut current_cmd: Option<u64> = None;
            let mut current_lines: Vec<String> = Vec::new();

            while let Ok(Some(line)) = lines.next_line().await {
                if let Some(event) = TmuxEvent::parse(&line) {
                    match &event {
                        TmuxEvent::Begin { command_num, .. } => {
                            current_cmd = Some(*command_num);
                            current_lines.clear();
                        }
                        TmuxEvent::End { command_num, .. } => {
                            if Some(*command_num) == current_cmd {
                                let mut map = pending.lock().await;
                                if let Some(sender) = map.remove(command_num) {
                                    let _ = sender.send(CommandResult {
                                        lines: current_lines.drain(..).collect(),
                                        success: true,
                                    });
                                }
                                current_cmd = None;
                            }
                        }
                        TmuxEvent::Error { command_num, .. } => {
                            if Some(*command_num) == current_cmd {
                                let mut map = pending.lock().await;
                                if let Some(sender) = map.remove(command_num) {
                                    let _ = sender.send(CommandResult {
                                        lines: current_lines.drain(..).collect(),
                                        success: false,
                                    });
                                }
                                current_cmd = None;
                            }
                        }
                        TmuxEvent::Data(data) => {
                            if current_cmd.is_some() {
                                current_lines.push(data.clone());
                            }
                        }
                        _ => {}
                    }
                    // Forward all events
                    let _ = event_tx.send(event);
                }
            }
        });

        tracing::info!("tmux control mode connected to session '{}'", self.session_name);
        Ok(())
    }

    /// Kill the tmux session and clean up.
    pub async fn shutdown(&mut self) -> Result<(), String> {
        if let Some(tmux_path) = crate::tmux::find_tmux() {
            let _ = Command::new(&tmux_path)
                .args(["kill-session", "-t", &self.session_name])
                .status()
                .await;
        }
        if let Some(mut child) = self.child.take() {
            let _ = child.kill().await;
        }
        self.writer = None;
        Ok(())
    }
```

**Step 4: Verify it passes**
Run: `cargo test tmux::controller::tests::test_start_requires_tmux -- --nocapture 2>&1 | tail -5`
Expected: PASS

**Step 5: Commit**
```bash
git add -A && git commit -m "feat(tmux): control mode start/shutdown lifecycle"
```

---

### Task 10: TmuxController — async command send with response

**Files:**
- Modify: `src/tmux/controller.rs`

**Step 1: Write failing test**
```rust
// Add to src/tmux/controller.rs tests
#[tokio::test]
async fn test_send_command_when_disconnected() {
    let controller = TmuxController::new("test-disconnected".to_string());
    let result = controller.execute("list-windows", &[]).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("not connected"));
}
```

**Step 2: Verify it fails**
Run: `cargo test tmux::controller::tests::test_send_command_when_disconnected -- --nocapture 2>&1 | tail -5`
Expected: FAIL — no method `execute`

**Step 3: Implement**

Replace the `send_command` method and add `execute`:
```rust
    /// Send a command to tmux control mode and wait for its response.
    pub async fn execute(&self, cmd: &str, args: &[&str]) -> Result<CommandResult, String> {
        let writer = self.writer.as_ref()
            .ok_or_else(|| "Control mode not connected".to_string())?;

        let cmd_num = self.next_cmd.fetch_add(1, Ordering::SeqCst);
        let formatted = Self::format_command(cmd, args);

        // Register the pending response
        let (tx, rx) = oneshot::channel();
        {
            let mut map = self.pending.lock().await;
            map.insert(cmd_num, tx);
        }

        // Write to stdin — we need interior mutability for the writer
        // Use unsafe pin projection since we know we're single-writer
        let writer_ptr = writer as *const tokio::process::ChildStdin
            as *mut tokio::process::ChildStdin;
        unsafe {
            let w = &mut *writer_ptr;
            w.write_all(formatted.as_bytes()).await
                .map_err(|e| format!("Failed to write command: {}", e))?;
            w.flush().await
                .map_err(|e| format!("Failed to flush: {}", e))?;
        }

        // Wait for response with timeout
        match tokio::time::timeout(std::time::Duration::from_secs(10), rx).await {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(_)) => Err("Response channel dropped".to_string()),
            Err(_) => {
                // Remove from pending on timeout
                let mut map = self.pending.lock().await;
                map.remove(&cmd_num);
                Err("Command timed out".to_string())
            }
        }
    }
```

**Step 4: Verify it passes**
Run: `cargo test tmux::controller::tests::test_send_command_when_disconnected -- --nocapture 2>&1 | tail -5`
Expected: PASS

**Step 5: Commit**
```bash
git add -A && git commit -m "feat(tmux): async command execution with response tracking"
```

---

## Batch 3: Layout Management (Tasks 11–13)

### Task 11: Layout presets — apply logic

**Files:**
- Create: `src/tmux/layout.rs`
- Modify: `src/tmux/mod.rs`

**Step 1: Write failing test**
```rust
// src/tmux/layout.rs
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tmux::config::LayoutPreset;

    #[test]
    fn test_split_layout_commands() {
        let cmds = layout_commands(&LayoutPreset::Split, "%0", 60);
        assert!(!cmds.is_empty());
        // Should contain a split-window command
        assert!(cmds.iter().any(|c| c.contains("split-window")));
    }

    #[test]
    fn test_fullscreen_layout_no_split() {
        let cmds = layout_commands(&LayoutPreset::Fullscreen, "%0", 60);
        // Fullscreen should not split — just keep the TUI pane
        assert!(cmds.is_empty() || cmds.iter().all(|c| !c.contains("split-window")));
    }

    #[test]
    fn test_tiled_layout_commands() {
        let cmds = layout_commands(&LayoutPreset::Tiled, "%0", 50);
        assert!(cmds.iter().any(|c| c.contains("tiled")));
    }
}
```

**Step 2: Verify it fails**
Run: `cargo test tmux::layout::tests -- --nocapture 2>&1 | tail -5`
Expected: FAIL — module `layout` not found

**Step 3: Implement**
```rust
// src/tmux/layout.rs
//! Layout preset logic for tmux pane arrangements.

use super::config::LayoutPreset;

/// Generate the tmux commands needed to apply a layout preset.
/// `self_pane` is the Synaps TUI pane ID, `split_ratio` is the TUI width percentage.
pub fn layout_commands(preset: &LayoutPreset, self_pane: &str, split_ratio: u32) -> Vec<String> {
    match preset {
        LayoutPreset::Split => {
            // Split horizontally: TUI on left, shells on right
            let shell_pct = 100 - split_ratio;
            vec![
                format!("split-window -h -t {} -l {}%", self_pane, shell_pct),
            ]
        }
        LayoutPreset::Fullscreen => {
            // No splits — TUI takes full window, shells go to new windows
            vec![]
        }
        LayoutPreset::Tiled => {
            vec![
                format!("select-layout -t {} tiled", self_pane),
            ]
        }
        LayoutPreset::Custom(layout_string) => {
            vec![
                format!("select-layout -t {} {}", self_pane, layout_string),
            ]
        }
    }
}

/// Generate commands to cycle to the next layout preset.
pub fn next_preset(current: &LayoutPreset) -> LayoutPreset {
    match current {
        LayoutPreset::Split => LayoutPreset::Fullscreen,
        LayoutPreset::Fullscreen => LayoutPreset::Tiled,
        LayoutPreset::Tiled => LayoutPreset::Split,
        LayoutPreset::Custom(_) => LayoutPreset::Split,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_layout_commands() {
        let cmds = layout_commands(&LayoutPreset::Split, "%0", 60);
        assert!(!cmds.is_empty());
        assert!(cmds.iter().any(|c| c.contains("split-window")));
    }

    #[test]
    fn test_fullscreen_layout_no_split() {
        let cmds = layout_commands(&LayoutPreset::Fullscreen, "%0", 60);
        assert!(cmds.is_empty());
    }

    #[test]
    fn test_tiled_layout_commands() {
        let cmds = layout_commands(&LayoutPreset::Tiled, "%0", 50);
        assert!(cmds.iter().any(|c| c.contains("tiled")));
    }

    #[test]
    fn test_next_preset_cycle() {
        assert_eq!(next_preset(&LayoutPreset::Split), LayoutPreset::Fullscreen);
        assert_eq!(next_preset(&LayoutPreset::Fullscreen), LayoutPreset::Tiled);
        assert_eq!(next_preset(&LayoutPreset::Tiled), LayoutPreset::Split);
        assert_eq!(next_preset(&LayoutPreset::Custom("x".to_string())), LayoutPreset::Split);
    }
}
```

Add to `src/tmux/mod.rs`:
```rust
pub mod layout;
```

**Step 4: Verify it passes**
Run: `cargo test tmux::layout::tests -- --nocapture 2>&1 | tail -5`
Expected: PASS — all 4 tests pass

**Step 5: Commit**
```bash
git add -A && git commit -m "feat(tmux): layout preset commands and cycling"
```

---

### Task 12: Hotkey bindings setup

**Files:**
- Create: `src/tmux/hotkeys.rs`
- Modify: `src/tmux/mod.rs`

**Step 1: Write failing test**
```rust
// src/tmux/hotkeys.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hotkey_commands_not_empty() {
        let cmds = hotkey_bind_commands();
        assert!(!cmds.is_empty());
    }

    #[test]
    fn test_hotkey_commands_contain_bind_key() {
        let cmds = hotkey_bind_commands();
        for cmd in &cmds {
            assert!(cmd.starts_with("bind-key"), "Expected bind-key command: {}", cmd);
        }
    }

    #[test]
    fn test_mouse_enable_command() {
        let cmd = mouse_command(true);
        assert!(cmd.contains("mouse"));
        assert!(cmd.contains("on"));

        let cmd = mouse_command(false);
        assert!(cmd.contains("mouse"));
        assert!(cmd.contains("off"));
    }
}
```

**Step 2: Verify it fails**
Run: `cargo test tmux::hotkeys::tests -- --nocapture 2>&1 | tail -5`
Expected: FAIL — module `hotkeys` not found

**Step 3: Implement**
```rust
// src/tmux/hotkeys.rs
//! Convenience hotkey bindings for Synaps tmux mode.

/// Generate the tmux bind-key commands for Synaps convenience keys.
pub fn hotkey_bind_commands() -> Vec<String> {
    vec![
        // C-b F — toggle fullscreen (zoom current pane)
        "bind-key -T prefix F resize-pane -Z".to_string(),
        // C-b S — placeholder for cycle subagent display (handled via run-shell callback)
        "bind-key -T prefix S display-message 'Cycling subagent display...'".to_string(),
        // C-b L — placeholder for cycle layout (handled via run-shell callback)
        "bind-key -T prefix L display-message 'Cycling layout...'".to_string(),
        // C-b N — next pane (enhanced: skip non-agent panes)
        "bind-key -T prefix N select-pane -t :.+".to_string(),
        // C-b P — previous pane
        "bind-key -T prefix P select-pane -t :.-".to_string(),
        // C-b H — show help popup
        "bind-key -T prefix H display-message 'Synaps: F=fullscreen S=subagent L=layout N/P=nav H=help'".to_string(),
    ]
}

/// Generate the mouse enable/disable command.
pub fn mouse_command(enabled: bool) -> String {
    if enabled {
        "set-option -g mouse on".to_string()
    } else {
        "set-option -g mouse off".to_string()
    }
}

/// Generate status bar format command for Synaps.
pub fn status_bar_commands(session_name: &str) -> Vec<String> {
    vec![
        "set-option -g status-position bottom".to_string(),
        format!(
            "set-option -g status-left '#[fg=cyan,bold][{}] '",
            session_name
        ),
        "set-option -g status-right '#[fg=yellow]C-b H:help #[fg=white]%H:%M'".to_string(),
        "set-option -g status-style 'bg=black,fg=white'".to_string(),
        "set-option -g window-status-current-style 'fg=green,bold'".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hotkey_commands_not_empty() {
        let cmds = hotkey_bind_commands();
        assert!(!cmds.is_empty());
    }

    #[test]
    fn test_hotkey_commands_contain_bind_key() {
        let cmds = hotkey_bind_commands();
        for cmd in &cmds {
            assert!(cmd.starts_with("bind-key"), "Expected bind-key command: {}", cmd);
        }
    }

    #[test]
    fn test_mouse_enable_command() {
        let cmd = mouse_command(true);
        assert!(cmd.contains("mouse"));
        assert!(cmd.contains("on"));

        let cmd = mouse_command(false);
        assert!(cmd.contains("mouse"));
        assert!(cmd.contains("off"));
    }

    #[test]
    fn test_status_bar_commands() {
        let cmds = status_bar_commands("my-project");
        assert!(cmds.iter().any(|c| c.contains("my-project")));
        assert!(cmds.iter().any(|c| c.contains("status-position")));
    }
}
```

Add to `src/tmux/mod.rs`:
```rust
pub mod hotkeys;
```

**Step 4: Verify it passes**
Run: `cargo test tmux::hotkeys::tests -- --nocapture 2>&1 | tail -5`
Expected: PASS — all 4 tests pass

**Step 5: Commit**
```bash
git add -A && git commit -m "feat(tmux): convenience hotkey bindings and status bar"
```

---

### Task 13: Wire TmuxController into Runtime

**Files:**
- Modify: `src/runtime/mod.rs`
- Modify: `src/tools/mod.rs`

**Step 1: Write failing test**
```rust
// Add to existing tests or in src/tmux/mod.rs tests
#[cfg(test)]
mod tests {
    #[test]
    fn test_auto_session_name_in_git_repo() {
        let name = super::auto_session_name();
        assert!(name.starts_with("synaps-"));
    }

    #[test]
    fn test_find_tmux_returns_option() {
        // Just verify the function exists and returns cleanly
        let _result = super::find_tmux();
    }
}
```

**Step 2: Verify it fails**
Run: `cargo build 2>&1 | tail -10`
Expected: Should compile (this is a wiring task — test is that it builds)

**Step 3: Implement**

Add to `Runtime` struct in `src/runtime/mod.rs`:
```rust
    tmux_controller: Option<Arc<crate::tmux::TmuxController>>,
```

Add to `Runtime::new()` struct literal:
```rust
    tmux_controller: None,
```

Add accessor methods:
```rust
    pub fn tmux_controller(&self) -> Option<&Arc<crate::tmux::TmuxController>> {
        self.tmux_controller.as_ref()
    }

    pub fn set_tmux_controller(&mut self, controller: Arc<crate::tmux::TmuxController>) {
        self.tmux_controller = Some(controller);
    }
```

Add to `ToolCapabilities` in `src/tools/mod.rs`:
```rust
    pub tmux_controller: Option<Arc<crate::tmux::TmuxController>>,
```

Update all `ToolCapabilities` construction sites to include `tmux_controller: None` (or `tmux_controller: runtime.tmux_controller().cloned()` where the runtime is available).

**Step 4: Verify it passes**
Run: `cargo build 2>&1 | tail -5`
Expected: compiles successfully

**Step 5: Commit**
```bash
git add -A && git commit -m "feat(tmux): wire TmuxController into Runtime and ToolCapabilities"
```

---

## Batch 4: tmux Tools — Part 1 (Tasks 14–17)

### Task 14: tmux_split tool

**Files:**
- Create: `src/tools/tmux_split.rs`
- Modify: `src/tools/mod.rs`
- Modify: `src/tools/registry.rs`

**Step 1: Write failing test**
```rust
// Add to src/tools/tests.rs
#[test]
fn test_tmux_split_tool_schema() {
    let tool = crate::tools::TmuxSplitTool;
    assert_eq!(tool.name(), "tmux_split");
    assert!(!tool.description().is_empty());
    let params = tool.parameters();
    assert_eq!(params["type"], "object");
    assert!(params["properties"]["direction"].is_object());
    assert!(params["properties"]["command"].is_object());
}

#[tokio::test]
async fn test_tmux_split_without_controller() {
    let tool = crate::tools::TmuxSplitTool;
    let ctx = create_tool_context();
    let params = serde_json::json!({ "direction": "horizontal" });
    let result = tool.execute(params, ctx).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("tmux"));
}
```

**Step 2: Verify it fails**
Run: `cargo test test_tmux_split_tool_schema -- --nocapture 2>&1 | tail -5`
Expected: FAIL — `TmuxSplitTool` not found

**Step 3: Implement**
```rust
// src/tools/tmux_split.rs
use serde_json::{json, Value};
use crate::{Result, RuntimeError};
use super::{Tool, ToolContext};

pub struct TmuxSplitTool;

#[async_trait::async_trait]
impl Tool for TmuxSplitTool {
    fn name(&self) -> &str { "tmux_split" }

    fn description(&self) -> &str {
        "Create a new tmux pane by splitting. Only available in tmux mode. Use to create visible terminal panes for running commands the user can see."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "direction": {
                    "type": "string",
                    "description": "Split direction: 'horizontal' (left/right) or 'vertical' (top/bottom). Default: horizontal",
                    "enum": ["horizontal", "vertical"]
                },
                "size": {
                    "type": "string",
                    "description": "Size of new pane as percentage (e.g. '30%') or line count. Default: 50%"
                },
                "target": {
                    "type": "string",
                    "description": "Pane ID to split (e.g. '%0'). Default: current active pane"
                },
                "command": {
                    "type": "string",
                    "description": "Command to run in the new pane"
                },
                "title": {
                    "type": "string",
                    "description": "Title for the new pane"
                },
                "focus": {
                    "type": "boolean",
                    "description": "Switch focus to the new pane. Default: false"
                }
            }
        })
    }

    async fn execute(&self, params: Value, ctx: ToolContext) -> Result<String> {
        let controller = ctx.capabilities.tmux_controller
            .as_ref()
            .ok_or_else(|| RuntimeError::Tool("tmux mode not active. Launch synaps with --tmux to use tmux tools.".to_string()))?;

        let direction = params["direction"].as_str().unwrap_or("horizontal");
        let size = params["size"].as_str().unwrap_or("50%");
        let target = params["target"].as_str();
        let command = params["command"].as_str();
        let title = params["title"].as_str();
        let focus = params["focus"].as_bool().unwrap_or(false);

        let dir_flag = if direction == "vertical" { "-v" } else { "-h" };

        let mut args = vec![dir_flag, "-P", "-F", "#{pane_id}"];

        let size_arg = format!("-l {}", size);
        args.push("-l");
        args.push(size);

        if !focus {
            args.push("-d");
        }

        if let Some(t) = target {
            args.push("-t");
            args.push(t);
        }

        if let Some(cmd) = command {
            args.push(cmd);
        }

        let result = controller.execute("split-window", &args).await
            .map_err(|e| RuntimeError::Tool(format!("tmux split failed: {}", e)))?;

        let pane_id = result.lines.first()
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        // Set title if provided
        if let Some(t) = title {
            let _ = controller.execute("select-pane", &["-t", &pane_id, "-T", t]).await;
        }

        Ok(json!({
            "pane_id": pane_id,
            "direction": direction,
            "size": size,
        }).to_string())
    }
}
```

Add to `src/tools/mod.rs`:
```rust
mod tmux_split;
pub use tmux_split::TmuxSplitTool;
```

Add to `src/tools/registry.rs` `new()` vec (conditionally — only in tmux mode, or always register and let the tool error):
```rust
Arc::new(crate::tools::tmux_split::TmuxSplitTool),
```

**Step 4: Verify it passes**
Run: `cargo test test_tmux_split_tool_schema -- --nocapture 2>&1 | tail -5`
Expected: PASS

**Step 5: Commit**
```bash
git add -A && git commit -m "feat(tmux): add tmux_split tool"
```

---

### Task 15: tmux_send tool

*(Same pattern as Task 14 — create `src/tools/tmux_send.rs`, register, test schema + no-controller error)*

### Task 16: tmux_capture tool

*(Same pattern — create `src/tools/tmux_capture.rs`, register, test)*

### Task 17: tmux_layout tool

*(Same pattern — create `src/tools/tmux_layout.rs`, register, test)*

---

## Batch 5: tmux Tools — Part 2 (Tasks 18–20)

### Task 18: tmux_window tool
### Task 19: tmux_resize tool
### Task 20: Update tool registry count in tests

---

## Batch 6: Startup Flow (Tasks 21–25)

### Task 21: Main entry point — tmux mode detection and launch
### Task 22: Session creation and control mode connection
### Task 23: Self-pane identification
### Task 24: Apply initial layout
### Task 25: Apply hotkeys and mouse settings

---

## Batch 7: Subagent Integration (Tasks 26–30)

### Task 26: Subagent pane display mode — create pane on subagent_start
### Task 27: Subagent window display mode — create window on subagent_start
### Task 28: Subagent output streaming to tmux pane
### Task 29: Subagent cleanup on completion
### Task 30: Config toggle for subagent display mode

---

## Batch 8: Shutdown & Cleanup (Tasks 31–34)

### Task 31: Graceful shutdown — kill session on exit
### Task 32: Signal handling (SIGINT, SIGTERM)
### Task 33: Cleanup stale sessions on startup
### Task 34: Error recovery — control mode pipe breaks

---

## Batch 9: Settings Integration (Tasks 35–38)

### Task 35: Project-level config (.synaps/config) loader
### Task 36: /settings tmux page in TUI
### Task 37: Agent can change layout via natural language
### Task 38: Persist layout changes to config

---

## Batch 10: Polish & Testing (Tasks 39–42)

### Task 39: Integration test — full tmux lifecycle (requires tmux)
### Task 40: System prompt context — inject tmux session info
### Task 41: Documentation — README section for tmux mode
### Task 42: Final test suite pass and cleanup
