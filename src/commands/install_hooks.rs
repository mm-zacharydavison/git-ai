use crate::error::GitAiError;
use indicatif::{ProgressBar, ProgressStyle};
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

pub fn run(_args: &[String]) -> Result<(), GitAiError> {
    // Run async operations with smol
    smol::block_on(async_run())
}

async fn async_run() -> Result<(), GitAiError> {
    check_claude_code().await;

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

    Ok(())
}

async fn check_claude_code() {
    let spinner = Spinner::new("Claude code: checking installation");
    spinner.start();

    let exists = binary_exists("claude");

    if exists {
        spinner.success("Claude code: Installed and ready");
    } else {
        spinner.skipped("Claude code: Not installed");
    }
}

#[allow(dead_code)]
async fn check_codex() {
    let spinner = Spinner::new("Codex: checking install");
    spinner.start();

    // Check if Codex binary exists
    let exists = binary_exists("claude");

    // Simulate some checking time
    spinner.wait_for(1500).await;

    if exists {
        spinner.success("Codex: Installed");
    } else {
        spinner.error("Codex: Not found");
    }
}

#[allow(dead_code)]
async fn check_windsurf() {
    let spinner = Spinner::new("Windsurf: checking status");
    spinner.start();

    // Simulate checking Windsurf
    spinner.wait_for(500).await;

    spinner.error("Windsurf: Not installed");
}

#[allow(dead_code)]
async fn check_cursor() {
    let spinner = Spinner::new("Cursor: checking status");
    spinner.start();

    // Simulate checking Cursor
    spinner.wait_for(2000).await;

    spinner.success("Cursor: Ready");
}

#[allow(dead_code)]
async fn check_github_copilot() {
    let spinner = Spinner::new("GitHub Copilot: verifying");
    spinner.start();

    // Simulate verifying GitHub Copilot
    spinner.wait_for(3000).await;

    spinner.success("GitHub Copilot: Active");
}

// Shared utilities

/// Check if a binary with the given name exists in the system PATH
fn binary_exists(name: &str) -> bool {
    if let Ok(path) = std::env::var("PATH") {
        for dir in path.split(':') {
            let binary_path = std::path::Path::new(dir).join(name);
            if binary_path.exists() && binary_path.is_file() {
                return true;
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
                            "command": "git-ai checkpoint --author \"Claude Code\" 2>/dev/null || true"
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

fn merge_hooks(existing: Option<&Value>, desired: Option<&Value>) -> Option<Value> {
    let mut result = existing.cloned().unwrap_or_else(|| json!({}));
    let desired = match desired { Some(v) => v, None => return Some(result) };

    // Merge arrays by matcher for PreToolUse and PostToolUse. Append missing hooks, avoid duplicates.
    if let Some(obj) = result.as_object_mut() {
        for key in ["PreToolUse", "PostToolUse"].iter() {
            let desired_arr = desired.get(*key).and_then(|v| v.as_array());
            if desired_arr.is_none() { continue; }
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
                if desired_matcher.is_none() { continue; }
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
                                if e == d { return true; }
                                let dc = d.get("command").and_then(|c| c.as_str());
                                let ec = e.get("command").and_then(|c| c.as_str());
                                match (dc, ec) { (Some(a), Some(b)) => a == b, _ => false }
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
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    Path::new(&home).join(".claude").join("settings.json")
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

// Loader
struct Spinner {
    pb: ProgressBar,
    _handle: smol::Task<()>,
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

        // Start the auto-ticking task
        let pb_clone = pb.clone();
        let _handle = smol::spawn(async move {
            loop {
                pb_clone.tick();
                smol::Timer::after(std::time::Duration::from_millis(100)).await;
            }
        });

        Self { pb, _handle }
    }

    fn start(&self) {
        // Spinner starts automatically when created
    }

    fn update_message(&self, message: &str) {
        self.pb.set_message(message.to_string());
    }

    async fn wait_for(&self, duration_ms: u64) {
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
