mod commands;
mod error;
mod git;
mod log_fmt;
mod utils;

// aidan test
// next aidan line

use clap::Parser;
use git::find_repository;
use git::refs::AI_AUTHORSHIP_REFSPEC;
use std::io::{IsTerminal, Write};
use std::process::Command;
use utils::debug_log;

use crate::commands::checkpoint_agent::agent_preset::{
    AgentCheckpointFlags, AgentCheckpointPreset, ClaudePreset, CursorPreset,
};
use crate::git::refs::DEFAULT_REFSPEC;

#[derive(Parser)]
#[command(name = "git-ai")]
#[command(about = "git proxy with AI authorship tracking", long_about = None)]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(disable_help_flag = true)]
struct Cli {
    /// Git command and arguments
    #[arg(trailing_var_arg = true)]
    args: Vec<String>,
}

fn main() {
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
            // User called 'git' (via alias), show git help
            //
            //
            proxy_to_git(&["help".to_string()]);
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
        _ => {
            debug_log(&format!("proxying: git {}", command));
            // Proxy all other commands to git
            proxy_to_git(&cli.args);
        }
    }
}

fn handle_checkpoint(args: &[String]) {
    // Parse checkpoint-specific arguments
    let mut author = None;
    let mut show_working_log = false;
    let mut reset = false;
    let mut model = None;
    let mut _prompt_json = None;
    let mut prompt_path = None;
    let mut prompt_id = None;

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
                    prompt_path = Some(args[i + 1].clone());
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
                    transcript: None,
                    model: model.clone(),
                    prompt_id: prompt_id.clone(),
                    prompt_path: prompt_path.clone(),
                    workspace_id: None,
                }) {
                    Ok(agent_run) => {
                        agent_run_result = Some(agent_run);
                    }
                    Err(e) => {
                        eprintln!("Error running Claude preset: {}", e);
                        std::process::exit(1);
                    }
                }
            }
            "cursor" => {
                match CursorPreset.run(AgentCheckpointFlags {
                    transcript: None,
                    model: None,
                    prompt_id: None,
                    prompt_path: None,
                    workspace_id: None,
                }) {
                    Ok(agent_run) => {
                        agent_run_result = Some(agent_run);
                    }
                    Err(e) => {
                        eprintln!("Error running Claude preset: {}", e);
                        std::process::exit(1);
                    }
                }
            }
            _ => {}
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

