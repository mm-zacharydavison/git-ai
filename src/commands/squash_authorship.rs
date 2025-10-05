use crate::authorship::rebase_authorship::rewrite_authorship_after_squash_or_rebase;
use crate::git::find_repository_in_path;

pub fn handle_squash_authorship(args: &[String]) {
    // Parse squash-authorship-specific arguments
    let mut dry_run = false;
    let mut branch = None;
    let mut new_sha = None;
    let mut old_sha = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--dry-run" => {
                dry_run = true;
                i += 1;
            }
            _ => {
                // Positional arguments: branch, new_sha, old_sha
                if branch.is_none() {
                    branch = Some(args[i].clone());
                } else if new_sha.is_none() {
                    new_sha = Some(args[i].clone());
                } else if old_sha.is_none() {
                    old_sha = Some(args[i].clone());
                } else {
                    eprintln!("Unknown squash-authorship argument: {}", args[i]);
                    std::process::exit(1);
                }
                i += 1;
            }
        }
    }

    // Validate required arguments
    let branch = match branch {
        Some(b) => b,
        None => {
            eprintln!("Error: branch argument is required");
            eprintln!("Usage: git-ai squash-authorship <branch> <new_sha> <old_sha> [--dry-run]");
            std::process::exit(1);
        }
    };

    let new_sha = match new_sha {
        Some(s) => s,
        None => {
            eprintln!("Error: new_sha argument is required");
            eprintln!("Usage: git-ai squash-authorship <branch> <new_sha> <old_sha> [--dry-run]");
            std::process::exit(1);
        }
    };

    let old_sha = match old_sha {
        Some(s) => s,
        None => {
            eprintln!("Error: old_sha argument is required");
            eprintln!("Usage: git-ai squash-authorship <branch> <new_sha> <old_sha> [--dry-run]");
            std::process::exit(1);
        }
    };

    // TODO Think about whether or not path should be an optional argument

    // Find the git repository
    let repo = match find_repository_in_path(".") {
        Ok(repo) => repo,
        Err(e) => {
            eprintln!("Failed to find repository: {}", e);
            std::process::exit(1);
        }
    };

    if let Err(e) =
        rewrite_authorship_after_squash_or_rebase(&repo, &branch, &old_sha, &new_sha, dry_run)
    {
        eprintln!("Squash authorship failed: {}", e);
        std::process::exit(1);
    }
}
