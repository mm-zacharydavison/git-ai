use crate::commands::checkpoint_agent::agent_preset::AgentRunResult;
use crate::error::GitAiError;
use crate::git::repo_storage::{PersistedWorkingLog, RepoStorage};
use crate::git::status::{EntryKind, StatusCode, status_porcelainv2};
use crate::log_fmt::working_log::{Checkpoint, Line, WorkingLogEntry};
use crate::utils::debug_log;
use git2::Repository;
use sha2::{Digest, Sha256};
use similar::{ChangeTag, TextDiff};
use std::collections::HashMap;

pub fn run(
    repo: &Repository,
    author: &str,
    show_working_log: bool,
    reset: bool,
    quiet: bool,
    _model: Option<&str>,
    _human_author: Option<&str>,
    agent_run_result: Option<AgentRunResult>,
) -> Result<(usize, usize, usize), GitAiError> {
    // Robustly handle zero-commit repos
    let base_commit = match repo.head() {
        Ok(head) => {
            if let Some(oid) = head.target() {
                oid.to_string()
            } else {
                "initial".to_string()
            }
        }
        Err(_) => "initial".to_string(),
    };

    // Initialize the new storage system
    let repo_storage = RepoStorage::for_repo_path(repo.path());
    let working_log = repo_storage.working_log_for_base_commit(&base_commit);

    let files = get_all_tracked_files(repo, &base_commit, &working_log)?;
    let mut checkpoints = if reset {
        // If reset flag is set, start with an empty working log
        working_log.reset_working_log()?;
        Vec::new()
    } else {
        working_log.read_all_checkpoints()?
    };

    if show_working_log {
        if checkpoints.is_empty() {
            debug_log("No working log entries found.");
        } else {
            debug_log("Working Log Entries:");
            debug_log(&format!("{}", "=".repeat(80)));
            for (i, checkpoint) in checkpoints.iter().enumerate() {
                debug_log(&format!("Checkpoint {}: {}", i + 1, checkpoint.snapshot));
                debug_log(&format!("  Diff: {}", checkpoint.diff));
                debug_log(&format!("  Author: {}", checkpoint.author));
                debug_log(&format!(
                    "  Agent ID: {}",
                    checkpoint
                        .agent_id
                        .as_ref()
                        .map(|id| id.tool.clone())
                        .unwrap_or_default()
                ));

                // Display first user message from transcript if available
                if let Some(transcript) = &checkpoint.transcript {
                    if let Some(first_message) = transcript.messages().first() {
                        if let crate::log_fmt::transcript::Message::User { text, .. } =
                            first_message
                        {
                            let agent_info = checkpoint
                                .agent_id
                                .as_ref()
                                .map(|id| format!(" (Agent: {})", id.tool))
                                .unwrap_or_default();
                            let message_count = transcript.messages().len();
                            debug_log(&format!(
                                "  First message{} ({} messages): {}",
                                agent_info, message_count, text
                            ));
                        }
                    }
                }

                debug_log("  Entries:");
                for entry in &checkpoint.entries {
                    debug_log(&format!("    File: {}", entry.file));
                    debug_log(&format!("    Added lines: {:?}", entry.added_lines));
                    debug_log(&format!("    Deleted lines: {:?}", entry.deleted_lines));
                }
                debug_log("");
            }
        }
        return Ok((0, files.len(), checkpoints.len()));
    }

    let previous_commit = if reset {
        None
    } else {
        checkpoints.last().map(|c| c.snapshot.clone())
    };

    // Save current file states and get content hashes
    let file_content_hashes = save_current_file_states(&working_log, &files)?;

    // Order file hashes by key and create a hash of the ordered hashes
    let mut ordered_hashes: Vec<_> = file_content_hashes.iter().collect();
    ordered_hashes.sort_by_key(|(file_path, _)| *file_path);

    let mut combined_hasher = Sha256::new();
    for (file_path, hash) in ordered_hashes {
        combined_hasher.update(file_path.as_bytes());
        combined_hasher.update(hash.as_bytes());
    }
    let combined_hash = format!("{:x}", combined_hasher.finalize());

    // If this is not the first checkpoint, diff against the last saved state
    let entries = if checkpoints.is_empty() || reset {
        // First checkpoint or reset - diff against base commit
        get_initial_checkpoint_entries(repo, &files, &base_commit)?
    } else {
        // Subsequent checkpoint - diff against last saved state
        get_subsequent_checkpoint_entries(
            &working_log,
            &files,
            &file_content_hashes,
            previous_commit.as_deref(),
            &base_commit,
        )?
    };

    // Skip adding checkpoint if there are no changes
    if !entries.is_empty() {
        let mut checkpoint = Checkpoint::new(
            base_commit.clone(),
            combined_hash.clone(),
            author.to_string(),
            entries.clone(),
        );

        // Set transcript and agent_id if provided
        if let Some(agent_run) = &agent_run_result {
            checkpoint.transcript = Some(agent_run.transcript.clone().unwrap_or_default());
            checkpoint.agent_id = Some(agent_run.agent_id.clone());
        }

        // Append checkpoint to the working log with file content hashes
        working_log.append_checkpoint(&checkpoint, &file_content_hashes)?;
        checkpoints.push(checkpoint);
    }

    let agent_tool = if let Some(agent_run_result) = &agent_run_result {
        Some(agent_run_result.agent_id.tool.as_str())
    } else {
        None
    };

    // Print summary with new format
    if reset {
        debug_log("Working log reset. Starting fresh checkpoint.");
    }

    let label = if entries.len() > 1 {
        "checkpoint"
    } else {
        "commit"
    };

    if !quiet {
        let log_author = agent_tool.unwrap_or(author);
        eprintln!(
            "{}{} changed {} of the {} file(s) that have changed since the last {}",
            if agent_run_result.is_some() {
                "AI: "
            } else {
                "Human: "
            },
            log_author,
            entries.len(),
            files.len(),
            label
        );
    }

    // Return the requested values: (entries_len, files_len, working_log_len)
    Ok((entries.len(), files.len(), checkpoints.len()))
}

