mod commands;
mod config;
mod error;
mod git;
mod log_fmt;
mod utils;

use clap::Parser;
use git::find_repository;
use git::refs::AI_AUTHORSHIP_REFSPEC;
use git::repository::run_git_and_forward;
use std::io::IsTerminal;

#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;
use std::process::Command;
use utils::debug_log;

use crate::commands::checkpoint_agent::agent_preset::{
    AgentCheckpointFlags, AgentCheckpointPreset, ClaudePreset, CursorPreset,
};
use crate::git::find_repository_in_path;

#[derive(Parser)]
#[command(name = "git-ai")]
#[command(about = "git proxy with AI authorship tracking", long_about = None)]
#[command(disable_help_flag = true, disable_version_flag = true)]
struct Cli {
    /// Git command and arguments
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
}

fn main() {
    // Ensure SIGPIPE uses the default action (terminate), and do not inherit ignored SIGPIPE
    reset_sigpipe_to_default();
    // Initialize global configuration early
    config::Config::init();
    // Get the binary name that was called
    let binary_name = std::env::args_os()
        .next()
        .and_then(|arg| arg.into_string().ok())
        .and_then(|path| {
            std::path::Path::new(&path)
                .file_name()
                .and_then(|name| name.to_str())
                .map(|s| s.to_string())
        })
        .unwrap_or("git-ai".to_string());

    let cli = Cli::parse();
    if cli.args.is_empty() {
        if binary_name == "git" {
            // User called 'git'
            proxy_to_git(&[]);
        } else {
            // User called 'git-ai', show git-ai specific help
            print_help();
        }
        return;
    }

    let command = &cli.args[0];
    let args = &cli.args[1..];

    match command.as_str() {
        "stats-delta" => {
            handle_stats_delta(args);
        }
        "checkpoint" => {
            handle_checkpoint(args);
        }
        "blame" => {
            debug_log(&format!("overriding: git blame"));
            handle_blame(args);
        }
        "commit" => {
            // debug_log(&format!("wrapping: git commit"));
            handle_commit(args);
        }
        "pre-commit" => {
            // Backwards compatibility: do nothing and exit 0
            std::process::exit(0);
        }
        "post-commit" => {
            // Backwards compatibility: do nothing and exit 0
            std::process::exit(0);
        }
        "fetch" => {
            handle_fetch(args);
        }
        "push" => {
            handle_push(args);
        }
        "install-hooks" => {
            // This command only works when called as git-ai, not as git alias
            if binary_name == "git" {
                debug_log(&format!("binary_name: {}", binary_name));
                eprintln!(
                    "Error: install-hooks command is only available when called as 'git-ai', not as 'git'"
                );
                std::process::exit(1);
            }

            // This command is not ready for production - only allow in debug builds
            if !cfg!(debug_assertions) {
                eprintln!("Error: install-hooks command is not ready for production");
                std::process::exit(1);
            }

            if let Err(e) = commands::install_hooks::run(args) {
                eprintln!("Install hooks failed: {}", e);
                std::process::exit(1);
            }
        }
        "squash-authorship" => {
            // This command only works when called as git-ai, not as git alias
            if binary_name == "git" {
                eprintln!(
                    "Error: squash-authorship command is only available when called as 'git-ai', not as 'git'"
                );
                std::process::exit(1);
            }

            commands::rebase_authorship::handle_squash_authorship(args);
        }
        _ => {
            // debug_log(&format!("proxying: git {}", command));
            // Proxy all other commands to git
            proxy_to_git(&cli.args);
        }
    }
}

