use crate::git::cli_parser::{ParsedGitInvocation, is_dry_run};
use crate::git::find_repository;
use crate::git::refs::AI_AUTHORSHIP_REFSPEC;
use crate::git::repository::exec_git;
use crate::utils::debug_log;

pub fn fetch_post_command_hook(
    parsed_args: &ParsedGitInvocation,
    exit_status: std::process::ExitStatus,
) {
    if is_dry_run(&parsed_args.command_args) || !exit_status.success() {
        return;
    }

    // TODO Take into account global args

    // Find the git repository
    let repo = match find_repository(parsed_args.command_args.clone()) {
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
        let fetch_authorship = vec![
            "-c".to_string(),
            "core.hooksPath=/dev/null".to_string(),
            "fetch".to_string(),
            "--no-tags".to_string(),
            "--recurse-submodules=no".to_string(),
            "--no-write-fetch-head".to_string(),
            "--no-write-commit-graph".to_string(),
            "--no-auto-maintenance".to_string(),
            remote,
            AI_AUTHORSHIP_REFSPEC.to_string(),
        ];
        debug_log(&format!(
            "fetching authorship refs: {:?}",
            &fetch_authorship
        ));
        let fetch_res = exec_git(&fetch_authorship);
        if let Err(e) = fetch_res {
            eprintln!("Failed to fetch authorship refs: {}", e);
            std::process::exit(1);
        }
    } else {
        eprintln!("Failed to fetch authorship refs: No git remotes found.");
        std::process::exit(1);
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
