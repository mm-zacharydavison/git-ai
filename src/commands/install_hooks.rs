use crate::error::GitAiError;
use indicatif::{ProgressBar, ProgressStyle};
use serde_json::{Value, json};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

pub fn run(_args: &[String]) -> Result<(), GitAiError> {
    // Run async operations with smol
    smol::block_on(async_run())
}

async fn async_run() -> Result<(), GitAiError> {
    let mut any_installed = false;

    if check_claude_code() {
        // Install/update Claude Code hooks
        let spinner = Spinner::new("Claude code: installing hooks");
        spinner.start();

        if let Err(e) = install_claude_code_hooks() {
            // We intentionally don't fail hard for teammate workflows
            // but we surface a message for the current user.
            spinner.skipped("Claude code: Could not update hooks (safe to ignore)");
            eprintln!("Note: failed to update .claude/settings.json: {}", e);
        } else {
            spinner.success("Claude code: Hooks installed");
        }
        any_installed = true;
    }

    if check_cursor() {
        // Install/update Cursor hooks
        let spinner = Spinner::new("Cursor: installing hooks");
        spinner.start();

        if let Err(e) = install_cursor_hooks() {
            // Do not fail hard; keep teammate workflows safe
            spinner.skipped("Cursor: Could not update hooks (safe to ignore)");
            eprintln!("Note: failed to update ~/.cursor/hooks.json: {}", e);
        } else {
            spinner.success("Cursor: Hooks installed");
        }
        any_installed = true;
    }

    if !any_installed {
        println!("No compatible IDEs or agent configurations detected. Nothing to install.");
    }

    Ok(())
}

fn check_claude_code() -> bool {
    if binary_exists("claude") {
        return true;
    }

    // Sometimes the binary won't be in the PATH, but the dotfiles will be
    let home = home_dir();
    return home.join(".claude").exists();
}

fn check_cursor() -> bool {
    // TODO: Also check if dotfiles for cursor exist (windows?)
    if binary_exists("cursor") {
        return true;
    }

    // TODO Approach for Windows?

    // Sometimes the binary won't be in the PATH, but the dotfiles will be
    let home = home_dir();
    return home.join(".cursor").exists();
}

// Shared utilities

