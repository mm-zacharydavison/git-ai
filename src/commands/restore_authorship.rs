use crate::authorship::authorship_log_serialization::AuthorshipLog;
use crate::error::GitAiError;
use crate::git::refs::get_commits_with_notes_from_list;
use crate::git::repository::{CommitRange, OwnedCommit, find_repository_in_path};

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

    // Process commits in parallel with concurrency limit
    let concurrency_limit = 10;

    // Convert to owned commits for async processing
    let owned_commits: Vec<OwnedCommit> = commits_without_notes
        .iter()
        .map(|c| c.to_owned_commit())
        .collect();

    smol::block_on(async {
        let semaphore = std::sync::Arc::new(smol::lock::Semaphore::new(concurrency_limit));
        let mut tasks = Vec::new();

        for commit in owned_commits {
            let sem = semaphore.clone();

            let task = smol::spawn(async move {
                let _permit = sem.acquire().await;

                let _ = reconstruct_authorship_for_commit(commit).await;
            });

            tasks.push(task);
        }

        // Wait for all tasks to complete
        for task in tasks {
            task.await;
        }
    });

    eprintln!("All commits processed!");
}

async fn reconstruct_authorship_for_commit(
    default_branch: String,
    commit: OwnedCommit,
) -> Result<(AuthorshipLog), GitAiError> {
    // Step 1 - Parse the commit summary and look for a branch name ie `feat/text` < maybe we need regex package now?
    // Ignore default_branch if we find it. Ie skip origin/main if we find it, and keep searching for feat/text or whatever they call it

    // Step 2 - If the branch name is found, fetch it into ai/authorship-fix/  git fetch origin feat/xyz:ai/authorship-fix/ --force

    // Step 3 - Check if the first commit in that refname is on the default_branch (main)
    // this basically checks if we have enoug history checked out locally to reconstruct.
    // If not fail with a graceful error

    // Step 4 - call rewrite_authorship_after_squash_or_rebase (internal library) with the commit ID (passed into this function) and the last commit in the branch lineage

    // always clean up! even if a step fails make sure we clear ai/authorship-fix/branch

    // Return Authorship Log Ok(authorship_log)
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
