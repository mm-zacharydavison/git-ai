use crate::config;
use crate::error::GitAiError;
use git2::Repository;
use std::process::{Command, Output};

pub fn find_repository() -> Result<Repository, GitAiError> {
    Repository::discover(".").map_err(GitAiError::GitError)
}

pub fn find_repository_in_path(path: &str) -> Result<Repository, GitAiError> {
    Repository::discover(path).map_err(GitAiError::GitError)
}

pub fn get_default_remote(repo: &git2::Repository) -> Option<String> {
    if let Ok(remotes) = repo.remotes() {
        if remotes.len() == 0 {
            return None;
        }
        // Prefer 'origin' if it exists
        for i in 0..remotes.len() {
            if let Some(name) = remotes.get(i) {
                if name == "origin" {
                    return Some("origin".to_string());
                }
            }
        }
        // Otherwise, just use the first remote
        remotes.get(0).map(|s| s.to_string())
    } else {
        None
    }
}

/// Helper to execute a git command
pub fn exec_git(args: &[String]) -> Result<Output, GitAiError> {
    // TODO Make sure to handle process signals, etc.
    let output = Command::new(config::Config::get().git_cmd())
        .args(args)
        .output()
        .map_err(GitAiError::IoError)?;

    if !output.status.success() {
        let code = output.status.code();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(GitAiError::GitCliError { code, stderr });
    }

    Ok(output)
}
