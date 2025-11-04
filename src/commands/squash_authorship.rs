use crate::authorship::rebase_authorship::rewrite_authorship_after_squash_or_rebase;
use crate::git::find_repository_in_path;

pub fn handle_squash_authorship(args: &[String]) {
    // Parse squash-authorship-specific arguments
    let mut base_branch = None;
    let mut new_sha = None;
    let mut old_sha = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--dry-run" => {
                // Dry-run flag is parsed but not used in current implementation
                i += 1;
            }
            _ => {
                // Positional arguments: base_branch, new_sha, old_sha
                if base_branch.is_none() {
                    base_branch = Some(args[i].clone());
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
    let base_branch = match base_branch {
        Some(s) => s,
        None => {
            eprintln!("Error: base_branch argument is required");
            eprintln!(
                "Usage: git-ai squash-authorship <base_branch> <new_sha> <old_sha> [--dry-run]"
            );
            std::process::exit(1);
        }
    };

    let new_sha = match new_sha {
        Some(s) => s,
        None => {
            eprintln!("Error: new_sha argument is required");
            eprintln!(
                "Usage: git-ai squash-authorship <base_branch> <new_sha> <old_sha> [--dry-run]"
            );
            std::process::exit(1);
        }
    };

    let old_sha = match old_sha {
        Some(s) => s,
        None => {
            eprintln!("Error: old_sha argument is required");
            eprintln!(
                "Usage: git-ai squash-authorship <base_branch> <new_sha> <old_sha> [--dry-run]"
            );
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

    // Use the same function as CI handlers to create authorship log for the new commit
    if let Err(e) = rewrite_authorship_after_squash_or_rebase(
        &repo,
        "",           // head_ref - not used by the function
        &base_branch, // merge_ref - the base branch name (e.g., "main")
        &old_sha,     // source_head_sha - the old commit
        &new_sha,     // merge_commit_sha - the new commit
        false,        // suppress_output
    ) {
        eprintln!("Squash authorship failed: {}", e);
        std::process::exit(1);
    }
}
