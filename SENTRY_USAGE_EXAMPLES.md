# Sentry Usage Examples

This document provides practical examples of how to integrate Sentry logging into git-ai commands.

## Example 0: Setting Repository Context (Required)

```rust
// In src/commands/git_handlers.rs - early in handle_git()

use crate::observability;

pub fn handle_git(args: &[String]) {
    // ... parse args ...
    
    let mut repository_option = find_repository(&parsed_args.global_args).ok();
    
    // Set observability context as soon as we have a repository
    if let Some(ref repo) = repository_option {
        observability::set_repo_context(repo);
    }
    
    // Now all log_error/log_performance calls will write to disk
    // ... rest of function ...
}
```

**Note:** Call `set_repo_context()` as early as possible once you have a `Repository` instance. Events logged before this will be buffered in memory and flushed when context is set.

## Example 1: Logging Errors in install-hooks Command

```rust
// In src/commands/install_hooks.rs

use crate::observability::log_error;
use serde_json::json;

pub fn run(args: &[String]) -> Result<(), GitAiError> {
    // ... existing code ...
    
    if let Err(e) = install_vscode_hooks() {
        // Log the error to Sentry
        log_error(&e, Some(json!({
            "command": "install-hooks",
            "hook_type": "vscode"
        })));
        
        // Still return the error for normal error handling
        return Err(e);
    }
    
    // ... rest of code ...
    Ok(())
}
```

## Example 2: Logging Usage Event When Repository is Enabled

```rust
// In src/commands/install_hooks.rs (on successful completion)

use crate::observability::log_usage_event;
use serde_json::json;

pub fn run(args: &[String]) -> Result<(), GitAiError> {
    // ... installation logic ...
    
    // Log successful repository enablement
    log_usage_event("repo_enabled", json!({
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "cwd": std::env::current_dir()
            .ok()
            .and_then(|p| p.to_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "unknown".to_string())
    }));
    
    Ok(())
}
```

## Example 3: Logging Performance for Checkpoint Operations

```rust
// In src/commands/checkpoint.rs

use crate::observability::log_performance;
use crate::utils::Timer;
use serde_json::json;

pub fn run(
    repo: &Repository,
    default_user_name: &str,
    checkpoint_kind: CheckpointKind,
    show_working_log: bool,
    reset: bool,
    skip_snapshot: bool,
    agent_run_result: Option<AgentRunResult>,
) -> Result<(), GitAiError> {
    let timer = Timer::default();
    let measure = timer.start_quiet("checkpoint");
    
    // ... existing checkpoint logic ...
    
    let duration = measure();
    
    // Log performance metric
    log_performance("checkpoint", duration, Some(json!({
        "checkpoint_kind": format!("{:?}", checkpoint_kind),
        "has_agent_result": agent_run_result.is_some(),
        "show_working_log": show_working_log,
        "reset": reset
    })));
    
    Ok(())
}
```

## Example 4: Logging Errors in Preset Execution

```rust
// In src/commands/checkpoint_agent/agent_presets.rs

use crate::observability::log_error;
use serde_json::json;

impl AgentCheckpointPreset for CursorPreset {
    fn run(&self, flags: AgentCheckpointFlags) -> Result<AgentRunResult, GitAiError> {
        match self.find_and_parse_cursor_db() {
            Ok(result) => Ok(result),
            Err(e) => {
                // Log preset errors
                log_error(&e, Some(json!({
                    "preset": "cursor",
                    "hook_input_provided": flags.hook_input.is_some()
                })));
                Err(e)
            }
        }
    }
}
```

## Example 5: Logging Performance for Long-Running Git Operations

```rust
// In src/commands/hooks/fetch_hooks.rs

use crate::observability::log_performance;
use crate::utils::Timer;
use serde_json::json;

pub fn fetch_pull_post_command_hook(
    repository: &Repository,
    parsed_args: &ParsedGitInvocation,
    exit_status: std::process::ExitStatus,
    command_hooks_context: &mut CommandHooksContext,
) {
    if let Some(handle) = command_hooks_context.fetch_authorship_handle.take() {
        let timer = Timer::default();
        let measure = timer.start_quiet("authorship_fetch_wait");
        
        let _ = handle.join();
        
        let duration = measure();
        
        // Log how long we waited for authorship sync
        if duration.as_millis() > 100 {  // Only log if significant
            log_performance("authorship_fetch", duration, Some(json!({
                "command": parsed_args.command.as_deref().unwrap_or("unknown"),
                "exit_success": exit_status.success()
            })));
        }
    }

    if exit_status.success() {
        crate::observability::spawn_background_flush();
    }
}
```

## Example 6: Logging Critical Errors in Git Wrapper

```rust
// In src/commands/git_handlers.rs

use crate::observability::log_error;
use serde_json::json;

fn proxy_to_git(args: &[String], exit_on_completion: bool) -> std::process::ExitStatus {
    let child = Command::new(config::Config::get().git_cmd())
        .args(args)
        .spawn();

    match child {
        Ok(mut child) => {
            // ... wait for child ...
        }
        Err(e) => {
            // Log critical failure to execute git
            log_error(&e, Some(json!({
                "context": "proxy_to_git",
                "git_command": args.get(0).map(|s| s.as_str()).unwrap_or("unknown"),
                "git_path": config::Config::get().git_cmd()
            })));
            
            eprintln!("Failed to execute git command: {}", e);
            std::process::exit(1);
        }
    }
}
```

## Best Practices

1. **Always include context**: Add relevant context to help debug issues
2. **Don't log PII**: Avoid logging user email addresses or sensitive data
3. **Log actionable metrics**: Focus on metrics that help improve the product
4. **Fail silently**: The logging functions already fail gracefully, but ensure your code continues to work even if logging fails
5. **Log performance for slow operations**: Focus on operations that users might notice (> 100ms)
6. **Use structured data**: Always use JSON objects for context, not strings

## Environment Setup for Testing

### Testing OSS Telemetry

```bash
# Build with OSS DSN
SENTRY_OSS="https://oss-key@o0.ingest.sentry.io/123" cargo build --release

# Run git operations to trigger logging
cd /path/to/test/repo
git fetch
git push

# Check the log files (before flush)
ls -la .git/ai/logs/

# Manually trigger flush to see events in Sentry
git-ai flush-logs
```

### Testing Dual DSN (OSS + Enterprise)

```bash
# Build with OSS DSN
SENTRY_OSS="https://oss-key@o0.ingest.sentry.io/123" cargo build --release

# Enterprise adds their DSN at runtime
export SENTRY_ENTERPRISE="https://enterprise-key@o0.ingest.sentry.io/456"
git-ai flush-logs  # Events sent to BOTH Sentry instances

# Check events in both Sentry projects - filter by:
# - instance: "oss" (in OSS Sentry)
# - instance: "enterprise" (in Enterprise Sentry)
```

### Testing Disable OSS Telemetry

```bash
# Disable OSS telemetry at runtime
export SENTRY_OSS=""
export SENTRY_ENTERPRISE="https://enterprise-key@o0.ingest.sentry.io/456"
git-ai flush-logs  # Only sends to Enterprise Sentry
```

## Verifying Events in Sentry

1. Go to your Sentry dashboard
2. Navigate to "Issues" to see error events
3. Navigate to "Performance" to see performance metrics
4. Use the search to filter by event type or context fields
5. Set up alerts based on error frequency or performance degradation