fn handle_commit(args: &[String]) {
    // Find the git repository
    let repo = match find_repository() {
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

    // Run pre-commit logic
    if let Err(e) = git::pre_commit::pre_commit(&repo, default_user_name.clone()) {
        eprintln!("Pre-commit failed: {}", e);
        std::process::exit(1);
    }

    // Proxy to git commit with interactive support
    let mut full_args = vec!["commit".to_string()];
    full_args.extend_from_slice(args);

    let child = std::process::Command::new("git").args(&full_args).spawn();

    match child {
        Ok(mut child) => {
            // Wait for the process to complete
            let status = child.wait();
            match status {
                Ok(status) => {
                    let code = status.code().unwrap_or(1);
                    // If commit succeeded, run post-commit
                    if code == 0 {
                        if let Err(e) = git::post_commit::post_commit(&repo, false) {
                            eprintln!("Post-commit failed: {}", e);
                        }
                    }
                    std::process::exit(code);
                }
                Err(e) => {
                    eprintln!("Failed to wait for git commit process: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Err(e) => {
            eprintln!("Failed to execute git commit: {}", e);
            std::process::exit(1);
        }
    }
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
    if args.is_empty() {
        // git fetch (no remote): inject default remote and refspecs (heads + authorship)
        if let Some(default_remote) = get_default_remote(&repo) {
            // Use the default refspec but add --update-head-ok flag for fetch operations
            let refspec = DEFAULT_REFSPEC.to_string();

            let mut args_to_pass = vec!["fetch".to_string()];
            args_to_pass.push("--update-head-ok".to_string());
            args_to_pass.extend_from_slice(&[
                default_remote.clone(),
                AI_AUTHORSHIP_REFSPEC.to_string(),
                refspec,
            ]);
            proxy_to_git(&args_to_pass);
        } else {
            eprintln!("No git remotes found.");
            std::process::exit(1);
        }
        return;
    }
    if args.len() == 1 && remote_names.contains(&args[0]) {
        // git fetch <remote>: inject refspec after remote
        let mut args_to_pass = vec!["fetch".to_string()];
        args_to_pass.push("--update-head-ok".to_string());
        args_to_pass.extend_from_slice(&[args[0].clone(), AI_AUTHORSHIP_REFSPEC.to_string()]);
        proxy_to_git(&args_to_pass);
        return;
    }
    // More complex: just proxy as-is

    let mut full_args = vec!["fetch".to_string()];

    // println!("fetching or pulling from remote: {:?}", &full_args);
    full_args.extend_from_slice(args);
    proxy_to_git(&full_args);
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

    // Helper to run a git command and optionally forward output, returning exit code
    fn run_git_and_forward(args: &[String], quiet: bool) -> i32 {
        let output = Command::new("git").args(args).output();
        match output {
            Ok(output) => {
                if !quiet {
                    if !output.stdout.is_empty() {
                        std::io::stdout().write_all(&output.stdout).unwrap();
                    }
                    if !output.stderr.is_empty() {
                        std::io::stderr().write_all(&output.stderr).unwrap();
                    }
                }
                output.status.code().unwrap_or(1)
            }
            Err(e) => {
                eprintln!("Failed to execute git command: {}", e);
                1
            }
        }
    }

    // 1) Run exactly what the user typed (no arg mutation)
    let mut user_push = vec!["push".to_string()];
    user_push.extend_from_slice(args);
    let status_code = run_git_and_forward(&user_push, false);
    if status_code != 0 {
        std::process::exit(status_code);
    }

    // 2) Push authorship refs to the appropriate remote
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
        let mut push_authorship = vec!["push".to_string()];
        push_authorship.extend_from_slice(&[remote, AI_AUTHORSHIP_REFSPEC.to_string()]);
        // Silence the second push unless we're in debug mode
        let quiet_second_push = !cfg!(debug_assertions);
        if !quiet_second_push {
            debug_log(&format!("pushing authorship refs: {:?}", &push_authorship));
        }
        let auth_status = run_git_and_forward(&push_authorship, quiet_second_push);
        std::process::exit(auth_status);
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
    // Check if this is an interactive command that needs special handling
    // Including color-supporting commands to preserve TTY for proper color output
    let interactive_commands = [
        "add",
        "am",
        "apply",
        "bisect",
        "branch",
        "checkout",
        "cherry-pick",
        "clean",
        "clone",
        "commit",
        "config",
        "diff",
        "describe",
        "fetch",
        "help",
        "init",
        "interactive",
        "log",
        "ls-files",
        "ls-tree",
        "merge",
        "mv",
        "notes",
        "pull",
        "push",
        "rebase",
        "remote",
        "reflog",
        "reset",
        "restore",
        "revert",
        "rm",
        "shortlog",
        "show",
        "stash",
        "status",
        "submodule",
        "switch",
        "tag",
        "whatchanged",
        "worktree",
    ];
    let is_interactive = args
        .first()
        .map(|cmd| interactive_commands.contains(&cmd.as_str()))
        .unwrap_or(false);

    if is_interactive {
        // Use spawn for interactive commands
        let child = Command::new("git").args(args).spawn();

        match child {
            Ok(mut child) => {
                let status = child.wait();
                match status {
                    Ok(status) => {
                        std::process::exit(status.code().unwrap_or(1));
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
    } else {
        // Use output for non-interactive commands
        let output = Command::new("git").args(args).output();

        match output {
            Ok(output) => {
                // Forward stdout and stderr
                if !output.stdout.is_empty() {
                    std::io::stdout().write_all(&output.stdout).unwrap();
                }
                if !output.stderr.is_empty() {
                    std::io::stderr().write_all(&output.stderr).unwrap();
                }

                // Forward the exit code
                std::process::exit(output.status.code().unwrap_or(1));
            }
            Err(e) => {
                eprintln!("Failed to execute git command: {}", e);
                std::process::exit(1);
            }
        }
    }
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
    eprintln!("");
    std::process::exit(0);
}
