use crate::git::find_repository_in_path;
use crate::utils::debug_log;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

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
        debug_log("No Sentry DSN configured (SENTRY_OSS or SENTRY_ENTERPRISE), skipping log flush");
        std::process::exit(1);
    }

    // Find the .git/ai/logs directory
    let logs_dir = match find_logs_directory() {
        Some(dir) => dir,
        None => {
            debug_log("No .git/ai/logs directory found");
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
            debug_log("Failed to read logs directory");
            std::process::exit(1);
        }
    };

    if log_files.is_empty() {
        debug_log("No log files to flush");
        std::process::exit(1);
    }

    // Try to get repository info for metadata
    let repo = find_repository_in_path(&logs_dir.to_string_lossy()).ok();
    let remotes_info = repo
        .as_ref()
        .and_then(|r| r.remotes_with_urls().ok())
        .unwrap_or_default();

    // Initialize Sentry clients (one or both)
    let (oss_hub, enterprise_hub) = initialize_sentry_hubs(oss_dsn, enterprise_dsn, &remotes_info);

    let mut events_sent = 0;
    let mut files_to_delete = Vec::new();

    // Process each log file and send to both hubs
    for log_file in log_files {
        match process_log_file(&log_file, &oss_hub, &enterprise_hub) {
            Ok(count) => {
                events_sent += count;
                if count > 0 {
                    files_to_delete.push(log_file);
                }
            }
            Err(e) => {
                debug_log(&format!("Failed to process log file {:?}: {}", log_file, e));
            }
        }
    }

    if events_sent > 0 {
        debug_log(&format!("Flushing {} events to Sentry", events_sent));
        // Flush both hubs
        if let Some(hub) = oss_hub.as_ref() {
            hub.client().map(|c| c.flush(Some(Duration::from_secs(2))));
        }
        if let Some(hub) = enterprise_hub.as_ref() {
            hub.client().map(|c| c.flush(Some(Duration::from_secs(2))));
        }
        std::thread::sleep(Duration::from_millis(100)); // Small delay to ensure events are sent

        // Delete successfully flushed log files
        for file_path in files_to_delete {
            let _ = fs::remove_file(&file_path);
            debug_log(&format!("Deleted log file: {:?}", file_path));
        }

        std::process::exit(0);
    } else {
        debug_log("No events to flush");
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

fn initialize_sentry_hubs(
    oss_dsn: Option<String>,
    enterprise_dsn: Option<String>,
    remotes_info: &[(String, String)],
) -> (
    Option<std::sync::Arc<sentry::Hub>>,
    Option<std::sync::Arc<sentry::Hub>>,
) {
    let create_hub = |dsn: String, name: &str| -> std::sync::Arc<sentry::Hub> {
        let client = std::sync::Arc::new(sentry::Client::from((
            dsn,
            sentry::ClientOptions {
                release: sentry::release_name!(),
                ..Default::default()
            },
        )));

        let hub = std::sync::Arc::new(sentry::Hub::new(Some(client), Default::default()));

        // Configure scope for this hub
        hub.configure_scope(|scope| {
            scope.set_tag("instance", name);

            for (remote_name, remote_url) in remotes_info {
                scope.set_tag(&format!("remote.{}", remote_name), remote_url);
            }

            scope.set_tag("os", std::env::consts::OS);
            scope.set_tag("arch", std::env::consts::ARCH);
        });

        hub
    };

    let oss_hub = oss_dsn.map(|dsn| {
        debug_log("Initializing OSS Sentry hub");
        create_hub(dsn, "oss")
    });

    let enterprise_hub = enterprise_dsn.map(|dsn| {
        debug_log("Initializing Enterprise Sentry hub");
        create_hub(dsn, "enterprise")
    });

    (oss_hub, enterprise_hub)
}

fn process_log_file(
    path: &PathBuf,
    oss_hub: &Option<std::sync::Arc<sentry::Hub>>,
    enterprise_hub: &Option<std::sync::Arc<sentry::Hub>>,
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

                // Send to OSS hub if configured
                if let Some(hub) = oss_hub {
                    if send_envelope_to_sentry(&envelope, hub) {
                        sent = true;
                    }
                }

                // Send to Enterprise hub if configured
                if let Some(hub) = enterprise_hub {
                    if send_envelope_to_sentry(&envelope, hub) {
                        sent = true;
                    }
                }

                if sent {
                    count += 1;
                }
            }
            Err(e) => {
                debug_log(&format!("Failed to parse envelope: {}", e));
            }
        }
    }

    Ok(count)
}

fn send_envelope_to_sentry(envelope: &Value, hub: &sentry::Hub) -> bool {
    let event_type = envelope.get("type").and_then(|t| t.as_str());

    match event_type {
        Some("error") => {
            let message = envelope
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("Unknown error");
            let context = envelope.get("context");

            let mut event = sentry::protocol::Event {
                message: Some(message.to_string()),
                level: sentry::protocol::Level::Error,
                ..Default::default()
            };

            if let Some(ctx) = context {
                if let Some(obj) = ctx.as_object() {
                    let mut extra = BTreeMap::new();
                    for (key, value) in obj {
                        extra.insert(key.clone(), value.clone());
                    }
                    event.extra = extra;
                }
            }

            hub.capture_event(event);
            true
        }
        Some("usage") => {
            let event_name = envelope
                .get("event")
                .and_then(|e| e.as_str())
                .unwrap_or("unknown");
            let properties = envelope.get("properties");

            let mut event = sentry::protocol::Event {
                message: Some(format!("Usage: {}", event_name)),
                level: sentry::protocol::Level::Info,
                ..Default::default()
            };

            if let Some(props) = properties {
                if let Some(obj) = props.as_object() {
                    let mut extra = BTreeMap::new();
                    for (key, value) in obj {
                        extra.insert(key.clone(), value.clone());
                    }
                    event.extra = extra;
                }
            }

            hub.capture_event(event);
            true
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

            let mut event = sentry::protocol::Event {
                message: Some(format!("Performance: {} ({}ms)", operation, duration_ms)),
                level: sentry::protocol::Level::Info,
                ..Default::default()
            };

            let mut extra = BTreeMap::new();
            extra.insert("duration_ms".to_string(), Value::from(duration_ms));
            extra.insert("operation".to_string(), Value::from(operation));

            if let Some(ctx) = context {
                if let Some(obj) = ctx.as_object() {
                    for (key, value) in obj {
                        extra.insert(key.clone(), value.clone());
                    }
                }
            }

            event.extra = extra;

            hub.capture_event(event);
            true
        }
        _ => {
            debug_log(&format!("Unknown event type: {:?}", event_type));
            false
        }
    }
}
