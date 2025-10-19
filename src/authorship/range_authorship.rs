use crate::authorship::authorship_log_serialization::AuthorshipLog;
use crate::authorship::rebase_authorship::rewrite_authorship_after_squash_or_rebase;
use crate::error::GitAiError;
use crate::git::refs::get_commits_with_notes_from_list;
use crate::git::repository::{CommitRange, OwnedCommit};

pub fn range_authorship(commit_range: CommitRange) -> Result<(), GitAiError> {
    eprintln!("Restoring authorship for refname: {}", commit_range.refname);
    eprintln!(
        "  Commit range: {}..{}",
        commit_range.start_oid, commit_range.end_oid
    );

    commit_range.is_valid()?;

    // First, collect all commit SHAs from the range
    let repository = commit_range.repo();
    let refname = commit_range.refname.clone();
    let all_commits: Vec<_> = commit_range.into_iter().collect();
    let commit_shas: Vec<String> = all_commits
        .iter()
        .map(|commit| commit.id().to_string())
        .collect();

    eprintln!("Checking {} commits in range", commit_shas.len());

    // Batch-check which commits have notes using a single git invocation
    let commits_with_notes = get_commits_with_notes_from_list(repository, &commit_shas)?;

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
            let refname_clone = refname.clone();

            let task = smol::spawn(async move {
                let _permit = sem.acquire().await;

                match reconstruct_authorship_for_commit(&refname_clone, commit).await {
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
    Ok(())
}

async fn reconstruct_authorship_for_commit(
    default_branch: &str,
    commit: OwnedCommit,
) -> Result<AuthorshipLog, GitAiError> {
    let repository = commit.repo();
    let commit_id = commit.id();
    let repo_path = repository.path();

    // Step 1: Check ai-reflog for this merge commit
    let reflog_note = get_ai_reflog_note(&repository, &commit_id).ok_or_else(|| {
        GitAiError::Generic(format!(
            "No ai-reflog entry found for merge commit: {}. Skipping reconstruction.",
            commit_id
        ))
    })?;

    let (branch_name, branch_head) = parse_reflog_note(&reflog_note).ok_or_else(|| {
        GitAiError::Generic(format!("Invalid ai-reflog note format: {}", reflog_note))
    })?;

    eprintln!(
        "  Found ai-reflog entry: branch={}, head={}",
        branch_name, branch_head
    );

    // Step 2: Fetch the branch HEAD directly (server-side, no temp ref needed)
    let remote = extract_remote(default_branch).unwrap_or("origin");
    let fetch_result = std::process::Command::new(crate::config::Config::get().git_cmd())
        .arg("fetch")
        .arg(remote)
        .arg(&branch_head)
        .current_dir(&repo_path)
        .output();

    match fetch_result {
        Ok(output) if output.status.success() => {
            eprintln!("  Fetched branch HEAD {}", branch_head);
        }
        Ok(output) => {
            return Err(GitAiError::Generic(format!(
                "Failed to fetch commit {}: {}",
                branch_head,
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        Err(e) => {
            return Err(GitAiError::Generic(format!(
                "Failed to fetch commit {}: {}",
                branch_head, e
            )));
        }
    }

    // Step 3: Verify the fetched commit is connected to default branch
    let merge_base_result = std::process::Command::new(crate::config::Config::get().git_cmd())
        .arg("merge-base")
        .arg(default_branch)
        .arg(&branch_head)
        .current_dir(&repo_path)
        .output();

    match merge_base_result {
        Ok(output) if output.status.success() => {
            eprintln!("  Verified branch history is connected to default branch");
        }
        _ => {
            return Err(GitAiError::Generic(
                "Branch history is not connected to default branch. Not enough history to reconstruct."
                    .to_string(),
            ));
        }
    }

    eprintln!(
        "  Reconstructing authorship from {} to {}",
        branch_head, commit_id
    );

    // Step 4: Call rewrite_authorship_after_squash_or_rebase
    let log = rewrite_authorship_after_squash_or_rebase(
        repository,
        "", // destination_branch not used
        &branch_head,
        &commit_id,
        false, // not a dry run
    )?;

    eprintln!(
        "  ✓ Successfully reconstructed authorship for {}",
        commit_id
    );
    Ok(log)
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
