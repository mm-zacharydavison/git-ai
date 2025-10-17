// call pattern git-ai log-pr-closed <remote> <branch refname> <merge-commit> <branch HEAD commit>

use crate::error::GitAiError;
use crate::git::repository::exec_git;
use crate::utils::debug_log;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;

const AI_REFLOG_REFNAME: &str = "ai-reflog";
const AI_REFLOG_REF: &str = "refs/notes/ai-reflog";

pub fn log_pr_closed(args: &[String]) -> Result<(), GitAiError> {
    // Parse arguments: <remote> <branch refname> <merge-commit> <branch HEAD commit>
    if args.len() < 4 {
        return Err(GitAiError::Generic(
            "Expected 4 arguments: <remote> <branch refname> <merge-commit> <branch HEAD commit>"
                .to_string(),
        ));
    }

    let remote_url = &args[0];
    let branch_refname = &args[1];
    let merge_commit = &args[2];
    let branch_head_commit = &args[3];

    debug_log(&format!(
        "log_pr_closed: remote={}, branch={}, merge={}, head={}",
        remote_url, branch_refname, merge_commit, branch_head_commit
    ));

    // Step 1: Ensure tmp/git-ai-log-pr exists
    let tmp_dir = std::env::temp_dir().join("git-ai-log-pr");
    std::fs::create_dir_all(&tmp_dir)?;

    // Step 2: Create a stable directory name by hashing the remote URL
    let repo_dir_name = hash_remote_url(remote_url);
    let bare_repo_path = tmp_dir.join(repo_dir_name);

    // Step 2a: If it doesn't exist, bare clone it (we only need notes)
    if !bare_repo_path.exists() {
        debug_log(&format!(
            "Creating bare clone at {}",
            bare_repo_path.display()
        ));
        clone_bare_for_notes(remote_url, &bare_repo_path)?;
    }

    // Step 3: Fetch latest notes/ai-reflog into a tracking ref
    let tracking_ref = format!("refs/notes/{}-tracking", AI_REFLOG_REFNAME);
    fetch_notes_reflog(&bare_repo_path, remote_url, &tracking_ref)?;

    // Step 4: Add a new note with format "{refname} {branch HEAD commit}"
    // attached to the merge commit
    let note_content = format!("{} {}", branch_refname, branch_head_commit);
    add_note_to_commit(&bare_repo_path, merge_commit, &note_content)?;

    // Step 5: Merge the tracking ref into local notes/ai-reflog (if it exists)
    if ref_exists_in_repo(&bare_repo_path, &tracking_ref) {
        if ref_exists_in_repo(&bare_repo_path, AI_REFLOG_REF) {
            // Both exist - merge them
            debug_log(&format!("Merging {} into {}", tracking_ref, AI_REFLOG_REF));
            merge_notes_reflog(&bare_repo_path, &tracking_ref)?;
        } else {
            // Only tracking ref exists - copy it to local
            debug_log(&format!(
                "Initializing {} from {}",
                AI_REFLOG_REF, tracking_ref
            ));
            copy_ref_in_repo(&bare_repo_path, &tracking_ref, AI_REFLOG_REF)?;
        }
    }

    // Step 6: Push the notes back
    push_notes_reflog(&bare_repo_path, remote_url)?;

    debug_log("log_pr_closed completed successfully");
    Ok(())
}

/// Hash the remote URL to create a stable directory name
fn hash_remote_url(url: &str) -> String {
    let mut hasher = DefaultHasher::new();
    url.hash(&mut hasher);
    let hash = hasher.finish();
    format!("repo-{:x}", hash)
}

/// Create a bare clone of the remote repository (only fetching notes)
fn clone_bare_for_notes(remote_url: &str, target_path: &Path) -> Result<(), GitAiError> {
    debug_log(&format!(
        "Cloning bare repository from {} to {}",
        remote_url,
        target_path.display()
    ));

    // Create a bare repository
    let args = vec![
        "clone".to_string(),
        "--bare".to_string(),
        "--no-tags".to_string(),
        "--filter=blob:none".to_string(), // Treeless clone - we only need notes
        remote_url.to_string(),
        target_path.to_string_lossy().to_string(),
    ];

    exec_git(&args)?;
    Ok(())
}

