use crate::config;
use crate::error::GitAiError;
use git2::Repository;
use std::io::Write;
use std::process::Command;

pub fn find_repository() -> Result<Repository, GitAiError> {
    Repository::discover(".").map_err(GitAiError::GitError)
}

pub fn find_repository_in_path(path: &str) -> Result<Repository, GitAiError> {
    Repository::discover(path).map_err(GitAiError::GitError)
}

/// Helper to run a git command and optionally forward output, returning ExitStatus
pub fn run_git_and_forward(args: &[String], quiet: bool) -> std::process::ExitStatus {
    let output = Command::new(config::Config::get().git_cmd())
        .args(args)
        .output();
    match output {
        Ok(output) => {
            if !quiet {
                if !output.stdout.is_empty() {
                    let _ = std::io::stdout().write_all(&output.stdout);
                }
                if !output.stderr.is_empty() {
                    let _ = std::io::stderr().write_all(&output.stderr);
                }
            }
            output.status
        }
        Err(e) => {
            if !quiet {
                eprintln!("Failed to execute git command: {}", e);
            }
            std::process::exit(1);
        }
    }
}