fn handle_checkpoint(args: &[String]) {
    let mut repository_working_dir = std::env::current_dir()
        .unwrap()
        .to_string_lossy()
        .to_string();

    // Parse checkpoint-specific arguments
    let mut author = None;
    let mut show_working_log = false;
    let mut reset = false;
    let mut model = None;
    let mut _prompt_json = None;
    let mut _prompt_path = None;
    let mut prompt_id = None;
    let mut hook_input = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--author" => {
                if i + 1 < args.len() {
                    author = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    eprintln!("Error: --author requires a value");
                    std::process::exit(1);
                }
            }
            "--show-working-log" => {
                show_working_log = true;
                i += 1;
            }
            "--reset" => {
                reset = true;
                i += 1;
            }
            "--model" => {
                if i + 1 < args.len() {
                    model = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    eprintln!("Error: --model requires a value");
                    std::process::exit(1);
                }
            }
            "--prompt" => {
                if i + 1 < args.len() {
                    _prompt_json = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    eprintln!("Error: --prompt requires a JSON value");
                    std::process::exit(1);
                }
            }
            "--prompt-path" => {
                if i + 1 < args.len() {
                    _prompt_path = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    eprintln!("Error: --prompt-path requires a value");
                    std::process::exit(1);
                }
            }
            "--prompt-id" => {
                if i + 1 < args.len() {
                    prompt_id = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    eprintln!("Error: --prompt-id requires a value");
                    std::process::exit(1);
                }
            }
            "--hook-input" => {
                if i + 1 < args.len() {
                    hook_input = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    eprintln!("Error: --hook-input requires a value");
                    std::process::exit(1);
                }
            }

            _ => {
                i += 1;
            }
        }
    }

    let mut agent_run_result = None;
    // Handle preset arguments after parsing all flags
    if !args.is_empty() {
        match args[0].as_str() {
            "claude" => {
                match ClaudePreset.run(AgentCheckpointFlags {
                    prompt_id: prompt_id.clone(),
                    hook_input: hook_input.clone(),
                }) {
                    Ok(agent_run) => {
                        agent_run_result = Some(agent_run);
                    }
                    Err(e) => {
                        eprintln!("Claude preset error: {}", e);
                        std::process::exit(1);
                    }
                }
            }
            "cursor" => {
                match CursorPreset.run(AgentCheckpointFlags {
                    prompt_id: prompt_id.clone(),
                    hook_input: hook_input.clone(),
                }) {
                    Ok(agent_run) => {
                        if agent_run.is_human {
                            agent_run_result = None;
                            if agent_run.repo_working_dir.is_some() {
                                repository_working_dir = agent_run.repo_working_dir.unwrap();
                            }
                        } else {
                            agent_run_result = Some(agent_run);
                        }
                    }
                    Err(e) => {
                        eprintln!("Error running Cursor preset: {}", e);
                        std::process::exit(1);
                    }
                }
            }
            _ => {}
        }
    }

    let final_working_dir = agent_run_result
        .as_ref()
        .and_then(|r| r.repo_working_dir.clone())
        .unwrap_or_else(|| repository_working_dir);
    // Find the git repository
    let repo = match find_repository_in_path(&final_working_dir) {
        Ok(repo) => repo,
        Err(e) => {
            eprintln!("Failed to find repository: {}", e);
            std::process::exit(1);
        }
    };

    // Get the current user name from git config
    let default_user_name = match repo.config() {
        Ok(config) => match config.get_string("user.name") {
            Ok(name) => name,
            Err(_) => {
                eprintln!("Warning: git user.name not configured. Using 'unknown' as author.");
                "unknown".to_string()
            }
        },
        Err(_) => {
            eprintln!("Warning: Failed to get git config. Using 'unknown' as author.");
            "unknown".to_string()
        }
    };

    let final_author = author.as_ref().unwrap_or(&default_user_name);

    if let Err(e) = commands::checkpoint::run(
        &repo,
        final_author,
        show_working_log,
        reset,
        false,
        model.as_deref(),
        Some(&default_user_name),
        agent_run_result,
    ) {
        eprintln!("Checkpoint failed: {}", e);
        std::process::exit(1);
    }
}

fn handle_stats_delta(args: &[String]) {
    // Parse stats-delta-specific arguments
    let mut json_output = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => {
                json_output = true;
                i += 1;
            }
            _ => {
                eprintln!("Unknown stats-delta argument: {}", args[i]);
                std::process::exit(1);
            }
        }
    }

    // Find the git repository
    let repo = match find_repository() {
        Ok(repo) => repo,
        Err(e) => {
            eprintln!("Failed to find repository: {}", e);
            std::process::exit(1);
        }
    };

    if let Err(e) = commands::stats_delta::run(&repo, json_output) {
        eprintln!("Stats delta failed: {}", e);
        std::process::exit(1);
    }
}

