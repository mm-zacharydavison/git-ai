use crate::git::find_repository_in_path;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

/// Handle the flush-logs command
pub fn handle_flush_logs(_args: &[String]) {
    // Check for OSS DSN: runtime env var takes precedence over build-time value
    // Can be explicitly disabled with empty string
    let oss_dsn = std::env::var("SENTRY_OSS")
        .ok()
        .or_else(|| option_env!("SENTRY_OSS").map(|s| s.to_string()))
        .filter(|s| !s.is_empty());

    // Check for Enterprise DSN: runtime env var takes precedence over build-time value
    // Off by default unless they build their own fork or set at runtime
    let enterprise_dsn = std::env::var("SENTRY_ENTERPRISE")
        .ok()
        .or_else(|| option_env!("SENTRY_ENTERPRISE").map(|s| s.to_string()))
        .filter(|s| !s.is_empty());

    // Need at least one DSN to proceed
    if oss_dsn.is_none() && enterprise_dsn.is_none() {
        std::process::exit(1);
    }

    // Find the .git/ai/logs directory
    let logs_dir = match find_logs_directory() {
        Some(dir) => dir,
        None => {
            std::process::exit(1);
        }
    };

    // Get current PID to exclude our own log file
    let current_pid = std::process::id();
    let current_log_file = format!("{}.log", current_pid);

    // Read all log files except current PID
    let log_files: Vec<PathBuf> = match fs::read_dir(&logs_dir) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| {
                path.is_file()
                    && path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| n != current_log_file && n.ends_with(".log"))
                        .unwrap_or(false)
            })
            .collect(),
        Err(_) => {
            std::process::exit(1);
        }
    };

    if log_files.is_empty() {
        std::process::exit(1);
    }

    // Try to get repository info for metadata
    let repo = find_repository_in_path(&logs_dir.to_string_lossy()).ok();
    let remotes_info = repo
        .as_ref()
        .and_then(|r| r.remotes_with_urls().ok())
        .unwrap_or_default();

    // Initialize Sentry clients
    let (oss_client, enterprise_client) = initialize_sentry_clients(oss_dsn, enterprise_dsn);

    let mut events_sent = 0;
    let mut files_to_delete = Vec::new();

    // Process each log file and send events
    for log_file in log_files {
        match process_log_file(&log_file, &oss_client, &enterprise_client, &remotes_info) {
            Ok(count) => {
                events_sent += count;
                if count > 0 {
                    files_to_delete.push(log_file);
                }
            }
            Err(_) => {}
        }
    }

    if events_sent > 0 {
        for file_path in files_to_delete {
            let _ = fs::remove_file(&file_path);
        }

        std::process::exit(0);
    } else {
        std::process::exit(1);
    }
}

fn find_logs_directory() -> Option<PathBuf> {
    let mut current = std::env::current_dir().ok()?;

    loop {
        let git_dir = current.join(".git");
        if git_dir.exists() && git_dir.is_dir() {
            let logs_dir = git_dir.join("ai").join("logs");
            if logs_dir.exists() && logs_dir.is_dir() {
                return Some(logs_dir);
            }
        }

        if !current.pop() {
            break;
        }
    }

    None
}

struct SentryClient {
    endpoint: String,
    public_key: String,
}

impl SentryClient {
    fn from_dsn(dsn: &str) -> Option<Self> {
        // Parse DSN: https://PUBLIC_KEY@HOST/PROJECT_ID
        let url = url::Url::parse(dsn).ok()?;
        let public_key = url.username().to_string();
        let host = url.host_str()?;
        let project_id = url.path().trim_start_matches('/');

        let scheme = url.scheme();
        let endpoint = format!("{}://{}/api/{}/store/", scheme, host, project_id);

        Some(SentryClient {
            endpoint,
            public_key,
        })
    }

