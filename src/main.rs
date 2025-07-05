mod commands;
mod error;
mod git;
mod log_fmt;

use clap::{Parser, Subcommand};
use git::find_repository;

#[derive(Parser)]
#[command(name = "git-ai")]
#[command(about = "track AI authorship and prompts in git", long_about = None)]
#[command(version = env!("CARGO_PKG_VERSION"))]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// set up your local git repository for AI authorship tracking.
    Init,
    /// [tool use] create a checkpoint with the current working directory state
    Checkpoint {
        /// author of the checkpoint
        #[arg(long)]
        author: Option<String>,
        /// show log of working copy changes
        #[arg(long)]
        show_working_log: bool,
        /// rest working copy changes
        #[arg(long)]
        reset: bool,
        /// AI model + version
        #[arg(long)]
        model: Option<String>,
    },
    /// line-by-line ownership for a file
    Blame {
        /// file to blame (can include line range like "file.rs:10-20")
        file: String,
    },
    /// show authorship statistics for a commit
    Stats {
        /// commit SHA to analyze (defaults to HEAD)
        sha: Option<String>,
    },
    PreCommit,
    PostCommit {
        /// force execution even if working directory is not clean
        #[arg(long)]
        force: bool,
    },
}

fn main() {
    let cli = Cli::parse();

    // Find the git repository
    let repo = match find_repository() {
        Ok(repo) => repo,
        Err(e) => {
            eprintln!("Failed to find repository: {}", e);
            return;
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

    // Execute the command
    if let Err(e) = match &cli.command {
        Commands::Init => commands::init(&repo),
        Commands::Checkpoint {
            author,
            show_working_log,
            reset,
            model,
        } => {
            let final_author = author.as_ref().unwrap_or(&default_user_name);
            let result = commands::checkpoint(
                &repo,
                final_author,
                *show_working_log,
                *reset,
                false,
                model.as_deref(),
                Some(&default_user_name),
            );
            // Convert the tuple result to unit result to match other commands
            result.map(|_| ())
        }
        Commands::Blame { file } => {
            // Parse file argument for line range (e.g., "file.rs:10-20" or "file.rs:10")
            let (file_path, line_range) = parse_file_with_line_range(&file);
            // Convert the blame result to unit result to match other commands
            commands::blame(&repo, &file_path, line_range).map(|_| ())
        }
        Commands::Stats { sha } => {
            let sha = sha.as_deref().unwrap_or("HEAD");
            commands::stats(&repo, sha)
        }
        Commands::PreCommit => commands::pre_commit(&repo, default_user_name),
        Commands::PostCommit { force } => {
            commands::post_commit(&repo, *force).unwrap();
            Ok(())
        }
    } {
        eprintln!("Command failed: {}", e);
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