fn handle_blame(args: &[String]) {
    if args.is_empty() {
        eprintln!("Error: blame requires a file argument");
        std::process::exit(1);
    }

    // Find the git repository
    let repo = match find_repository() {
        Ok(repo) => repo,
        Err(e) => {
            eprintln!("Failed to find repository: {}", e);
            std::process::exit(1);
        }
    };

    // Parse blame arguments
    let (file_path, options) = match commands::blame::parse_blame_args(args) {
        Ok(result) => result,
        Err(e) => {
            eprintln!("Failed to parse blame arguments: {}", e);
            std::process::exit(1);
        }
    };

    // Check if this is an interactive terminal
    let is_interactive = std::io::stdout().is_terminal();

    if is_interactive && options.incremental {
        // For incremental mode in interactive terminal, we need special handling
        // This would typically involve a pager like less
        let mut full_args = vec!["blame".to_string()];
        full_args.extend_from_slice(args);
        proxy_to_git(&full_args);
        return;
    }

    if let Err(e) = commands::blame::run(&repo, &file_path, &options) {
        eprintln!("Blame failed: {}", e);
        std::process::exit(1);
    }
}

fn get_commit_default_user_name(repo: &git2::Repository, args: &[String]) -> String {
    // According to git commit manual, --author flag overrides all other author information
    if let Some(author_spec) = extract_author_from_args(args) {
        return resolve_author_spec(repo, &author_spec);
    }

    // Normal precedence when --author is not specified:
    // 1. GIT_AUTHOR_NAME environment variable
    // 2. user.name config variable
    // 3. EMAIL environment variable
    // 4. System user name and hostname (we'll use 'unknown' as fallback)

    // Check GIT_AUTHOR_NAME environment variable
    if let Ok(author_name) = std::env::var("GIT_AUTHOR_NAME") {
        if !author_name.trim().is_empty() {
            return author_name.trim().to_string();
        }
    }

    // Fall back to git config user.name
    if let Ok(config) = repo.config() {
        if let Ok(name) = config.get_string("user.name") {
            if !name.trim().is_empty() {
                return name.trim().to_string();
            }
        }
    }

    // Check EMAIL environment variable as fallback
    if let Ok(email) = std::env::var("EMAIL") {
        if !email.trim().is_empty() {
            // Extract name part from email if it contains a name
            if let Some(at_pos) = email.find('@') {
                let name_part = &email[..at_pos];
                if !name_part.is_empty() {
                    return name_part.to_string();
                }
            }
            return email;
        }
    }

    // Final fallback (instead of trying to get system user name and hostname)
    eprintln!("Warning: No author information found. Using 'unknown' as author.");
    "unknown".to_string()
}

fn extract_author_from_args(args: &[String]) -> Option<String> {
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];

        // Handle --author=<author> format
        if let Some(author_value) = arg.strip_prefix("--author=") {
            return Some(author_value.to_string());
        }

        // Handle --author <author> format (separate arguments)
        if arg == "--author" && i + 1 < args.len() {
            return Some(args[i + 1].clone());
        }

        i += 1;
    }
    None
}

fn resolve_author_spec(repo: &git2::Repository, author_spec: &str) -> String {
    // According to git commit docs, --author can be:
    // 1. "A U Thor <author@example.com>" format - use as explicit author
    // 2. A pattern to search for existing commits via git rev-list --all -i --author=<pattern>

    // If it looks like "Name <email>" format, extract the name part
    if let Some(email_start) = author_spec.rfind('<') {
        let name_part = author_spec[..email_start].trim();
        if !name_part.is_empty() {
            return name_part.to_string();
        }
    }

    // If it doesn't look like an explicit format, treat it as a search pattern
    // Try to find an existing commit by that author
    if let Ok(mut revwalk) = repo.revwalk() {
        if revwalk.push_glob("refs/*").is_ok() {
            for oid_result in revwalk {
                if let Ok(oid) = oid_result {
                    if let Ok(commit) = repo.find_commit(oid) {
                        let author = commit.author();
                        if let Some(author_name) = author.name() {
                            // Case-insensitive search (like git rev-list -i --author)
                            if author_name
                                .to_lowercase()
                                .contains(&author_spec.to_lowercase())
                            {
                                return author_name.to_string();
                            }
                        }
                    }
                }
            }
        }
    }

    // If no matching commit found, use the pattern as-is
    author_spec.trim().to_string()
}

