use crate::git::cli_parser::{ParsedGitInvocation, is_dry_run};
use crate::git::find_repository;
use crate::git::refs::AI_AUTHORSHIP_REFSPEC;
use crate::git::repository::{exec_git, get_default_remote};
use crate::utils::debug_log;

pub fn push_post_command_hook(
    parsed_args: &ParsedGitInvocation,
    exit_status: std::process::ExitStatus,
) {
    if is_dry_run(&parsed_args.command_args)
        || !exit_status.success()
        || parsed_args
            .command_args
            .iter()
            .any(|a| a == "-d" || a == "--delete")
        || parsed_args.command_args.iter().any(|a| a == "--mirror")
    {
        return;
    }

    // TODO Take into account global args
    // TODO Migrate off of libgit2

    // Find the git repository
    let repo = match find_repository() {
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

    // 2) Push authorship refs to the appropriate remote
    let positional_remote = extract_remote_from_push_args(&parsed_args.command_args, &remote_names);

    let specified_remote = positional_remote.or_else(|| {
        parsed_args
            .command_args
            .iter()
            .find(|a| remote_names.iter().any(|r| r == *a))
            .cloned()
    });
    // If not specified, try to get upstream remote of current branch
    fn upstream_remote(repo: &git2::Repository) -> Option<String> {
        let head = repo.head().ok()?;
        if !head.is_branch() {
            return None;
        }
        let branch_name = head.shorthand()?;
        let branch = repo
            .find_branch(branch_name, git2::BranchType::Local)
            .ok()?;
        let upstream = branch.upstream().ok()?;
        let upstream_name = upstream.name().ok()??; // e.g., "origin/main"
        let remote = upstream_name.split('/').next()?.to_string();
        Some(remote)
    }

    let remote = specified_remote
        .or_else(|| upstream_remote(&repo))
        .or_else(|| get_default_remote(&repo));

    if let Some(remote) = remote {
        // Build the internal authorship push with explicit flags and disabled hooks
        let push_authorship = vec![
            "-c".to_string(),
            "core.hooksPath=/dev/null".to_string(),
            "push".to_string(),
            "--quiet".to_string(),
            "--no-recurse-submodules".to_string(),
            "--no-verify".to_string(),
            remote,
            AI_AUTHORSHIP_REFSPEC.to_string(),
        ];
        debug_log(&format!("pushing authorship refs: {:?}", &push_authorship));
        let push_res = exec_git(&push_authorship);
        if let Err(e) = push_res {
            eprintln!("Failed to push authorship refs: {}", e);
            std::process::exit(1);
        }
    } else {
        eprintln!("No git remotes found.");
        std::process::exit(1);
    }
}

fn extract_remote_from_push_args(args: &[String], known_remotes: &[String]) -> Option<String> {
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--" {
            return args.get(i + 1).cloned();
        }
        if arg.starts_with('-') {
            if let Some((flag, value)) = is_push_option_with_inline_value(arg) {
                if flag == "--repo" {
                    return Some(value.to_string());
                }
                i += 1;
                continue;
            }

            if option_consumes_separate_value(arg.as_str()) {
                if arg == "--repo" {
                    return args.get(i + 1).cloned();
                }
                i += 2;
                continue;
            }

            i += 1;
            continue;
        }
        return Some(arg.clone());
    }

    known_remotes
        .iter()
        .find(|r| args.iter().any(|arg| arg == *r))
        .cloned()
}

fn is_push_option_with_inline_value(arg: &str) -> Option<(&str, &str)> {
    if let Some((flag, value)) = arg.split_once('=') {
        Some((flag, value))
    } else if (arg.starts_with("-C") || arg.starts_with("-c")) && arg.len() > 2 {
        // Treat -C<path> or -c<name>=<value> as inline values
        let flag = &arg[..2];
        let value = &arg[2..];
        Some((flag, value))
    } else {
        None
    }
}

fn option_consumes_separate_value(arg: &str) -> bool {
    matches!(
        arg,
        "--repo" | "--receive-pack" | "--exec" | "-o" | "--push-option" | "-c" | "-C"
    )
}
