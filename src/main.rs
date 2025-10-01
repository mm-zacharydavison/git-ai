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
    AgentCheckpointFlags, AgentCheckpointPreset, AgentRunResult, ClaudePreset, CursorPreset,
};
use crate::git::find_repository_in_path;
use crate::log_fmt::working_log::AgentId;

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
    // If we're being invoked from a shell completion context, bypass git-ai logic
    // and delegate directly to the real git so existing completion scripts work.
    if in_shell_completion_context() {
        let orig_args: Vec<String> = std::env::args().skip(1).collect();
        proxy_to_git(&orig_args);
        return;
    }
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

    if binary_name == "git-ai" {
        commands::git_ai_handlers::handle_git_ai(&cli.args);
        std::process::exit(0);
    }

    // debug_log(&format!("in main, command: {}", command));
    // debug_log(&format!("in main, args: {:?}", positional_args));

    commands::git_handlers::handle_git(&cli.args);
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
    // Try to detect remote (named remote, URL, or local path) from args first
    let positional_remote = extract_remote_from_fetch_args(args);
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
        // Forward relevant fetch flags so semantics match the primary fetch
        let forwarded_flags = extract_forwarded_fetch_flags(args);

        let mut fetch_authorship = vec!["fetch".to_string()];
        // Place options before positional args per git's CLI conventions
        fetch_authorship.extend(forwarded_flags);
        // Unless explicitly requested otherwise, do not fetch tags on the
        // secondary authorship fetch to avoid creating unexpected tag refs
        let user_specified_tags_pref = args.iter().any(|a| a == "--tags" || a == "--no-tags");
        if !user_specified_tags_pref {
            fetch_authorship.push("--no-tags".to_string());
        }
        // Do not clobber FETCH_HEAD from the user's fetch (see git t5515 expectations)
        fetch_authorship.push("--no-write-fetch-head".to_string());
        fetch_authorship.extend_from_slice(&[remote, AI_AUTHORSHIP_REFSPEC.to_string()]);
        // Always silence the secondary fetch to avoid interfering with caller output/trace
        let silent = true;
        if cfg!(debug_assertions) {
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

    // If this was a dry-run or delete, do not perform any secondary pushes
    let is_dry_run = args.iter().any(|a| a == "--dry-run" || a == "-n");
    let is_delete = args.iter().any(|a| a == "-d" || a == "--delete");
    if is_dry_run || is_delete {
        return;
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
        // Skip secondary push for mirrored pushes/remotes to avoid combining --mirror with refspecs
        let has_mirror_flag = args
            .iter()
            .any(|a| a == "--mirror" || a.starts_with("--mirror="));
        if has_mirror_flag || remote_is_mirror(&repo, &remote) {
            return;
        }

        // Forward relevant flags so the secondary push has matching semantics
        let forwarded_flags = extract_forwarded_push_flags(args);

        let mut push_authorship = vec!["push".to_string()];
        // Place options before positional args per git's CLI conventions
        // Always bypass hooks for internal authorship push to avoid interfering with user's hooks
        push_authorship.push("--no-verify".to_string());
        push_authorship.extend(forwarded_flags);
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

fn remote_is_mirror(repo: &git2::Repository, remote: &str) -> bool {
    if let Ok(cfg) = repo.config() {
        let key = format!("remote.{}.mirror", remote);
        if let Ok(val) = cfg.get_string(&key) {
            let v = val.to_lowercase();
            if v == "true" || v == "push" || v == "yes" || v == "on" || v == "1" {
                return true;
            }
        }
        if let Ok(b) = cfg.get_bool(&key) {
            if b {
                return true;
            }
        }
    }
    false
}

fn proxy_to_git(args: &[String]) {
    _proxy_to_git(args, true);
}

fn proxy_to_git_no_exit(args: &[String]) -> i32 {
    return _proxy_to_git(args, false);
}

fn _proxy_to_git(args: &[String], exit_on_completion: bool) -> i32 {
    // debug_log(&format!("proxying to git with args: {:?}", args));
    // debug_log(&format!("prepended global args: {:?}", prepend_global(args)));
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

// Detect if current process invocation is coming from shell completion machinery
// (bash, zsh via bashcompinit). If so, we should proxy directly to the real git
// without any extra behavior that could interfere with completion scripts.
fn in_shell_completion_context() -> bool {
    std::env::var("COMP_LINE").is_ok()
        || std::env::var("COMP_POINT").is_ok()
        || std::env::var("COMP_TYPE").is_ok()
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

fn extract_forwarded_push_flags(args: &[String]) -> Vec<String> {
    let mut forwarded: Vec<String> = Vec::new();

    // Helpers
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];

        // --push-option/-o are handled via extract_push_options to preserve values
        if let Some(val) = arg.strip_prefix("--push-option=") {
            forwarded.push(format!("--push-option={}", val));
            i += 1;
            continue;
        }
        if arg == "--push-option" || arg == "-o" {
            if i + 1 < args.len() {
                forwarded.push(format!("--push-option={}", args[i + 1]));
            }
            i += 2;
            continue;
        }

        // --signed, --no-signed, --signed=<mode>
        if arg == "--signed" || arg == "--no-signed" || arg.starts_with("--signed=") {
            forwarded.push(arg.clone());
            i += 1;
            continue;
        }

        // --atomic / --no-atomic
        if arg == "--atomic" || arg == "--no-atomic" {
            forwarded.push(arg.clone());
            i += 1;
            continue;
        }

        // --receive-pack / --exec (with or without =)
        if arg.starts_with("--receive-pack=") || arg.starts_with("--exec=") {
            forwarded.push(arg.clone());
            i += 1;
            continue;
        }
        if arg == "--receive-pack" || arg == "--exec" {
            if i + 1 < args.len() {
                forwarded.push(arg.clone());
                forwarded.push(args[i + 1].clone());
            } else {
                forwarded.push(arg.clone());
            }
            i += 2;
            continue;
        }

        // --force-with-lease variants and --no-force-with-lease
        if arg == "--force-with-lease"
            || arg == "--no-force-with-lease"
            || arg.starts_with("--force-with-lease=")
        {
            forwarded.push(arg.clone());
            i += 1;
            continue;
        }

        // --force-if-includes / --no-force-if-includes
        if arg == "--force-if-includes" || arg == "--no-force-if-includes" {
            forwarded.push(arg.clone());
            i += 1;
            continue;
        }

        // -f / --force
        if arg == "-f" || arg == "--force" {
            forwarded.push(arg.clone());
            i += 1;
            continue;
        }

        // --thin / --no-thin
        if arg == "--thin" || arg == "--no-thin" {
            forwarded.push(arg.clone());
            i += 1;
            continue;
        }

        // --recurse-submodules forms
        if arg == "--no-recurse-submodules" || arg.starts_with("--recurse-submodules=") {
            forwarded.push(arg.clone());
            i += 1;
            continue;
        }

        // Do not forward --verify / --no-verify. Our internal push always uses --no-verify.
        if arg == "--verify" || arg == "--no-verify" {
            i += 1;
            continue;
        }

        // -4 / --ipv4, -6 / --ipv6
        if arg == "-4" || arg == "--ipv4" || arg == "-6" || arg == "--ipv6" {
            forwarded.push(arg.clone());
            i += 1;
            continue;
        }

        // --repo should not be forwarded (we already compute the target remote)

        i += 1;
    }

    forwarded
}

fn extract_forwarded_fetch_flags(args: &[String]) -> Vec<String> {
    let mut forwarded: Vec<String> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];

        // Do not forward remote selection/grouping; we compute our own remote
        if arg == "--all" || arg == "--multiple" {
            i += 1;
            continue;
        }

        // Forward FETCH_HEAD write behavior
        if arg == "--no-write-fetch-head" {
            forwarded.push(arg.clone());
            i += 1;
            continue;
        }

        // Forward tags behavior
        if arg == "--tags" || arg == "--no-tags" {
            forwarded.push(arg.clone());
            i += 1;
            continue;
        }

        // Forward dry-run
        if arg == "-n" || arg == "--dry-run" {
            forwarded.push(arg.clone());
            i += 1;
            continue;
        }

        // Forward IP version preferences
        if arg == "-4" || arg == "--ipv4" || arg == "-6" || arg == "--ipv6" {
            forwarded.push(arg.clone());
            i += 1;
            continue;
        }

        i += 1;
    }

    forwarded
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
