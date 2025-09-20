use crate::commands::checkpoint_agent::agent_preset::AgentRunResult;
use crate::error::GitAiError;
use crate::git::refs::{get_reference, put_reference};
use crate::log_fmt::working_log::{Checkpoint, Line, WorkingLogEntry};
use crate::utils::debug_log;
use git2::{Repository, StatusOptions};
use sha2::{Digest, Sha256};
use similar::{ChangeTag, TextDiff};
use std::collections::HashMap;
use std::path::Path;

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

    // aidan
    let files = get_all_files(repo)?;
    let mut working_log = if reset {
        // If reset flag is set, start with an empty working log
        Vec::new()
    } else {
        get_or_create_working_log(repo, &base_commit)?
    };

    // Clear ai-working-log/diffs references when reset is true
    if reset {
        clear_working_log_diffs(repo, &base_commit)?;
    }

    if show_working_log {
        if working_log.is_empty() {
            debug_log("No working log entries found.");
        } else {
            debug_log("Working Log Entries:");
            debug_log(&format!("{}", "=".repeat(80)));
            for (i, checkpoint) in working_log.iter().enumerate() {
                debug_log(&format!("Checkpoint {}: {}", i + 1, checkpoint.snapshot));
                debug_log(&format!("  Diff: {}", checkpoint.diff));
                debug_log(&format!("  Author: {}", checkpoint.author));
                debug_log("  Entries:");
                for entry in &checkpoint.entries {
                    debug_log(&format!("    File: {}", entry.file));
                    debug_log(&format!("    Added lines: {:?}", entry.added_lines));
                    debug_log(&format!("    Deleted lines: {:?}", entry.deleted_lines));
                }
                debug_log("");
            }
        }
        return Ok((0, files.len(), working_log.len()));
    }

    let previous_commit = if reset {
        None
    } else {
        working_log.last().map(|c| c.snapshot.clone())
    };

    let file_hashes: std::collections::HashMap<String, String> = files
        .iter()
        .map(|file_path| {
            let mut hasher = sha2::Sha256::new();
            hasher.update(file_path.as_bytes());
            let file_hash = format!("{:x}", hasher.finalize());
            (file_path.clone(), file_hash)
        })
        .collect();

    // Order file hashes by key and create a hash of the ordered hashes
    let mut ordered_hashes: Vec<_> = file_hashes.iter().collect();
    ordered_hashes.sort_by_key(|(file_path, _)| *file_path);

    let mut combined_hasher = Sha256::new();
    for (file_path, hash) in ordered_hashes {
        combined_hasher.update(file_path.as_bytes());
        combined_hasher.update(hash.as_bytes());
    }
    let combined_hash = format!("{:x}", combined_hasher.finalize());

    // If this is not the first checkpoint, diff against the last saved state
    let entries = if working_log.is_empty() || reset {
        // First checkpoint or reset - diff against base commit
        get_initial_checkpoint_entries(repo, &files, &base_commit)?
    } else {
        // Subsequent checkpoint - diff against last saved state
        get_subsequent_checkpoint_entries(
            repo,
            &files,
            &file_hashes,
            previous_commit.as_deref(),
            &base_commit,
        )?
    };

    let mut checkpoint = Checkpoint::new(
        base_commit.clone(),
        combined_hash.clone(),
        author.to_string(),
        entries.clone(),
    );

    // Set transcript and agent_id if provided
    if let Some(agent_run) = &agent_run_result {
        checkpoint.transcript = Some(agent_run.transcript.clone());
        checkpoint.agent_id = Some(agent_run.agent_id.clone());
    }
    let agent_tool = if let Some(agent_run_result) = &agent_run_result {
        Some(agent_run_result.agent_id.tool.as_str())
    } else {
        None
    };

    working_log.push(checkpoint);

    // Use pretty formatting in debug builds, single-line in release builds
    let working_log_json = if cfg!(debug_assertions) {
        serde_json::to_string_pretty(&working_log)?
    } else {
        serde_json::to_string(&working_log)?
    };

    put_reference(
        repo,
        &format!("ai-working-log/{}", base_commit),
        &working_log_json,
        &format!("Checkpoint by {}", author),
    )?;

    save_current_file_states(repo, &base_commit, &files)?;

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
    Ok((entries.len(), files.len(), working_log.len()))
}

fn get_all_files(repo: &Repository) -> Result<Vec<String>, GitAiError> {
    let mut files = Vec::new();

    let mut status_opts = StatusOptions::new();
    status_opts.include_untracked(true);
    status_opts.include_ignored(false);
    status_opts.include_unmodified(false);

    let statuses = repo.statuses(Some(&mut status_opts))?;
    for entry in statuses.iter() {
        if let Some(path) = entry.path() {
            // Only include text files
            if is_text_file(repo, path) {
                files.push(path.to_string());
            }
        }
    }

    // Also check for deleted files by looking at the working directory vs HEAD
    if let Ok(head) = repo.head() {
        if let Some(target) = head.target() {
            if let Ok(commit) = repo.find_commit(target) {
                if let Ok(tree) = commit.tree() {
                    // Recursively traverse the tree to find files that exist in HEAD but not in working directory
                    fn walk_tree(
                        tree: &git2::Tree,
                        repo: &Repository,
                        files: &mut Vec<String>,
                        prefix: &str,
                    ) -> Result<(), GitAiError> {
                        for entry in tree.iter() {
                            let name = entry.name().unwrap_or("");
                            let path = if prefix.is_empty() {
                                name.to_string()
                            } else {
                                format!("{}/{}", prefix, name)
                            };

                            match entry.kind() {
                                Some(git2::ObjectType::Blob) => {
                                    // Check if file exists in working directory and is a text file
                                    if !Path::new(&path).exists()
                                        && !files.contains(&path)
                                        && is_text_file(repo, &path)
                                    {
                                        files.push(path);
                                    }
                                }
                                Some(git2::ObjectType::Tree) => {
                                    if let Ok(subtree) = repo.find_tree(entry.id()) {
                                        walk_tree(&subtree, repo, files, &path)?;
                                    }
                                }
                                _ => {}
                            }
                        }
                        Ok(())
                    }

                    walk_tree(&tree, repo, &mut files, "")?;
                }
            }
        }
    }

    Ok(files)
}