fn get_all_files(repo: &Repository) -> Result<Vec<String>, GitAiError> {
    let mut files = Vec::new();

    // Use porcelain v2 format to get status
    let statuses = status_porcelainv2(repo)?;

    for entry in statuses {
        // Skip ignored files
        if entry.kind == EntryKind::Ignored {
            continue;
        }

        // Include files that have any change (staged or unstaged) or are untracked
        let has_change = entry.staged != StatusCode::Unmodified
            || entry.unstaged != StatusCode::Unmodified
            || entry.kind == EntryKind::Untracked;

        if has_change {
            // For deleted files, check if they were text files in HEAD
            let is_deleted =
                entry.staged == StatusCode::Deleted || entry.unstaged == StatusCode::Deleted;

            let is_text = if is_deleted {
                is_text_file_in_head(repo, &entry.path)
            } else {
                is_text_file(repo, &entry.path)
            };

            if is_text {
                files.push(entry.path.clone());
            }
        }
    }

    Ok(files)
}

/// Get all files that should be tracked, including those from previous checkpoints
fn get_all_tracked_files(
    repo: &Repository,
    _base_commit: &str,
    working_log: &PersistedWorkingLog,
) -> Result<Vec<String>, GitAiError> {
    let mut files = get_all_files(repo)?;

    // Also include files that were in previous checkpoints but might not show up in git status
    // This ensures we track deletions when files return to their original state
    if let Ok(checkpoints) = working_log.read_all_checkpoints() {
        for checkpoint in &checkpoints {
            for entry in &checkpoint.entries {
                if !files.contains(&entry.file) {
                    // Check if it's a text file before adding
                    if is_text_file(repo, &entry.file) {
                        files.push(entry.file.clone());
                    }
                }
            }
        }
    }

    Ok(files)
}

