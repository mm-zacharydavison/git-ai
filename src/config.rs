use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::Deserialize;

use crate::git::repository::Repository;

/// Centralized configuration for the application
pub struct Config {
    git_path: String,
    ignore_prompts: bool,
    allow_repositories: HashSet<String>,
}
#[derive(Deserialize)]
struct FileConfig {
    #[serde(default)]
    git_path: Option<String>,
    #[serde(default)]
    ignore_prompts: Option<bool>,
    #[serde(default)]
    allow_repositories: Option<Vec<String>>,
}

static CONFIG: OnceLock<Config> = OnceLock::new();

impl Config {
    /// Initialize the global configuration exactly once.
    /// Safe to call multiple times; subsequent calls are no-ops.
    #[allow(dead_code)]
    pub fn init() {
        let _ = CONFIG.get_or_init(|| build_config());
    }

    /// Access the global configuration. Lazily initializes if not already initialized.
    pub fn get() -> &'static Config {
        CONFIG.get_or_init(|| build_config())
    }

    /// Returns the command to invoke git.
    pub fn git_cmd(&self) -> &str {
        &self.git_path
    }

    pub fn get_ignore_prompts(&self) -> bool {
        self.ignore_prompts
    }

    pub fn is_allowed_repository(&self, repository: &Option<Repository>) -> bool {
        // If allowlist is empty, allow everything
        if self.allow_repositories.is_empty() {
            return true;
        }

        // If allowlist is defined, only allow repos whose remotes match the list
        if let Some(repository) = repository {
            match repository.remotes_with_urls().ok() {
                Some(remotes) => remotes
                    .iter()
                    .any(|remote| self.allow_repositories.contains(&remote.1)),
                None => false, // Can't verify, deny by default when allowlist is active
            }
        } else {
            false // No repository provided, deny by default when allowlist is active
        }
    }

    /// Returns whether prompts should be ignored (currently unused by internal APIs).
    #[allow(dead_code)]
    pub fn ignore_prompts(&self) -> bool {
        self.ignore_prompts
    }
}

fn build_config() -> Config {
    let file_cfg = load_file_config();
    let ignore_prompts = file_cfg
        .as_ref()
        .and_then(|c| c.ignore_prompts)
        .unwrap_or(false);
    let allow_repositories = file_cfg
        .as_ref()
        .and_then(|c| c.allow_repositories.clone())
        .unwrap_or(vec![])
        .into_iter()
        .collect();

    let git_path = resolve_git_path(&file_cfg);

    Config {
        git_path,
        ignore_prompts,
        allow_repositories,
    }
}

fn resolve_git_path(file_cfg: &Option<FileConfig>) -> String {
    // 1) From config file
    if let Some(cfg) = file_cfg {
        if let Some(path) = cfg.git_path.as_ref() {
            let trimmed = path.trim();
            if !trimmed.is_empty() {
                let p = Path::new(trimmed);
                if is_executable(p) {
                    return trimmed.to_string();
                }
            }
        }
    }

    // 2) Probe common locations across platforms
    let candidates: &[&str] = &[
        // macOS Homebrew (ARM and Intel)
        "/opt/homebrew/bin/git",
        "/usr/local/bin/git",
        // Common Unix paths
        "/usr/bin/git",
        "/bin/git",
        "/usr/local/sbin/git",
        "/usr/sbin/git",
        // Windows Git for Windows
        r"C:\\Program Files\\Git\\bin\\git.exe",
        r"C:\\Program Files (x86)\\Git\\bin\\git.exe",
    ];

    if let Some(found) = candidates.iter().map(Path::new).find(|p| is_executable(p)) {
        return found.to_string_lossy().to_string();
    }

    // 3) Fatal error: no real git found
    eprintln!(
        "Fatal: Could not locate a real 'git' binary.\n\
         Expected a valid 'git_path' in {cfg_path} or in standard locations.\n\
         Please install Git or update your config JSON.",
        cfg_path = config_file_path()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "~/.git-ai/config.json".to_string()),
    );
    std::process::exit(1);
}

fn load_file_config() -> Option<FileConfig> {
    let path = config_file_path()?;
    let data = fs::read(&path).ok()?;
    serde_json::from_slice::<FileConfig>(&data).ok()
}

fn config_file_path() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let home = env::var("USERPROFILE").ok()?;
        Some(Path::new(&home).join(".git-ai").join("config.json"))
    }
    #[cfg(not(windows))]
    {
        let home = env::var("HOME").ok()?;
        Some(Path::new(&home).join(".git-ai").join("config.json"))
    }
}

fn is_executable(path: &Path) -> bool {
    if !path.exists() || !path.is_file() {
        return false;
    }
    // Basic check: existence is sufficient for our purposes; OS will enforce exec perms.
    // On Unix we could check permissions, but many filesystems differ. Keep it simple.
    true
}