/// Fetch notes/ai-reflog from the remote into a tracking ref
pub fn fetch_notes_reflog(
    repo_path: &Path,
    remote_url: &str,
    tracking_ref: &str,
) -> Result<(), GitAiError> {
    let fetch_refspec = format!("+{}:{}", AI_REFLOG_REF, tracking_ref);

    let args = vec![
        "-C".to_string(),
        repo_path.to_string_lossy().to_string(),
        "-c".to_string(),
        "core.hooksPath=/dev/null".to_string(),
        "fetch".to_string(),
        "--no-tags".to_string(),
        "--no-write-fetch-head".to_string(),
        "--no-write-commit-graph".to_string(),
        "--no-auto-maintenance".to_string(),
        remote_url.to_string(),
        fetch_refspec,
    ];

    debug_log(&format!("Fetching notes/ai-reflog: {:?}", &args));

    // Best-effort fetch; might fail if notes don't exist yet on remote
    if let Err(e) = exec_git(&args) {
        debug_log(&format!("Notes fetch failed (might not exist yet): {}", e));
    }

    Ok(())
}

/// Add a note to a specific commit
pub fn add_note_to_commit(
    repo_path: &Path,
    commit_sha: &str,
    note_content: &str,
) -> Result<(), GitAiError> {
    use crate::git::repository::exec_git_stdin;

    let args = vec![
        "-C".to_string(),
        repo_path.to_string_lossy().to_string(),
        "notes".to_string(),
        format!("--ref={}", AI_REFLOG_REFNAME),
        "add".to_string(),
        "-f".to_string(), // Force overwrite if exists
        "-F".to_string(),
        "-".to_string(), // Read from stdin
        commit_sha.to_string(),
    ];

    debug_log(&format!(
        "Adding note to commit {}: {}",
        commit_sha, note_content
    ));

    exec_git_stdin(&args, note_content.as_bytes())?;
    Ok(())
}

/// Check if a ref exists in the repository
fn ref_exists_in_repo(repo_path: &Path, ref_name: &str) -> bool {
    let args = vec![
        "-C".to_string(),
        repo_path.to_string_lossy().to_string(),
        "show-ref".to_string(),
        "--verify".to_string(),
        "--quiet".to_string(),
        ref_name.to_string(),
    ];

    exec_git(&args).is_ok()
}

/// Merge notes from a source ref into refs/notes/ai-reflog
fn merge_notes_reflog(repo_path: &Path, source_ref: &str) -> Result<(), GitAiError> {
    let args = vec![
        "-C".to_string(),
        repo_path.to_string_lossy().to_string(),
        "notes".to_string(),
        format!("--ref={}", AI_REFLOG_REFNAME),
        "merge".to_string(),
        "-s".to_string(),
        "ours".to_string(),
        "--quiet".to_string(),
        source_ref.to_string(),
    ];

    debug_log(&format!(
        "Merging notes from {} into {}",
        source_ref, AI_REFLOG_REF
    ));

    exec_git(&args)?;
    Ok(())
}

/// Copy a ref to another location
fn copy_ref_in_repo(repo_path: &Path, source_ref: &str, dest_ref: &str) -> Result<(), GitAiError> {
    let args = vec![
        "-C".to_string(),
        repo_path.to_string_lossy().to_string(),
        "update-ref".to_string(),
        dest_ref.to_string(),
        source_ref.to_string(),
    ];

    debug_log(&format!("Copying ref {} to {}", source_ref, dest_ref));

    exec_git(&args)?;
    Ok(())
}

/// Push notes/ai-reflog back to the remote
fn push_notes_reflog(repo_path: &Path, remote_url: &str) -> Result<(), GitAiError> {
    // Push without force (requires fast-forward). Creating new refs works without force.
    // We've already merged remote notes before pushing, so we should always be ahead.
    let push_refspec = format!("{}:{}", AI_REFLOG_REF, AI_REFLOG_REF);

    let args = vec![
        "-C".to_string(),
        repo_path.to_string_lossy().to_string(),
        "-c".to_string(),
        "core.hooksPath=/dev/null".to_string(),
        "push".to_string(),
        "--quiet".to_string(),
        "--no-verify".to_string(),
        remote_url.to_string(),
        push_refspec,
    ];

    debug_log(&format!("Pushing notes/ai-reflog: {:?}", &args));

    exec_git(&args).map_err(|e| {
        debug_log(&format!("Notes push failed: {}", e));
        e
    })?;

    Ok(())
}