fn save_current_file_states(
    working_log: &PersistedWorkingLog,
    files: &[String],
) -> Result<HashMap<String, String>, GitAiError> {
    let mut file_content_hashes = HashMap::new();

    for file_path in files {
        let abs_path = working_log.repo_root.join(file_path);
        let content = if abs_path.exists() {
            // Read file as bytes first, then convert to string with UTF-8 lossy conversion
            match std::fs::read(&abs_path) {
                Ok(bytes) => String::from_utf8_lossy(&bytes).to_string(),
                Err(_) => String::new(), // If we can't read the file, treat as empty
            }
        } else {
            String::new()
        };

        debug_log(&format!(
            "[SAVE] File: {}, Content length: {}, Content: {:?}",
            file_path,
            content.len(),
            content
        ));

        // Persist the file content and get the content hash
        let content_hash = working_log.persist_file_version(&content)?;
        debug_log(&format!("[SAVE] Generated hash: {}", content_hash));
        file_content_hashes.insert(file_path.clone(), content_hash);
    }

    Ok(file_content_hashes)
}

fn get_initial_checkpoint_entries(
    repo: &Repository,
    files: &[String],
    _base_commit: &str,
) -> Result<Vec<WorkingLogEntry>, GitAiError> {
    let mut entries = Vec::new();

    // Diff working directory against HEAD tree for each file
    let head_commit = repo
        .head()
        .ok()
        .and_then(|h| h.target())
        .and_then(|oid| repo.find_commit(oid).ok());
    let head_tree = head_commit.as_ref().and_then(|c| c.tree().ok());

    for file_path in files {
        let repo_workdir = repo.workdir().unwrap();
        let abs_path = repo_workdir.join(file_path);

        // Previous content from HEAD tree if present, otherwise empty
        let previous_content = if let Some(tree) = &head_tree {
            match tree.get_path(std::path::Path::new(file_path)) {
                Ok(entry) => {
                    if let Ok(blob) = repo.find_blob(entry.id()) {
                        String::from_utf8_lossy(blob.content()).to_string()
                    } else {
                        String::new()
                    }
                }
                Err(_) => String::new(),
            }
        } else {
            String::new()
        };

        // Current content from filesystem
        let current_content = std::fs::read_to_string(&abs_path).unwrap_or_else(|_| String::new());

        // Normalize trailing newlines to avoid spurious inserts
        let prev_norm = if previous_content.ends_with('\n') {
            previous_content.clone()
        } else {
            format!("{}\n", previous_content)
        };
        let curr_norm = if current_content.ends_with('\n') {
            current_content.clone()
        } else {
            format!("{}\n", current_content)
        };

        let diff = TextDiff::from_lines(&prev_norm, &curr_norm);
        let mut added_line_numbers = Vec::new();
        let mut deleted_line_numbers = Vec::new();
        let mut current_line = 1u32;

        let mut deletions_at_current_line = 0u32;

        for change in diff.iter_all_changes() {
            match change.tag() {
                ChangeTag::Equal => {
                    current_line += change.value().lines().count() as u32;
                    deletions_at_current_line = 0; // Reset deletion counter when we hit non-deleted content
                }
                ChangeTag::Delete => {
                    let delete_start = current_line + deletions_at_current_line;
                    let delete_count = change.value().lines().count() as u32;
                    // Collect individual line numbers for consolidation
                    for i in 0..delete_count {
                        deleted_line_numbers.push(delete_start + i);
                    }
                    deletions_at_current_line += delete_count;
                    // Don't advance current_line for deletions - insertions will happen at the same position
                }
                ChangeTag::Insert => {
                    let insert_start = current_line;
                    let insert_count = change.value().lines().count() as u32;
                    // Collect individual line numbers for consolidation
                    for i in 0..insert_count {
                        added_line_numbers.push(insert_start + i);
                    }
                    current_line += insert_count;
                    deletions_at_current_line = 0; // Reset deletion counter after insertions
                }
            }
        }

        // Consolidate consecutive lines into ranges
        let added_lines = consolidate_lines(added_line_numbers);
        let deleted_lines = consolidate_lines(deleted_line_numbers);

        if !added_lines.is_empty() || !deleted_lines.is_empty() {
            entries.push(WorkingLogEntry::new(
                file_path.clone(),
                added_lines,
                deleted_lines,
            ));
        }
    }

    Ok(entries)
}