fn handle_commit(args: &[String]) {
    let mut full_args = vec!["commit".to_string()];
    full_args.extend_from_slice(args);

    // Check if this is a dry-run - if so, we should not modify any state
    if args.iter().any(|arg| arg == "--dry-run") {
        // For dry-run, just pass through to git without our hooks
        proxy_to_git(&full_args);
        return;
    }

    // Find the git repository
    let repo = match find_repository() {
        Ok(repo) => repo,
        Err(e) => {
            eprintln!("Failed to find repository: {}", e);
            std::process::exit(1);
        }
    };

    let default_user_name = get_commit_default_user_name(&repo, args);

    // Run pre-commit logic
    if let Err(e) = git::pre_commit::pre_commit(&repo, default_user_name.clone()) {
        eprintln!("Pre-commit failed: {}", e);
        std::process::exit(1);
    }

    // Proxy to git commit with interactive support
    let status_code = proxy_to_git_no_exit(&full_args);

    match status_code {
        0 => {
            if let Err(e) = git::post_commit::post_commit(&repo, false) {
                eprintln!("Post-commit failed: {}", e);
            }
        }
        _ => {
            std::process::exit(status_code);
        }
    }

    // let child = std::process::Command::new(config::Config::get().git_cmd())
    //     .args(&full_args)
    //     .spawn();

    // match child {
    //     Ok(mut child) => {
    //         // Wait for the process to complete
    //         let status = child.wait();
    //         match status {
    //             Ok(status) => {
    //                 let code = status.code().unwrap_or(1);
    //                 // If commit succeeded, run post-commit
    //                 if code == 0 {
    //                     if let Err(e) = git::post_commit::post_commit(&repo, false) {
    //                         eprintln!("Post-commit failed: {}", e);
    //                     }
    //                 }
    //                 std::process::exit(code);
    //             }
    //             Err(e) => {
    //                 eprintln!("Failed to wait for git commit process: {}", e);
    //                 std::process::exit(1);
    //             }
    //         }
    //     }
    //     Err(e) => {
    //         eprintln!("Failed to execute git commit: {}", e);
    //         std::process::exit(1);
    //     }
    // }
}

fn handle_fetch(args: &[String]) {
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

    // 1) Run exactly what the user typed (no arg mutation)
    let mut user_fetch = vec!["fetch".to_string()];
    user_fetch.extend_from_slice(args);
    let status = run_git_and_forward(&user_fetch, false);
    if !status.success() {
        exit_with_status(status);
    }

    // 2) Fetch authorship refs from the appropriate remote
    // Try to detect remote from args first
    let specified_remote = args
        .iter()
        .find(|a| remote_names.iter().any(|r| r == *a))
        .cloned();

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
        let mut fetch_authorship = vec!["fetch".to_string()];
        fetch_authorship.extend_from_slice(&[remote, AI_AUTHORSHIP_REFSPEC.to_string()]);
        // Silence the second fetch unless we're in debug mode
        let silent = !cfg!(debug_assertions);
        if !silent {
            debug_log(&format!(
                "fetching authorship refs: {:?}",
                &fetch_authorship
            ));
        }
        let auth_status = run_git_and_forward(&fetch_authorship, silent);
        exit_with_status(auth_status);
    } else {
        eprintln!("No git remotes found.");
        std::process::exit(1);
    }
}

