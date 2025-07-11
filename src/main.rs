mod commands;
mod error;
mod git;
mod log_fmt;

use clap::Parser;
use git::find_repository;
use git::refs::AI_AUTHORSHIP_REFSPEC;
use std::io::Write;
use std::process::Command;

#[derive(Parser)]
#[command(name = "git-ai")]
#[command(about = "git proxy with AI authorship tracking", long_about = None)]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(disable_help_flag = true)]
#[command(disable_version_flag = true)]
struct Cli {
    /// Git command and arguments
    #[arg(trailing_var_arg = true)]
    args: Vec<String>,
}

fn debug_log(msg: &str) {
    if cfg!(debug_assertions) {
        eprintln!("\x1b[1;33m[git-ai]\x1b[0m {}", msg);
    }
}

fn main() {
    let cli = Cli::parse();

    if cli.args.is_empty() {
        // No arguments provided, proxy to git help
        proxy_to_git(&["help".to_string()]);
        return;
    }

    let command = &cli.args[0];
    let args = &cli.args[1..];

    match command.as_str() {
        "checkpoint" => {
            handle_checkpoint(args);
        }
        "blame" => {
            debug_log(&format!("overriding: git blame"));
            handle_blame(args);
        }
        "commit" => {
            debug_log(&format!("wrapping: git commit"));
            handle_commit(args);
        }
        "fetch" => {
            handle_fetch_or_pull("fetch", args);
        }
        "pull" => {
            handle_fetch_or_pull("pull", args);
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
    let mut quiet = false;

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
            "--quiet" => {
                quiet = true;
                i += 1;
            }
            _ => {
                eprintln!("Unknown checkpoint argument: {}", args[i]);
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

    if let Err(e) = commands::checkpoint(
        &repo,
        final_author,
        show_working_log,
        reset,
        quiet,
        model.as_deref(),
        Some(&default_user_name),
    ) {
        eprintln!("Checkpoint failed: {}", e);
        std::process::exit(1);
    }
}

fn handle_blame(args: &[String]) {
    if args.is_empty() {
        eprintln!("Error: blame requires a file argument");
        std::process::exit(1);
    }

    let file_arg = &args[0];

    // Find the git repository
    let repo = match find_repository() {
        Ok(repo) => repo,
        Err(e) => {
            eprintln!("Failed to find repository: {}", e);
            std::process::exit(1);
        }
    };

    // Parse file argument for line range (e.g., "file.rs:10-20" or "file.rs:10")
    let (file_path, line_range) = parse_file_with_line_range(file_arg);

    if let Err(e) = commands::blame(&repo, &file_path, line_range) {
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
    if let Err(e) = commands::pre_commit(&repo, default_user_name.clone()) {
        eprintln!("Pre-commit failed: {}", e);
        std::process::exit(1);
    }

    // Proxy to git commit
    let mut full_args = vec!["commit".to_string()];
    full_args.extend_from_slice(args);
    let output = std::process::Command::new("git").args(&full_args).output();

    match output {
        Ok(output) => {
            // Forward stdout and stderr
            if !output.stdout.is_empty() {
                std::io::stdout().write_all(&output.stdout).unwrap();
            }
            if !output.stderr.is_empty() {
                std::io::stderr().write_all(&output.stderr).unwrap();
            }
            let code = output.status.code().unwrap_or(1);
            // If commit succeeded, run post-commit
            if code == 0 {
                if let Err(e) = commands::post_commit(&repo, false) {
                    eprintln!("Post-commit failed: {}", e);
                }
            }
            std::process::exit(code);
        }
        Err(e) => {
            eprintln!("Failed to execute git commit: {}", e);
            std::process::exit(1);
        }
    }
}

fn handle_fetch_or_pull(cmd: &str, args: &[String]) {
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
        // git fetch or git pull (no remote): inject default remote and refspec
        if let Some(default_remote) = get_default_remote(&repo) {
            proxy_to_git(&[
                cmd.to_string(),
                default_remote,
                AI_AUTHORSHIP_REFSPEC.to_string(),
            ]);
        } else {
            eprintln!("No git remotes found.");
            std::process::exit(1);
        }
        return;
    }
    if args.len() == 1 && remote_names.contains(&args[0]) {
        // git fetch <remote> or git pull <remote>: inject refspec after remote
        proxy_to_git(&[
            cmd.to_string(),
            args[0].clone(),
            AI_AUTHORSHIP_REFSPEC.to_string(),
        ]);
        return;
    }
    // More complex: just proxy as-is
    let mut full_args = vec![cmd.to_string()];
    full_args.extend_from_slice(args);
    proxy_to_git(&full_args);
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