/// Check if a binary with the given name exists in the system PATH
fn binary_exists(name: &str) -> bool {
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            // First check exact name as provided
            let candidate = dir.join(name);
            if candidate.exists() && candidate.is_file() {
                return true;
            }

            // On Windows, executables usually have extensions listed in PATHEXT
            #[cfg(windows)]
            {
                let pathext =
                    std::env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.BAT;.CMD;.COM".to_string());
                for ext in pathext.split(';') {
                    let ext = ext.trim();
                    if ext.is_empty() {
                        continue;
                    }
                    let ext = if ext.starts_with('.') {
                        ext.to_string()
                    } else {
                        format!(".{}", ext)
                    };
                    let candidate = dir.join(format!("{}{}", name, ext));
                    if candidate.exists() && candidate.is_file() {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn install_claude_code_hooks() -> Result<(), GitAiError> {
    let settings_path = claude_settings_path();

    // Ensure directory exists
    if let Some(dir) = settings_path.parent() {
        fs::create_dir_all(dir)?;
    }

    // Read existing JSON if present, else start with empty object
    let existing: Value = if settings_path.exists() {
        let contents = fs::read_to_string(&settings_path)?;
        if contents.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str(&contents)?
        }
    } else {
        json!({})
    };

    // Desired hooks payload
    let desired: Value = json!({
        "hooks": {
            "PreToolUse": [
                {
                    "matcher": "Write|Edit|MultiEdit",
                    "hooks": [
                        {
                            "type": "command",
                            "command": "git-ai checkpoint 2>/dev/null || true"
                        }
                    ]
                }
            ],
            "PostToolUse": [
                {
                    "matcher": "Write|Edit|MultiEdit",
                    "hooks": [
                        {
                            "type": "command",
                            "command": "git-ai checkpoint claude --hook-input \"$(cat)\" 2>/dev/null || true"
                        }
                    ]
                }
            ]
        }
    });

    // Merge desired into existing (shallow for hooks; overwrite our specific arrays)
    let mut merged = existing.clone();
    {
        let hooks_obj = desired.get("hooks").cloned().unwrap_or_else(|| json!({}));
        let merged_hooks = merge_hooks(merged.get("hooks"), Some(&hooks_obj));
        if let Some(h) = merged_hooks {
            if let Some(obj) = merged.as_object_mut() {
                obj.insert("hooks".to_string(), h);
            }
        }
    }

    // Write pretty JSON to file atomically
    let pretty = serde_json::to_string_pretty(&merged)?;
    write_atomic(&settings_path, pretty.as_bytes())?;
    Ok(())
}

fn install_cursor_hooks() -> Result<(), GitAiError> {
    let hooks_path = cursor_hooks_path();

    // Ensure directory exists
    if let Some(dir) = hooks_path.parent() {
        fs::create_dir_all(dir)?;
    }

    // Read existing JSON if present, else start with empty object
    let existing: Value = if hooks_path.exists() {
        let contents = fs::read_to_string(&hooks_path)?;
        if contents.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str(&contents)?
        }
    } else {
        json!({})
    };

    // Desired hooks payload for Cursor
    let desired: Value = json!({
        "version": 1,
        "hooks": {
            "afterEdit": [
                {
                    "command": "git-ai checkpoint cursor 2>/dev/null || true"
                }
            ]
        }
    });

    // Merge desired into existing (version + hooks.afterEdit dedup by command)
    let mut merged = existing.clone();

    // Ensure version is set (preserve existing if present)
    if merged.get("version").is_none() {
        if let Some(obj) = merged.as_object_mut() {
            obj.insert("version".to_string(), json!(1));
        }
    }

    // Merge hooks object
    let mut hooks_obj = merged.get("hooks").cloned().unwrap_or_else(|| json!({}));

    // AfterEdit desired entries
    let desired_after = desired
        .get("hooks")
        .and_then(|h| h.get("afterEdit"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    // Existing afterEdit array
    let mut existing_after = hooks_obj
        .get("afterEdit")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    // Deduplicate by full object and by command string
    for d in desired_after {
        let is_dup = existing_after.iter().any(|e| {
            if e == &d {
                return true;
            }
            let dc = d.get("command").and_then(|c| c.as_str());
            let ec = e.get("command").and_then(|c| c.as_str());
            match (dc, ec) {
                (Some(a), Some(b)) => a == b,
                _ => false,
            }
        });
        if !is_dup {
            existing_after.push(d.clone());
        }
    }

    // Write back merged hooks
    if let Some(obj) = hooks_obj.as_object_mut() {
        obj.insert("afterEdit".to_string(), Value::Array(existing_after));
    }

    if let Some(root) = merged.as_object_mut() {
        root.insert("hooks".to_string(), hooks_obj);
    }

    // Write pretty JSON atomically
    let pretty = serde_json::to_string_pretty(&merged)?;
    write_atomic(&hooks_path, pretty.as_bytes())?;
    Ok(())
}

fn merge_hooks(existing: Option<&Value>, desired: Option<&Value>) -> Option<Value> {
    let mut result = existing.cloned().unwrap_or_else(|| json!({}));
    let desired = match desired {
        Some(v) => v,
        None => return Some(result),
    };

    // Merge arrays by matcher for PreToolUse and PostToolUse. Append missing hooks, avoid duplicates.
    if let Some(obj) = result.as_object_mut() {
        for key in ["PreToolUse", "PostToolUse"].iter() {
            let desired_arr = desired.get(*key).and_then(|v| v.as_array());
            if desired_arr.is_none() {
                continue;
            }
            let desired_arr = desired_arr.unwrap();

            let mut existing_arr = obj
                .get_mut(*key)
                .and_then(|v| v.as_array_mut())
                .cloned()
                .unwrap_or_else(Vec::new);

            // Build an index of existing entries by matcher string
            use std::collections::HashMap;
            let mut matcher_to_index: HashMap<String, usize> = HashMap::new();
            for (i, item) in existing_arr.iter().enumerate() {
                if let Some(matcher) = item.get("matcher").and_then(|m| m.as_str()) {
                    matcher_to_index.insert(matcher.to_string(), i);
                }
            }

            for desired_item in desired_arr {
                let desired_matcher = desired_item.get("matcher").and_then(|m| m.as_str());
                if desired_matcher.is_none() {
                    continue;
                }
                let desired_matcher = desired_matcher.unwrap();

                // Find or create the block for this matcher
                let target_index = if let Some(&idx) = matcher_to_index.get(desired_matcher) {
                    idx
                } else {
                    // Create new block
                    existing_arr.push(json!({
                        "matcher": desired_matcher,
                        "hooks": []
                    }));
                    let new_index = existing_arr.len() - 1;
                    matcher_to_index.insert(desired_matcher.to_string(), new_index);
                    new_index
                };

                // Merge hooks arrays, deduplicating by full object equality and by command string if present
                if let Some(target_hooks) = existing_arr[target_index]
                    .get_mut("hooks")
                    .and_then(|h| h.as_array_mut())
                {
                    let desired_hooks = desired_item.get("hooks").and_then(|h| h.as_array());
                    if let Some(desired_hooks) = desired_hooks {
                        for d in desired_hooks {
                            let duplicate = target_hooks.iter().any(|e| {
                                if e == d {
                                    return true;
                                }
                                let dc = d.get("command").and_then(|c| c.as_str());
                                let ec = e.get("command").and_then(|c| c.as_str());
                                match (dc, ec) {
                                    (Some(a), Some(b)) => a == b,
                                    _ => false,
                                }
                            });
                            if !duplicate {
                                target_hooks.push(d.clone());
                            }
                        }
                    }
                }
            }

            obj.insert((*key).to_string(), Value::Array(existing_arr));
        }
    }
    Some(result)
}

fn claude_settings_path() -> PathBuf {
    home_dir().join(".claude").join("settings.json")
}

fn cursor_hooks_path() -> PathBuf {
    home_dir().join(".cursor").join("hooks.json")
}

fn write_atomic(path: &Path, data: &[u8]) -> Result<(), GitAiError> {
    let tmp_path = path.with_extension("tmp");
    {
        let mut file = fs::File::create(&tmp_path)?;
        file.write_all(data)?;
        file.sync_all()?;
    }
    fs::rename(&tmp_path, path)?;
    Ok(())
}

fn home_dir() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home);
    }
    #[cfg(windows)]
    {
        if let Ok(userprofile) = std::env::var("USERPROFILE") {
            return PathBuf::from(userprofile);
        }
    }
    PathBuf::from(".")
}

// Loader
struct Spinner {
    pb: ProgressBar,
}

impl Spinner {
    fn new(message: &str) -> Self {
        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.green} {msg}")
                .unwrap()
                .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
        );
        pb.set_message(message.to_string());
        pb.enable_steady_tick(std::time::Duration::from_millis(100));

        Self { pb }
    }

    fn start(&self) {
        // Spinner starts automatically when created
    }

    fn _update_message(&self, message: &str) {
        self.pb.set_message(message.to_string());
    }

    async fn _wait_for(&self, duration_ms: u64) {
        smol::Timer::after(std::time::Duration::from_millis(duration_ms)).await;
    }

    fn success(&self, message: &'static str) {
        // Clear spinner and show success with green checkmark and green text
        self.pb.finish_and_clear();
        println!("\x1b[32m✓ {}\x1b[0m", message);
    }

    #[allow(dead_code)]
    fn error(&self, message: &'static str) {
        // Clear spinner and show error with red X and red text
        self.pb.finish_and_clear();
        println!("\x1b[31m✗ {}\x1b[0m", message);
    }

    fn skipped(&self, message: &'static str) {
        // Clear spinner and show skipped with gray circle and gray text
        self.pb.finish_and_clear();
        println!("\x1b[90m○ {}\x1b[0m", message);
    }
}