fn get_subsequent_checkpoint_entries(
    working_log: &PersistedWorkingLog,
    files: &[String],
    _file_content_hashes: &HashMap<String, String>,
    previous_commit: Option<&str>,
    _base_commit: &str,
) -> Result<Vec<WorkingLogEntry>, GitAiError> {
    let mut entries = Vec::new();

    // Get the file version mapping from the previous checkpoint
    let previous_file_hashes = if let Some(prev_commit) = previous_commit {
        debug_log(&format!(
            "Looking up file version mapping for previous commit: {}",
            prev_commit
        ));
        let mapping = working_log.get_file_version_mapping(prev_commit)?;
        debug_log(&format!("Found {} file mappings", mapping.len()));
        mapping
    } else {
        debug_log("No previous commit, using empty mapping");
        HashMap::new()
    };

    for file_path in files {
        let abs_path = working_log.repo_root.join(file_path);

        // Read the previous content from the blob storage using the previous checkpoint's content hash
        let previous_content = if let Some(prev_content_hash) = previous_file_hashes.get(file_path)
        {
            debug_log(&format!(
                "Found previous content hash for {}: {}",
                file_path, prev_content_hash
            ));
            working_log
                .get_file_version(prev_content_hash)
                .unwrap_or_default()
        } else {
            debug_log(&format!("No previous content hash found for {}", file_path));
            String::new() // No previous version, treat as empty
        };

        // Read current content directly from the file system
        let current_content = std::fs::read_to_string(&abs_path).unwrap_or_else(|_| String::new());

        debug_log(&format!(
            "File: {}, Previous content length: {}, Current content length: {}",
            file_path,
            previous_content.len(),
            current_content.len()
        ));
        debug_log(&format!("File path: {:?}", abs_path));
        debug_log(&format!("File exists: {}", abs_path.exists()));
        debug_log(&format!("Previous content: {:?}", previous_content));
        debug_log(&format!("Current content: {:?}", current_content));

        // Normalize by ensuring trailing newline to avoid off-by-one when appending lines
        let prev_norm = if previous_content.ends_with('\n') {
            previous_content.clone()
        } else {
            format!("{}\n", previous_content)
        };
        let curr_norm = if current_content.ends_with('\n') {
            current_content.clone()
        } else {
            format!("{}\n", current_content)
        };

        let diff = TextDiff::from_lines(&prev_norm, &curr_norm);
        let mut added_line_numbers = Vec::new();
        let mut deleted_line_numbers = Vec::new();
        let mut current_line = 1u32;

        let mut deletions_at_current_line = 0u32;

        for change in diff.iter_all_changes() {
            match change.tag() {
                ChangeTag::Equal => {
                    current_line += change.value().lines().count() as u32;
                    deletions_at_current_line = 0; // Reset deletion counter when we hit non-deleted content
                }
                ChangeTag::Delete => {
                    let delete_start = current_line + deletions_at_current_line;
                    let delete_count = change.value().lines().count() as u32;
                    // Collect individual line numbers for consolidation
                    for i in 0..delete_count {
                        deleted_line_numbers.push(delete_start + i);
                    }
                    deletions_at_current_line += delete_count;
                    // Don't advance current_line for deletions - insertions will happen at the same position
                }
                ChangeTag::Insert => {
                    let insert_start = current_line;
                    let insert_count = change.value().lines().count() as u32;
                    // Collect individual line numbers for consolidation
                    for i in 0..insert_count {
                        added_line_numbers.push(insert_start + i);
                    }
                    current_line += insert_count;
                    deletions_at_current_line = 0; // Reset deletion counter after insertions
                }
            }
        }

        // Consolidate consecutive lines into ranges
        let added_lines = consolidate_lines(added_line_numbers);
        let deleted_lines = consolidate_lines(deleted_line_numbers);

        if !added_lines.is_empty() || !deleted_lines.is_empty() {
            entries.push(WorkingLogEntry::new(
                file_path.clone(),
                added_lines,
                deleted_lines,
            ));
        }
    }

    Ok(entries)
}

