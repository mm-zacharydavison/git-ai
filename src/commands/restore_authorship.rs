use crate::authorship::authorship_log_serialization::AuthorshipLog;
use crate::authorship::rebase_authorship::rewrite_authorship_after_squash_or_rebase;
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
            let default_branch = refname.clone();

            let task = smol::spawn(async move {
                let _permit = sem.acquire().await;

                match reconstruct_authorship_for_commit(default_branch, commit).await {
                    Ok(_) => {
                        eprintln!("✓ Successfully reconstructed authorship");
                    }
                    Err(e) => {
                        eprintln!("✗ Failed to reconstruct authorship: {}", e);
                    }
                }
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
) -> Result<AuthorshipLog, GitAiError> {
    let repository = commit.repo();
    let commit_id = commit.id();
    let temp_ref = format!("refs/heads/ai/authorship-fix/{}", commit_id);
    let repo_path = repository.path();

    // Cleanup function to ensure we always delete the temp ref
    let cleanup = || {
        let _ = std::process::Command::new(crate::config::Config::get().git_cmd())
            .arg("update-ref")
            .arg("-d")
            .arg(&temp_ref)
            .current_dir(&repo_path)
            .output();
    };

    // Step 1: Check ai-reflog for this merge commit
    let reflog_note = match get_ai_reflog_note(&repository, &commit_id) {
        Some(note) => note,
        None => {
            return Err(GitAiError::Generic(format!(
                "No ai-reflog entry found for merge commit: {}. Skipping reconstruction.",
                commit_id
            )));
        }
    };

    let (branch_name, branch_head) = match parse_reflog_note(&reflog_note) {
        Some((branch, head)) => (branch, head),
        None => {
            return Err(GitAiError::Generic(format!(
                "Invalid ai-reflog note format: {}",
                reflog_note
            )));
        }
    };

    eprintln!(
        "  Found ai-reflog entry: branch={}, head={}",
        branch_name, branch_head
    );

    // Step 2: Fetch the branch HEAD into temp ref
    let remote = extract_remote(&default_branch).unwrap_or("origin");
    let fetch_result = std::process::Command::new(crate::config::Config::get().git_cmd())
        .arg("fetch")
        .arg(remote)
        .arg(format!("{}:{}", branch_head, temp_ref))
        .arg("--force")
        .current_dir(&repo_path)
        .output();

    if let Err(e) = fetch_result {
        cleanup();
        return Err(GitAiError::Generic(format!(
            "Failed to fetch commit {}: {}",
            branch_head, e
        )));
    }

    let output = fetch_result.unwrap();
    if !output.status.success() {
        cleanup();
        return Err(GitAiError::Generic(format!(
            "Failed to fetch commit {}: {}",
            branch_head,
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    eprintln!("  Fetched branch HEAD {} into {}", branch_head, temp_ref);

    // Step 3: Verify the fetched commit is connected to default branch
    let merge_base_result = std::process::Command::new(crate::config::Config::get().git_cmd())
        .arg("merge-base")
        .arg(&default_branch)
        .arg(&temp_ref)
        .current_dir(&repo_path)
        .output();

    if let Err(e) = merge_base_result {
        cleanup();
        return Err(GitAiError::Generic(format!(
            "Failed to find merge base: {}",
            e
        )));
    }

    let merge_base_output = merge_base_result.unwrap();
    if !merge_base_output.status.success() {
        cleanup();
        return Err(GitAiError::Generic(
            "Branch history is not connected to default branch. Not enough history to reconstruct."
                .to_string(),
        ));
    }

    eprintln!("  Verified branch history is connected to default branch");

    eprintln!(
        "  Reconstructing authorship from {} to {}",
        branch_head, commit_id
    );

    // Step 4: Call rewrite_authorship_after_squash_or_rebase
    let result = rewrite_authorship_after_squash_or_rebase(
        repository,
        "", // destination_branch not used
        &branch_head,
        &commit_id,
        false, // not a dry run
    );

    // Always cleanup
    cleanup();

    match result {
        Ok(log) => {
            eprintln!(
                "  ✓ Successfully reconstructed authorship for {}",
                commit_id
            );
            Ok(log)
        }
        Err(e) => Err(GitAiError::Generic(format!(
            "Failed to reconstruct authorship: {}",
            e
        ))),
    }
}

/// Get ai-reflog note for a commit if it exists
fn get_ai_reflog_note(
    repo: &crate::git::repository::Repository,
    commit_sha: &str,
) -> Option<String> {
    let mut args = repo.global_args_for_exec();
    args.push("notes".to_string());
    args.push("--ref=ai-reflog".to_string());
    args.push("show".to_string());
    args.push(commit_sha.to_string());

    match crate::git::repository::exec_git(&args) {
        Ok(output) => String::from_utf8(output.stdout)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        Err(_) => None,
    }
}

/// Parse ai-reflog note format: "{branch_refname} {branch_head_commit}"
/// Example: "origin/main 8c1a000878cd22bc04a04822da423d87143f728a"
fn parse_reflog_note(note: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = note.split_whitespace().collect();
    if parts.len() >= 2 {
        Some((parts[0].to_string(), parts[1].to_string()))
    } else {
        None
    }
}

/// Extract remote name from refname (e.g., "origin" from "origin/main")
fn extract_remote(refname: &str) -> Option<&str> {
    refname.split('/').next()
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
