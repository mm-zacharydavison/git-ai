use crate::commands::checkpoint_agent::agent_preset::CursorPreset;
use crate::error::GitAiError;
use crate::git::refs::put_reference;
use crate::git::repo_storage::RepoStorage;
use crate::git::repository::Repository;
use crate::log_fmt::authorship_log_serialization::AuthorshipLog;
use crate::log_fmt::working_log::Checkpoint;
use crate::utils::debug_log;

// @todo Acunniffe - move this and simplify.
pub fn post_commit(repo: &Repository) -> Result<(String, AuthorshipLog), GitAiError> {
    // Get the current commit SHA (the commit that was just made)
    let head = repo.head()?;
    let commit_sha = match head.target() {
        Ok(oid) => oid.to_string(),
        Err(_) => {
            return Err(GitAiError::Generic(
                "No HEAD commit found. Cannot run post-commit hook.".to_string(),
            ));
        }
    };

    let current_commit = repo.find_commit(head.target().unwrap())?;

    // Get the parent commit (base commit this was made on top of)
    let parent_sha = match current_commit.parent(0) {
        Ok(parent) => parent.id().to_string(),
        Err(_) => "initial".to_string(), // No parent commit found, use "initial" like in checkpoint.rs
    };

    // Initialize the new storage system
    let repo_storage = &repo.storage;
    let working_log = repo_storage.working_log_for_base_commit(&parent_sha);

    // Pull all working log entries from the parent commit
    let parent_working_log = working_log.read_all_checkpoints()?;

    // Filter out untracked files from the working log
    let mut filtered_working_log = filter_untracked_files(repo, &parent_working_log)?;

    debug_log(&format!(
        "Working log entries: {}",
        filtered_working_log.len()
    ));

    // mutates inline
    CursorPreset::update_cursor_conversations_to_latest(&mut filtered_working_log)?;

    // Get git user information for human_author field
    let human_author = {
        let name = repo
            .config_get_str("user.name")
            .ok()
            .flatten()
            .unwrap_or_default();
        let email = repo
            .config_get_str("user.email")
            .ok()
            .flatten()
            .unwrap_or_default();
        let display_name = if name.trim().is_empty() {
            "unknown".to_string()
        } else {
            name
        };
        Some(if email.trim().is_empty() {
            display_name
        } else {
            format!("{} <{}>", display_name, email)
        })
    };

    // --- NEW: Serialize authorship log and store it in refs/ai/authorship/{commit_sha} ---
    let authorship_log = AuthorshipLog::from_working_log_with_base_commit_and_human_author(
        &filtered_working_log,
        &parent_sha,
        human_author.as_deref(),
    );

    // Serialize the authorship log
    let authorship_json = authorship_log
        .serialize_to_string()
        .map_err(|_| GitAiError::Generic("Failed to serialize authorship log".to_string()))?;

    let ref_name = format!("ai/authorship/{}", commit_sha);
    put_reference(
        repo,
        &ref_name,
        &authorship_json,
        &format!("AI authorship attestation for commit {}", commit_sha),
    )?;

    debug_log(&format!(
        "Authorship log written to refs/ai/authorship/{}",
        commit_sha
    ));

    repo_storage.delete_working_log_for_base_commit(&parent_sha)?;

    debug_log(&format!(
        "Working log deleted for base commit: {}",
        parent_sha
    ));

    Ok((ref_name, authorship_log))
}

/// Filter out working log entries for untracked files
fn filter_untracked_files(
    repo: &Repository,
    working_log: &[Checkpoint],
) -> Result<Vec<Checkpoint>, GitAiError> {
    // Get the current commit tree to see which files are currently tracked
    let head = repo.head()?;
    let current_commit = repo.find_commit(head.target().unwrap())?;
    let current_tree = current_commit.tree()?;

    // Get the parent commit tree to see which files were tracked before
    let parent_tree = if let Ok(parent) = current_commit.parent(0) {
        parent.tree()?
    } else {
        // No parent commit, so all files in current tree are new
        current_tree.clone()
    };

    // Filter the working log
    let mut filtered_checkpoints = Vec::new();

    for checkpoint in working_log {
        let mut filtered_entries = Vec::new();

        for entry in &checkpoint.entries {
            // Check if this file is currently tracked in the current commit
            let is_currently_tracked = current_tree
                .get_path(std::path::Path::new(&entry.file))
                .is_ok();

            // Check if this file was tracked in the parent commit
            let was_previously_tracked = parent_tree
                .get_path(std::path::Path::new(&entry.file))
                .is_ok();

            // Include the entry if:
            // 1. The file is currently tracked, OR
            // 2. The file is new (not in parent) but has working log entries
            if is_currently_tracked || !was_previously_tracked {
                filtered_entries.push(entry.clone());
            }
        }

        // Only include checkpoints that have at least one tracked file entry
        if !filtered_entries.is_empty() {
            let mut filtered_checkpoint = checkpoint.clone();
            filtered_checkpoint.entries = filtered_entries;
            filtered_checkpoints.push(filtered_checkpoint);
        }
    }

    Ok(filtered_checkpoints)
}