/// Consolidate consecutive line numbers into ranges for efficiency
fn consolidate_lines(mut lines: Vec<u32>) -> Vec<Line> {
    if lines.is_empty() {
        return Vec::new();
    }

    // Sort lines to ensure proper consolidation
    lines.sort_unstable();
    lines.dedup(); // Remove duplicates

    let mut consolidated = Vec::new();
    let mut start = lines[0];
    let mut end = lines[0];

    for &line in lines.iter().skip(1) {
        if line == end + 1 {
            // Consecutive line, extend the range
            end = line;
        } else {
            // Gap found, save the current range and start a new one
            if start == end {
                consolidated.push(Line::Single(start));
            } else {
                consolidated.push(Line::Range(start, end));
            }
            start = line;
            end = line;
        }
    }

    // Add the final range
    if start == end {
        consolidated.push(Line::Single(start));
    } else {
        consolidated.push(Line::Range(start, end));
    }

    consolidated
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log_fmt::working_log::Line;

    #[test]
    fn test_consolidate_lines() {
        // Test consecutive lines
        let lines = vec![1, 2, 3, 4];
        let result = consolidate_lines(lines);
        assert_eq!(result, vec![Line::Range(1, 4)]);

        // Test single line
        let lines = vec![5];
        let result = consolidate_lines(lines);
        assert_eq!(result, vec![Line::Single(5)]);

        // Test mixed consecutive and single
        let lines = vec![1, 2, 5, 6, 7, 10];
        let result = consolidate_lines(lines);
        assert_eq!(
            result,
            vec![Line::Range(1, 2), Line::Range(5, 7), Line::Single(10)]
        );

        // Test unsorted input
        let lines = vec![5, 1, 3, 2, 4];
        let result = consolidate_lines(lines);
        assert_eq!(result, vec![Line::Range(1, 5)]);

        // Test duplicates
        let lines = vec![1, 1, 2, 2, 3];
        let result = consolidate_lines(lines);
        assert_eq!(result, vec![Line::Range(1, 3)]);

        // Test empty input
        let lines = vec![];
        let result = consolidate_lines(lines);
        assert_eq!(result, vec![]);
    }
}

fn is_text_file(repo: &Repository, path: &str) -> bool {
    let repo_workdir = repo.workdir().unwrap();
    let abs_path = repo_workdir.join(path);

    if let Ok(metadata) = std::fs::metadata(&abs_path) {
        if !metadata.is_file() {
            return false;
        }
    } else {
        return false; // If metadata can't be read, treat as non-text
    }

    if let Ok(content) = std::fs::read(&abs_path) {
        // Consider a file text if it contains no null bytes
        !content.contains(&0)
    } else {
        false
    }
}

fn is_text_file_in_head(repo: &Repository, path: &str) -> bool {
    // For deleted files, check if they were text files in HEAD
    let head_commit = match repo
        .head()
        .ok()
        .and_then(|h| h.target())
        .and_then(|oid| repo.find_commit(oid).ok())
    {
        Some(commit) => commit,
        None => return false,
    };

    let head_tree = match head_commit.tree().ok() {
        Some(tree) => tree,
        None => return false,
    };

    match head_tree.get_path(std::path::Path::new(path)) {
        Ok(entry) => {
            if let Ok(blob) = repo.find_blob(entry.id()) {
                // Consider a file text if it contains no null bytes
                !blob.content().contains(&0)
            } else {
                false
            }
        }
        Err(_) => false,
    }
}
