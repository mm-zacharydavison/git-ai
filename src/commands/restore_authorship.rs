use crate::authorship::authorship_log_serialization::AuthorshipLog;
use crate::authorship::rebase_authorship::rewrite_authorship_after_squash_or_rebase;
use crate::error::GitAiError;
use crate::git::refs::get_commits_with_notes_from_list;
use crate::git::repository::{CommitRange, OwnedCommit, find_repository_in_path};
use regex::Regex;

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

    // Step 1: Parse the commit summary and look for a branch name
    let summary = commit.summary()?;
    let branch_name = match extract_branch_name(&summary, &default_branch) {
        Some(name) => name,
        None => {
            return Err(GitAiError::Generic(format!(
                "No branch name found in commit message: {}",
                summary
            )));
        }
    };

    eprintln!("  Found branch name: {}", branch_name);

    // Step 2: Fetch the branch into ai/authorship-fix/
    let remote = extract_remote(&default_branch).unwrap_or("origin");
    let fetch_result = std::process::Command::new(crate::config::Config::get().git_cmd())
        .arg("fetch")
        .arg(remote)
        .arg(format!("{}:{}", branch_name, temp_ref))
        .arg("--force")
        .current_dir(&repo_path)
        .output();

    if let Err(e) = fetch_result {
        cleanup();
        return Err(GitAiError::Generic(format!(
            "Failed to fetch branch {}: {}",
            branch_name, e
        )));
    }

    let output = fetch_result.unwrap();
    if !output.status.success() {
        cleanup();
        return Err(GitAiError::Generic(format!(
            "Failed to fetch branch {}: {}",
            branch_name,
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    eprintln!("  Fetched branch into {}", temp_ref);

    // Step 3: Check if the first commit in that refname is on the default_branch
    // Get the first commit of the fetched branch
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

    let merge_base = String::from_utf8_lossy(&merge_base_output.stdout)
        .trim()
        .to_string();

    // Verify merge base exists and is on default branch
    let is_ancestor_result = std::process::Command::new(crate::config::Config::get().git_cmd())
        .arg("merge-base")
        .arg("--is-ancestor")
        .arg(&merge_base)
        .arg(&default_branch)
        .current_dir(&repo_path)
        .output();

    if let Err(e) = is_ancestor_result {
        cleanup();
        return Err(GitAiError::Generic(format!(
            "Failed to verify merge base: {}",
            e
        )));
    }

    if !is_ancestor_result.unwrap().status.success() {
        cleanup();
        return Err(GitAiError::Generic(
            "Merge base is not on default branch. Not enough history to reconstruct.".to_string(),
        ));
    }

    eprintln!("  Verified branch history is connected to default branch");

    // Step 4: Get the HEAD of the fetched branch
    let branch_head_result = std::process::Command::new(crate::config::Config::get().git_cmd())
        .arg("rev-parse")
        .arg(&temp_ref)
        .current_dir(&repo_path)
        .output();

    if let Err(e) = branch_head_result {
        cleanup();
        return Err(GitAiError::Generic(format!(
            "Failed to get branch head: {}",
            e
        )));
    }

    let branch_head = String::from_utf8_lossy(&branch_head_result.unwrap().stdout)
        .trim()
        .to_string();

    eprintln!(
        "  Reconstructing authorship from {} to {}",
        branch_head, commit_id
    );

    // Step 5: Call rewrite_authorship_after_squash_or_rebase
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

/// Extract branch name from commit message, excluding the default branch
/// Looks for common merge patterns like:
/// - "Merge pull request #123 from user/branch-name"
/// - "Merge branch 'feature/xyz' into main"
/// - "Merge branch 'branch-name'"
fn extract_branch_name(summary: &str, default_branch: &str) -> Option<String> {
    // Extract default branch name without remote prefix (e.g., "main" from "origin/main")
    let default_branch_name = default_branch.split('/').last().unwrap_or(default_branch);

    // Pattern 1: GitHub/GitLab style "from username/branch-name" or "from branch-name"
    // Matches: "Merge pull request #123 from user/branch-name"
    let from_pattern = Regex::new(r"from\s+(?:[a-zA-Z0-9_-]+/)?([a-zA-Z0-9_/-]+)").ok()?;
    if let Some(cap) = from_pattern.captures(summary) {
        if let Some(branch_match) = cap.get(1) {
            let branch = branch_match.as_str();
            if branch != default_branch_name && branch != default_branch {
                return Some(branch.to_string());
            }
        }
    }

    // Pattern 2: "Merge branch 'branch-name'" or "Merge branch \"branch-name\""
    // Matches: "Merge branch 'feat/xyz' into main"
    let branch_quoted_pattern =
        Regex::new(r#"[Mm]erge\s+branch\s+['"]([a-zA-Z0-9_/-]+)['"]"#).ok()?;
    if let Some(cap) = branch_quoted_pattern.captures(summary) {
        if let Some(branch_match) = cap.get(1) {
            let branch = branch_match.as_str();
            if branch != default_branch_name && branch != default_branch {
                return Some(branch.to_string());
            }
        }
    }

    // Pattern 3: "Merge branch branch-name" (without quotes)
    // Be more restrictive - require slash in branch name to avoid matching random words
    let branch_unquoted_pattern =
        Regex::new(r"[Mm]erge\s+branch\s+([a-zA-Z0-9_-]+/[a-zA-Z0-9_/-]+)").ok()?;
    if let Some(cap) = branch_unquoted_pattern.captures(summary) {
        if let Some(branch_match) = cap.get(1) {
            let branch = branch_match.as_str();
            if branch != default_branch_name && branch != default_branch {
                return Some(branch.to_string());
            }
        }
    }

    None
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