fn save_current_file_states(
    repo: &Repository,
    base_commit: &str,
    files: &[String],
) -> Result<HashMap<String, String>, GitAiError> {
    let mut file_hashes = HashMap::new();

    for file_path in files {
        let repo_workdir = repo.workdir().unwrap_or_else(|| Path::new("."));
        let abs_path = repo_workdir.join(file_path);
        let content = if abs_path.exists() {
            // Read file as bytes first, then convert to string with UTF-8 lossy conversion
            match std::fs::read(&abs_path) {
                Ok(bytes) => String::from_utf8_lossy(&bytes).to_string(),
                Err(_) => String::new(), // If we can't read the file, treat as empty
            }
        } else {
            String::new()
        };

        // Create a hash for the file name to use as ref name
        let mut hasher = Sha256::new();
        hasher.update(file_path.as_bytes());
        let file_hash = format!("{:x}", hasher.finalize());

        let ref_name = format!("ai-working-log/diffs/{}-{}", base_commit, file_hash);
        put_reference(
            repo,
            &ref_name,
            &content,
            &format!("File state for {}", file_path),
        )?;

        file_hashes.insert(file_path.clone(), file_hash);
    }

    Ok(file_hashes)
}

fn get_or_create_working_log(
    repo: &Repository,
    base_commit: &str,
) -> Result<Vec<Checkpoint>, GitAiError> {
    match get_reference(repo, &format!("ai-working-log/{}", base_commit)) {
        Ok(content) => {
            let working_log: Vec<Checkpoint> = serde_json::from_str(&content)?;
            Ok(working_log)
        }
        Err(_) => Ok(Vec::new()), // No working log exists yet
    }
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
        let mut added_lines = Vec::new();
        let mut deleted_lines = Vec::new();
        let mut current_line = 1u32;

        for change in diff.iter_all_changes() {
            match change.tag() {
                ChangeTag::Equal => {
                    current_line += change.value().lines().count() as u32;
                }
                ChangeTag::Delete => {
                    let delete_start = current_line;
                    let delete_count = change.value().lines().count() as u32;
                    let delete_end = delete_start + delete_count - 1;
                    deleted_lines.push(Line::Range(delete_start, delete_end));
                }
                ChangeTag::Insert => {
                    let insert_start = current_line;
                    let insert_count = change.value().lines().count() as u32;
                    let insert_end = insert_start + insert_count - 1;
                    added_lines.push(Line::Range(insert_start, insert_end));
                    current_line += insert_count;
                }
            }
        }

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
    repo: &Repository,
    files: &[String],
    file_hashes: &HashMap<String, String>,
    previous_commit: Option<&str>,
    base_commit: &str,
) -> Result<Vec<WorkingLogEntry>, GitAiError> {
    let mut entries = Vec::new();

    for file_path in files {
        let repo_workdir = repo.workdir().unwrap();
        let abs_path = repo_workdir.join(file_path);

        // Read the content from the ai-working-log/diffs reference
        let file_hash = &file_hashes[file_path];
        let ref_name = format!("ai-working-log/diffs/{}-{}", base_commit, file_hash);
        let previous_content = get_reference(repo, &ref_name).unwrap_or_default();

        // Read current content directly from the file system
        let current_content = std::fs::read_to_string(&abs_path).unwrap_or_else(|_| String::new());

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
        let mut added_lines = Vec::new();
        let mut deleted_lines = Vec::new();
        let mut current_line = 1u32;

        for change in diff.iter_all_changes() {
            match change.tag() {
                ChangeTag::Equal => {
                    current_line += change.value().lines().count() as u32;
                }
                ChangeTag::Delete => {
                    let delete_start = current_line;
                    let delete_count = change.value().lines().count() as u32;
                    let delete_end = delete_start + delete_count - 1;
                    deleted_lines.push(Line::Range(delete_start, delete_end));
                }
                ChangeTag::Insert => {
                    let insert_start = current_line;
                    let insert_count = change.value().lines().count() as u32;
                    let insert_end = insert_start + insert_count - 1;
                    added_lines.push(Line::Range(insert_start, insert_end));
                    current_line += insert_count;
                }
            }
        }

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

fn clear_working_log_diffs(repo: &Repository, base_commit: &str) -> Result<(), GitAiError> {
    // Overwrite the refs with empty content to "clear" them
    for reference in repo.references()? {
        let reference = reference?;
        if let Some(name) = reference.name() {
            if name.starts_with(&format!(
                "refs/{}ai-working-log/diffs/{}-",
                crate::git::refs::DEFAULT_REFSPEC,
                base_commit
            )) {
                put_reference(repo, name, "", "Cleared working log diff")?;
            }
        }
    }
    Ok(())
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
