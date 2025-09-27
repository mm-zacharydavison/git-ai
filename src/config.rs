use std::env;
use std::path::Path;
use std::sync::OnceLock;

/// Centralized configuration for the application
pub struct Config {
    git_path: String,
}

static CONFIG: OnceLock<Config> = OnceLock::new();

impl Config {
    /// Initialize the global configuration exactly once.
    /// Safe to call multiple times; subsequent calls are no-ops.
    pub fn init() {
        let _ = CONFIG.get_or_init(|| Config {
            git_path: resolve_git_path(),
        });
    }

    /// Access the global configuration. Panics if not initialized.
    pub fn get() -> &'static Config {
        CONFIG.get().expect("Config not initialized. Call Config::init() early in main().")
    }

    /// Returns the command to invoke git.
    pub fn git_cmd(&self) -> &str {
        &self.git_path
    }
}

fn resolve_git_path() -> String {
    // 1) Environment override
    if let Ok(val) = env::var("GIT_AI_GIT") {
        if !val.trim().is_empty() {
            return val;
        }
    }

    // 2) Probe common locations across platforms
    // Note: We intentionally do not attempt heavy PATH searches here to keep startup fast;
    // we rely on common absolute paths, and finally the bare "git" which allows PATH resolution.
    let candidates: &[&str] = &[
        // macOS Homebrew (ARM and Intel)
        "/opt/homebrew/bin/git",
        "/usr/local/bin/git",
        // Common Unix paths
        "/usr/bin/git",
        "/bin/git",
        "/usr/local/sbin/git",
        "/usr/sbin/git",
        // Windows Git for Windows (if running under compatibility layers)
        r"C:\\Program Files\\Git\\bin\\git.exe",
        r"C:\\Program Files (x86)\\Git\\bin\\git.exe",
    ];

    if let Some(found) = candidates.iter().map(Path::new).find(|p| is_executable(p)) {
        return found.to_string_lossy().to_string();
    }

    // 3) Fallback: rely on system PATH
    // TODO Deal with the fact that this might be a recursive call back to git-ai. Should we warn or even error out?
    "git".to_string()
}

fn is_executable(path: &Path) -> bool {
    if !path.exists() || !path.is_file() {
        return false;
    }
    // Basic check: existence is sufficient for our purposes; OS will enforce exec perms.
    // On Unix we could check permissions, but many filesystems differ. Keep it simple.
    true
}


