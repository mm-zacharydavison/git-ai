use crate::git::cli_parser::{ParsedGitInvocation, is_dry_run};
use crate::git::find_repository;
use crate::git::refs::AI_AUTHORSHIP_REFSPEC;
use crate::git::repository::{Repository, exec_git};
use crate::utils::debug_log;

pub fn fetch_post_command_hook(
    parsed_args: &ParsedGitInvocation,
    exit_status: std::process::ExitStatus,
    repository_option: &Option<Repository>,
) {
    if is_dry_run(&parsed_args.command_args) || !exit_status.success() {
        return;
    }

    // Find the git repository
    let repo = match find_repository(parsed_args.global_args.clone()) {
        Ok(repo) => repo,
        Err(e) => {
            eprintln!("Failed to find repository: {}", e);
            std::process::exit(1);
        }
    };

    let remotes = repo.remotes().ok();
    let remote_names: Vec<String> = remotes
        .as_ref()
        .map(|r| {
            (0..r.len())
                .filter_map(|i| r.get(i).map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    // 2) Fetch authorship refs from the appropriate remote
    // Try to detect remote (named remote, URL, or local path) from args first
    let positional_remote = extract_remote_from_fetch_args(&parsed_args.command_args);
    let specified_remote = positional_remote.or_else(|| {
        parsed_args
            .command_args
            .iter()
            .find(|a| remote_names.iter().any(|r| r == *a))
            .cloned()
    });

    let remote = specified_remote
        .or_else(|| repo.upstream_remote().ok().flatten())
        .or_else(|| repo.get_default_remote().ok().flatten());

    if let Some(remote) = remote {
        // Build the internal authorship fetch with explicit flags and disabled hooks
        // IMPORTANT: run in the same repo context by prefixing original global args (e.g., -C <path>)
        let mut fetch_authorship: Vec<String> = parsed_args.global_args.clone();
        fetch_authorship.push("-c".to_string());
        fetch_authorship.push("core.hooksPath=/dev/null".to_string());
        fetch_authorship.push("fetch".to_string());
        fetch_authorship.push("--no-tags".to_string());
        fetch_authorship.push("--recurse-submodules=no".to_string());
        fetch_authorship.push("--no-write-fetch-head".to_string());
        fetch_authorship.push("--no-write-commit-graph".to_string());
        fetch_authorship.push("--no-auto-maintenance".to_string());
        fetch_authorship.push(remote);
        fetch_authorship.push(AI_AUTHORSHIP_REFSPEC.to_string());
        debug_log(&format!(
            "fetching authorship refs: {:?}",
            &fetch_authorship
        ));
        if let Err(e) = exec_git(&fetch_authorship) {
            // Treat as best-effort; do not fail the user command if authorship sync fails
            debug_log(&format!("authorship fetch skipped due to error: {}", e));
        }
    } else {
        // No remotes to sync from; silently skip
        debug_log("no remotes found for authorship fetch; skipping");
    }
}

fn extract_remote_from_fetch_args(args: &[String]) -> Option<String> {
    let mut after_double_dash = false;

    for arg in args {
        if !after_double_dash {
            if arg == "--" {
                after_double_dash = true;
                continue;
            }
            if arg.starts_with('-') {
                // Option; skip
                continue;
            }
        }

        // Candidate positional arg; determine if it's a repository URL/path
        let s = arg.as_str();

        // 1) URL forms (https://, ssh://, file://, git://, etc.)
        if s.contains("://") || s.starts_with("file://") {
            return Some(arg.clone());
        }

        // 2) SCP-like syntax: user@host:path
        if s.contains('@') && s.contains(':') && !s.contains("://") {
            return Some(arg.clone());
        }

        // 3) Local path forms
        if s.starts_with('/') || s.starts_with("./") || s.starts_with("../") || s.starts_with("~/")
        {
            return Some(arg.clone());
        }

        // Heuristic: bare repo directories often end with .git
        if s.ends_with(".git") {
            return Some(arg.clone());
        }

        // 4) As a last resort, if the path exists on disk, treat as local path
        if std::path::Path::new(s).exists() {
            return Some(arg.clone());
        }

        // Otherwise, do not treat this positional token as a repository; likely a refspec
        break;
    }

    None
}
