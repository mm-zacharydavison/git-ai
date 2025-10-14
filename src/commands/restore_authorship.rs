use crate::git::refs::get_commits_with_notes_from_list;
use crate::git::repository::{CommitRange, find_repository_in_path};

pub fn restore_authorship(args: &[String]) {
    // Validate arguments
    if args.len() < 2 {
        eprintln!("Error: restore_authorship requires two arguments");
        eprintln!("Usage: git-ai restore_authorship <refname> <commit-range>");
        eprintln!("Example: git-ai restore_authorship origin/main abc123..def456");
        std::process::exit(1);
    }

    let refname = &args[0];
    let commit_range = &args[1];

    // Parse and validate commit range format
    // Accepts both full (40 chars) and short (4+ chars) hashes
    let (before_commit, after_commit) = parse_commit_range(commit_range);

    let repository_working_dir = std::env::current_dir()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let repository = match find_repository_in_path(&repository_working_dir) {
        Ok(repo) => repo,
        Err(e) => {
            eprintln!("Failed to find repository: {}", e);
            std::process::exit(1);
        }
    };

    let commit_range = CommitRange::new(
        &repository,
        before_commit,
        after_commit,
        refname.to_string(),
    );

    // Validate commit range
    if let Err(e) = commit_range.is_valid() {
        eprintln!("Error: Invalid commit range: {}", e);
        std::process::exit(1);
    }

    // TODO: Implement restore authorship logic
    eprintln!("Restoring authorship for refname: {}", refname);
    eprintln!(
        "  Commit range: {}..{}",
        commit_range.start_oid, commit_range.end_oid
    );

    // First, collect all commit SHAs from the range
    let all_commits: Vec<_> = commit_range.into_iter().collect();
    let commit_shas: Vec<String> = all_commits
        .iter()
        .map(|commit| commit.id().to_string())
        .collect();

    eprintln!("Checking {} commits in range", commit_shas.len());

    // Batch-check which commits have notes using a single git invocation
    let commits_with_notes = match get_commits_with_notes_from_list(&repository, &commit_shas) {
        Ok(set) => set,
        Err(e) => {
            eprintln!("Failed to check commits for notes: {}", e);
            std::process::exit(1);
        }
    };

    // Filter to only commits without notes
    let commits_without_notes: Vec<_> = all_commits
        .into_iter()
        .filter(|commit| !commits_with_notes.contains(&commit.id().to_string()))
        .collect();

    eprintln!(
        "Found {} commits without authorship notes (out of {} total)",
        commits_without_notes.len(),
        commit_shas.len()
    );
}

fn parse_commit_range(commit_range: &str) -> (String, String) {
    // Split on ".."
    let parts: Vec<&str> = commit_range.split("..").collect();

    if parts.len() != 2 {
        eprintln!("Error: commit range must be in format <before>..<after>");
        eprintln!("Where before and after are 4-40 character hexadecimal commit hashes");
        eprintln!("Example: abc123..def456");
        std::process::exit(1);
    }

    let before = parts[0];
    let after = parts[1];

    // Validate that both parts are valid hex strings of appropriate length
    if !is_valid_commit_hash(before) {
        eprintln!("Error: '{}' is not a valid commit hash", before);
        eprintln!("Commit hashes must be 4-40 character hexadecimal strings");
        std::process::exit(1);
    }

    if !is_valid_commit_hash(after) {
        eprintln!("Error: '{}' is not a valid commit hash", after);
        eprintln!("Commit hashes must be 4-40 character hexadecimal strings");
        std::process::exit(1);
    }

    (before.to_string(), after.to_string())
}

fn is_valid_commit_hash(hash: &str) -> bool {
    let len = hash.len();

    // Must be between 4 and 40 characters
    if len < 4 || len > 40 {
        return false;
    }

    // Must be all hexadecimal characters
    hash.chars().all(|c| c.is_ascii_hexdigit())
}
