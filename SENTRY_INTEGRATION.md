# Sentry Integration

This document describes the Sentry integration implemented in git-ai.

## Architecture

The integration uses a file-based logging approach to ensure zero latency impact on git operations:

1. **Logging**: Events are written to `.git/ai/logs/{pid}.log` files in newline-delimited JSON format
2. **Flushing**: A background `flush-logs` command reads all log files (except current process) and sends them to Sentry
3. **Background Execution**: During `git push` and `git fetch` operations, a detached flush process is spawned

## Configuration

git-ai supports **dual Sentry DSN** configuration, allowing both OSS maintainers and enterprises to collect telemetry simultaneously.

### Dual DSN Support

**Two independent Sentry instances:**

1. **`SENTRY_OSS`** - For open source maintainers
   - Set at build time for public releases
   - Can be disabled at runtime by setting to empty string: `SENTRY_OSS=""`
   
2. **`SENTRY_ENTERPRISE`** - For enterprise deployments
   - Off by default (not set in public builds)
   - Enterprises set this in their own builds or at runtime

**Events are sent to BOTH configured instances** (if both are set).

### Build-time Configuration

```bash
# Public OSS release (your DSN)
SENTRY_OSS="https://your-oss-key@o0.ingest.sentry.io/123" cargo build --release

# Enterprise builds their own fork with both DSNs
SENTRY_OSS="https://oss-key@o0.ingest.sentry.io/123" \
SENTRY_ENTERPRISE="https://enterprise-key@o0.ingest.sentry.io/456" \
cargo build --release
```

### Runtime Configuration (Override)

Runtime environment variables take precedence over build-time values:

```bash
# Disable OSS telemetry at runtime
export SENTRY_OSS=""

# Add enterprise telemetry at runtime (without rebuilding)
export SENTRY_ENTERPRISE="https://enterprise-key@o0.ingest.sentry.io/456"
```

### Use Cases

| Scenario | SENTRY_OSS | SENTRY_ENTERPRISE |
|----------|------------|-------------------|
| **Public release** | Set at build | Not set |
| **Enterprise using public binary** | Set at build (can disable) | Set at runtime |
| **Enterprise custom build** | Set at build | Set at build |
| **Disable all telemetry** | `""` at runtime | Not set |

**Metadata differentiation:** Each instance is tagged with `instance: "oss"` or `instance: "enterprise"` in Sentry.

If neither DSN is configured, the flush command will exit early (opt-in telemetry).

## Usage Examples

### Log Errors

```rust
use crate::observability::log_error;
use serde_json::json;

if let Err(e) = some_operation() {
    log_error(&e, Some(json!({
        "command": "checkpoint",
        "repo_path": "/path/to/repo"
    })));
    eprintln!("Operation failed: {}", e);
}
```

### Log Usage Events

```rust
use crate::observability::log_usage_event;
use serde_json::json;

// Track when git-ai is enabled in a repository
log_usage_event("repo_enabled", json!({
    "repo_path": repo.path().display().to_string(),
    "user": user_name
}));
```

### Log Performance Metrics

```rust
use crate::observability::log_performance;
use crate::utils::Timer;
use serde_json::json;

let timer = Timer::default();
let end = timer.start_quiet("checkpoint");

// ... perform operation ...

let duration = end();
log_performance("checkpoint", duration, Some(json!({
    "files_changed": 5,
    "lines_added": 100
})));
```

## Manual Flush

You can manually flush logs to Sentry:

```bash
git-ai flush-logs
```

This is useful for testing or ensuring logs are sent immediately.

## Implementation Details

### Files Modified/Created

1. **Cargo.toml**: Added `sentry` dependency with minimal features
2. **src/lib.rs**: Added `observability` module
3. **src/main.rs**: Added `observability` module
4. **src/observability/mod.rs**: Public logging API
5. **src/observability/flush.rs**: Flush command implementation
6. **src/commands/flush_logs.rs**: Command wrapper
7. **src/commands/mod.rs**: Registered flush_logs module
8. **src/commands/git_ai_handlers.rs**: Wired up flush-logs command
9. **src/git/repo_storage.rs**: Added logs directory to RepoStorage
10. **src/commands/hooks/fetch_hooks.rs**: Added background flush on successful fetch
11. **src/commands/hooks/push_hooks.rs**: Added background flush on successful push

### Log File Format

Each log file contains newline-delimited JSON envelopes:

```json
{"type":"error","timestamp":"2025-10-29T12:34:56Z","message":"Error message","context":{"key":"value"}}
{"type":"usage","timestamp":"2025-10-29T12:34:57Z","event":"repo_enabled","properties":{"repo_path":"/path"}}
{"type":"performance","timestamp":"2025-10-29T12:34:58Z","operation":"checkpoint","duration_ms":1234,"context":{"files":5}}
```

### Background Flush Process

When `git push` or `git fetch` completes successfully, a detached process is spawned:

```rust
git-ai flush-logs
```

This process:
1. Checks for `SENTRY_DSN` environment variable
2. Finds `.git/ai/logs` directory
3. Reads all log files except current PID
4. Initializes Sentry client
5. Sends events to Sentry
6. Deletes successfully flushed log files
7. Exits

The process runs completely independently and doesn't block the git operation.

## Testing

### Manual Testing

1. Set `SENTRY_DSN` environment variable:
   ```bash
   export SENTRY_DSN="your-sentry-dsn"
   ```

2. Run git commands in a repository:
   ```bash
   cd /path/to/repo
   git fetch  # or git push
   ```

3. Check for log files:
   ```bash
   ls -la .git/ai/logs/
   ```

4. Manually flush logs:
   ```bash
   git-ai flush-logs
   ```

5. Verify events in Sentry dashboard

### Adding Logging to Commands

To add logging to existing commands, simply import and call the logging functions:

```rust
use crate::observability::{log_error, log_usage_event, log_performance};
```

The logging functions are designed to fail silently - they will never panic or affect the command's operation.