    fn send_event(&self, event: Value) -> Result<String, Box<dyn std::error::Error>> {
        let auth_header = format!(
            "Sentry sentry_version=7, sentry_key={}, sentry_client=git-ai/{}",
            self.public_key,
            env!("CARGO_PKG_VERSION")
        );

        let body = serde_json::to_string(&event)?;

        let response = minreq::post(&self.endpoint)
            .with_header("X-Sentry-Auth", auth_header)
            .with_header("Content-Type", "application/json")
            .with_body(body)
            .send()?;

        let status = response.status_code;
        let event_id = serde_json::from_str::<Value>(response.as_str()?)
            .ok()
            .and_then(|v| {
                v.get("id")
                    .and_then(|id| id.as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| "unknown".to_string());

        if status >= 200 && status < 300 {
            Ok(event_id)
        } else {
            Err(format!("Sentry returned status {}", status).into())
        }
    }
}

fn initialize_sentry_clients(
    oss_dsn: Option<String>,
    enterprise_dsn: Option<String>,
) -> (Option<SentryClient>, Option<SentryClient>) {
    let oss_client = oss_dsn.and_then(|dsn| SentryClient::from_dsn(&dsn));
    let enterprise_client = enterprise_dsn.and_then(|dsn| SentryClient::from_dsn(&dsn));

    (oss_client, enterprise_client)
}

fn process_log_file(
    path: &PathBuf,
    oss_client: &Option<SentryClient>,
    enterprise_client: &Option<SentryClient>,
    remotes_info: &[(String, String)],
) -> Result<usize, Box<dyn std::error::Error>> {
    let content = fs::read_to_string(path)?;
    let mut count = 0;

    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }

        match serde_json::from_str::<Value>(line) {
            Ok(envelope) => {
                let mut sent = false;

                // Send to OSS if configured
                if let Some(client) = oss_client {
                    if send_envelope_to_sentry(&envelope, client, remotes_info) {
                        sent = true;
                    }
                }

                // Send to Enterprise if configured
                if let Some(client) = enterprise_client {
                    if send_envelope_to_sentry(&envelope, client, remotes_info) {
                        sent = true;
                    }
                }

                if sent {
                    count += 1;
                }
            }
            Err(_) => {}
        }
    }

    Ok(count)
}

fn send_envelope_to_sentry(
    envelope: &Value,
    client: &SentryClient,
    remotes_info: &[(String, String)],
) -> bool {
    let event_type = envelope.get("type").and_then(|t| t.as_str());
    let timestamp = envelope
        .get("timestamp")
        .and_then(|t| t.as_str())
        .unwrap_or("");

    // Build tags
    let mut tags = BTreeMap::new();
    tags.insert("os".to_string(), json!(std::env::consts::OS));
    tags.insert("arch".to_string(), json!(std::env::consts::ARCH));
    for (remote_name, remote_url) in remotes_info {
        tags.insert(format!("remote.{}", remote_name), json!(remote_url));
    }

    let event = match event_type {
        Some("error") => {
            let message = envelope
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("Unknown error");
            let context = envelope.get("context");

            let mut extra = BTreeMap::new();
            if let Some(ctx) = context {
                if let Some(obj) = ctx.as_object() {
                    for (key, value) in obj {
                        extra.insert(key.clone(), value.clone());
                    }
                }
            }

            json!({
                "message": message,
                "level": "error",
                "timestamp": timestamp,
                "platform": "other",
                "tags": tags,
                "extra": extra,
                "release": format!("git-ai@{}", env!("CARGO_PKG_VERSION")),
            })
        }
        Some("performance") => {
            let operation = envelope
                .get("operation")
                .and_then(|o| o.as_str())
                .unwrap_or("unknown");
            let duration_ms = envelope
                .get("duration_ms")
                .and_then(|d| d.as_u64())
                .unwrap_or(0);
            let context = envelope.get("context");

            let mut extra = BTreeMap::new();
            extra.insert("operation".to_string(), json!(operation));
            extra.insert("duration_ms".to_string(), json!(duration_ms));
            if let Some(ctx) = context {
                if let Some(obj) = ctx.as_object() {
                    for (key, value) in obj {
                        extra.insert(key.clone(), value.clone());
                    }
                }
            }

            json!({
                "message": format!("Performance: {} ({}ms)", operation, duration_ms),
                "level": "info",
                "timestamp": timestamp,
                "platform": "other",
                "tags": tags,
                "extra": extra,
                "release": format!("git-ai@{}", env!("CARGO_PKG_VERSION")),
            })
        }
        Some("message") => {
            let message = envelope
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("Unknown message");
            let level = envelope
                .get("level")
                .and_then(|l| l.as_str())
                .unwrap_or("info");
            let context = envelope.get("context");

            let mut extra = BTreeMap::new();
            if let Some(ctx) = context {
                if let Some(obj) = ctx.as_object() {
                    for (key, value) in obj {
                        extra.insert(key.clone(), value.clone());
                    }
                }
            }

            json!({
                "message": message,
                "level": level,
                "timestamp": timestamp,
                "platform": "other",
                "tags": tags,
                "extra": extra,
                "release": format!("git-ai@{}", env!("CARGO_PKG_VERSION")),
            })
        }
        _ => {
            return false;
        }
    };

    match client.send_event(event) {
        Ok(_) => true,
        Err(_) => false,
    }
}