fn handle_push(args: &[String]) {
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

    // 1) Run exactly what the user typed (no arg mutation)
    let mut user_push = vec!["push".to_string()];
    user_push.extend_from_slice(args);
    let status = run_git_and_forward(&user_push, false);
    if !status.success() {
        exit_with_status(status);
    }

    // 2) Push authorship refs to the appropriate remote
    let positional_remote = extract_remote_from_push_args(args, &remote_names);

    let specified_remote = positional_remote.or_else(|| {
        args.iter()
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
        let mut push_authorship = vec!["push".to_string()];
        push_authorship.extend_from_slice(&[remote, AI_AUTHORSHIP_REFSPEC.to_string()]);
        // Silence the second push unless we're in debug mode
        let silent = !cfg!(debug_assertions);
        if !silent {
            debug_log(&format!("pushing authorship refs: {:?}", &push_authorship));
        }
        let auth_status = run_git_and_forward(&push_authorship, silent);
        exit_with_status(auth_status);
    } else {
        eprintln!("No git remotes found.");
        std::process::exit(1);
    }
}

fn get_default_remote(repo: &git2::Repository) -> Option<String> {
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

fn proxy_to_git(args: &[String]) {
    _proxy_to_git(args, true);
}

fn proxy_to_git_no_exit(args: &[String]) -> i32 {
    return _proxy_to_git(args, false);
}

fn _proxy_to_git(args: &[String], exit_on_completion: bool) -> i32 {
    // Use spawn for interactive commands
    let child = Command::new(config::Config::get().git_cmd())
        .args(args)
        .spawn();

    match child {
        Ok(mut child) => {
            let status = child.wait();
            match status {
                Ok(status) => {
                    if exit_on_completion {
                        exit_with_status(status);
                    }
                    return status.code().unwrap_or(1);
                }
                Err(e) => {
                    eprintln!("Failed to wait for git process: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Err(e) => {
            eprintln!("Failed to execute git command: {}", e);
            std::process::exit(1);
        }
    }
}

// Ensure SIGPIPE default action, even if inherited ignored from a parent shell
fn reset_sigpipe_to_default() {
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

// Exit mirroring the child's termination: same signal if signaled, else exit code
fn exit_with_status(status: std::process::ExitStatus) -> ! {
    #[cfg(unix)]
    {
        if let Some(sig) = status.signal() {
            unsafe {
                libc::signal(sig, libc::SIG_DFL);
                libc::raise(sig);
            }
            // Should not return
            unreachable!();
        }
    }
    std::process::exit(status.code().unwrap_or(1));
}

#[allow(dead_code)]
fn parse_file_with_line_range(file_arg: &str) -> (String, Option<(u32, u32)>) {
    if let Some(colon_pos) = file_arg.rfind(':') {
        let file_path = file_arg[..colon_pos].to_string();
        let range_part = &file_arg[colon_pos + 1..];

        if let Some(dash_pos) = range_part.find('-') {
            // Range format: start-end
            let start_str = &range_part[..dash_pos];
            let end_str = &range_part[dash_pos + 1..];

            if let (Ok(start), Ok(end)) = (start_str.parse::<u32>(), end_str.parse::<u32>()) {
                return (file_path, Some((start, end)));
            }
        } else {
            // Single line format: line
            if let Ok(line) = range_part.parse::<u32>() {
                return (file_path, Some((line, line)));
            }
        }
    }
    (file_arg.to_string(), None)
}

fn print_help() {
    eprintln!("git-ai - git proxy with AI authorship tracking");
    eprintln!("");
    eprintln!("Usage: git-ai <git or git-ai command> [args...]");
    eprintln!("");
    eprintln!("Commands:");
    eprintln!("  checkpoint    [new] checkpoint working changes and specify author");
    eprintln!("    Presets: claude, cursor");
    eprintln!("    --author <name>       Override default author");
    eprintln!("    --model <model>       Override default model");
    eprintln!("    --prompt <json>       Override default prompt with JSON");
    eprintln!("    --prompt-path <path>  Override default prompt with file path");
    eprintln!("    --prompt-id <id>      Override default prompt with ID");
    eprintln!("    --show-working-log    Display current working log");
    eprintln!("    --reset               Reset working log");
    eprintln!("  blame         [override] git blame with AI authorship tracking");
    eprintln!(
        "  commit        [wrapper] pass through to 'git commit' with git-ai before/after hooks"
    );
    eprintln!("  fetch         [rewritten] Fetch from remote with AI authorship refs appended");
    eprintln!("  push          [rewritten] Push to remote with AI authorship refs appended");
    eprintln!("  install-hooks [new] Install git hooks for AI authorship tracking");
    eprintln!("  squash-authorship [new] Generate authorship from squashed commits");
    eprintln!("    <branch> <new_sha> <old_sha>  Required: branch, new commit SHA, old commit SHA");
    eprintln!("    --dry-run             Show what would be done without making changes");
    eprintln!("");
    std::process::exit(0);
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
