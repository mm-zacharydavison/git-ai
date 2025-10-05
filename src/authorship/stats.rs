use crate::authorship::authorship_log::LineRange;
use crate::authorship::authorship_log_serialization::AuthorshipLog;
use crate::authorship::transcript::Message;
use crate::error::GitAiError;
use crate::git::refs::get_reference_as_authorship_log_v3;
use crate::git::repository::Repository;
use similar::{ChangeTag, TextDiff};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct CommitStats {
    pub human_additions: u32,
    pub ai_additions: u32,
    pub ai_accepted: u32,
    pub time_waiting_for_ai: u64, // seconds
    pub git_diff_deleted_lines: u32,
    pub git_diff_added_lines: u32,
}

pub fn stats_for_commit_stats(
    repo: &Repository,
    commit_sha: &str,
    _refname: &str,
) -> Result<CommitStats, GitAiError> {
    // Step 1: get the diff between this commit and its parent ON refname (if more than one parent)
    // If initial than everything is additions
    // We want the count here git shows +111 -55
    let (git_diff_added_lines, git_diff_deleted_lines) = get_git_diff_stats(repo, commit_sha)?;

    // Step 2: get the authorship log for this commit
    let authorship_log = get_authorship_log_for_commit(repo, commit_sha)?;

    // Step 3: For prompts with > 1 messages, sum all the time between user messages and AI messages.
    // if the last message is a human message, don't count anything
    let (authorship_human_additions, ai_additions, ai_accepted, time_waiting_for_ai) =
        analyze_authorship_log(&authorship_log)?;

    // Calculate human additions as the difference between total git diff and AI additions
    // This handles cases where there are no AI-authored lines (authorship log is empty)
    let human_additions = if git_diff_added_lines >= ai_additions {
        git_diff_added_lines - ai_additions
    } else {
        authorship_human_additions
    };

    Ok(CommitStats {
        human_additions,
        ai_additions,
        ai_accepted,
        time_waiting_for_ai,
        git_diff_deleted_lines,
        git_diff_added_lines,
    })
}

/// Get git diff statistics between commit and its parent
fn get_git_diff_stats(repo: &Repository, commit_sha: &str) -> Result<(u32, u32), GitAiError> {
    // Get the commit object
    let commit = repo.find_commit(commit_sha.to_string())?;

    // Get parent count
    let parent_count = commit.parent_count()?;

    if parent_count == 0 {
        // Initial commit - everything is additions
        // Get the tree and count all lines
        let tree = commit.tree()?;
        let total_lines = count_lines_in_tree(repo, &tree)?;
        return Ok((total_lines, 0));
    }

    // For now, just compare with first parent (can be enhanced for merge commits)
    let parent = commit.parent(0)?;
    let parent_tree = parent.tree()?;
    let current_tree = commit.tree()?;

    // Get the diff between trees
    let diff = get_tree_diff(repo, &parent_tree, &current_tree)?;

    Ok((diff.0, diff.1))
}

/// Count lines in a git tree recursively
fn count_lines_in_tree(
    repo: &Repository,
    tree: &crate::git::repository::Tree,
) -> Result<u32, GitAiError> {
    let mut total_lines = 0u32;

    // Get list of files in tree using git ls-tree
    let mut args = repo.global_args_for_exec();
    args.push("ls-tree".to_string());
    args.push("-r".to_string()); // recursive
    args.push(tree.id().clone());

    let output = crate::git::repository::exec_git(&args)?;
    let stdout = String::from_utf8(output.stdout)?;

    for line in stdout.lines() {
        if line.trim().is_empty() {
            continue;
        }

        // Parse git ls-tree output: mode type sha\tpath
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() != 2 {
            continue;
        }

        let meta = parts[0];
        let _path = parts[1];

        // Parse mode and type
        let meta_parts: Vec<&str> = meta.split_whitespace().collect();
        if meta_parts.len() < 3 {
            continue;
        }

        let object_type = meta_parts[1];
        let sha = meta_parts[2];

        // Only count blob files
        if object_type == "blob" {
            let blob = repo.find_blob(sha.to_string())?;
            let content = blob.content()?;
            let content_str = String::from_utf8_lossy(&content);
            total_lines += content_str.lines().count() as u32;
        }
    }

    Ok(total_lines)
}

/// Get diff between two trees
fn get_tree_diff(
    repo: &Repository,
    parent_tree: &crate::git::repository::Tree,
    current_tree: &crate::git::repository::Tree,
) -> Result<(u32, u32), GitAiError> {
    let mut added_lines = 0u32;
    let mut deleted_lines = 0u32;

    // Get file lists from both trees
    let parent_files = get_tree_files(repo, parent_tree)?;
    let current_files = get_tree_files(repo, current_tree)?;

    // Compare files
    let all_files: std::collections::HashSet<String> = parent_files
        .keys()
        .chain(current_files.keys())
        .cloned()
        .collect();

    for file_path in all_files {
        let parent_content = parent_files.get(&file_path);
        let current_content = current_files.get(&file_path);

        match (parent_content, current_content) {
            (None, Some(current)) => {
                // File was added
                added_lines += count_lines_in_content(current);
            }
            (Some(parent), None) => {
                // File was deleted
                deleted_lines += count_lines_in_content(parent);
            }
            (Some(parent), Some(current)) => {
                // File was modified - get diff
                if parent != current {
                    let (added, deleted) = diff_content(parent, current);
                    added_lines += added;
                    deleted_lines += deleted;
                }
            }
            (None, None) => {
                // Should not happen
            }
        }
    }

    Ok((added_lines, deleted_lines))
}

/// Get all files in a tree with their content
fn get_tree_files(
    repo: &Repository,
    tree: &crate::git::repository::Tree,
) -> Result<HashMap<String, String>, GitAiError> {
    let mut files = HashMap::new();

    let mut args = repo.global_args_for_exec();
    args.push("ls-tree".to_string());
    args.push("-r".to_string());
    args.push(tree.id().clone());

    let output = crate::git::repository::exec_git(&args)?;
    let stdout = String::from_utf8(output.stdout)?;

    for line in stdout.lines() {
        if line.trim().is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() != 2 {
            continue;
        }

        let meta = parts[0];
        let path = parts[1];

        let meta_parts: Vec<&str> = meta.split_whitespace().collect();
        if meta_parts.len() < 3 {
            continue;
        }

        let object_type = meta_parts[1];
        let sha = meta_parts[2];

        if object_type == "blob" {
            let blob = repo.find_blob(sha.to_string())?;
            let content = blob.content()?;
            let content_str = String::from_utf8_lossy(&content);
            files.insert(path.to_string(), content_str.to_string());
        }
    }

    Ok(files)
}

/// Count lines in content string
fn count_lines_in_content(content: &str) -> u32 {
    content.lines().count() as u32
}

/// Get diff between two content strings
fn diff_content(parent: &str, current: &str) -> (u32, u32) {
    let diff = TextDiff::from_lines(parent, current);
    let mut added = 0u32;
    let mut deleted = 0u32;

    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Equal => {
                // No change
            }
            ChangeTag::Insert => {
                added += change.value().lines().count() as u32;
            }
            ChangeTag::Delete => {
                deleted += change.value().lines().count() as u32;
            }
        }
    }

    (added, deleted)
}

/// Get authorship log for a commit
fn get_authorship_log_for_commit(
    repo: &Repository,
    commit_sha: &str,
) -> Result<AuthorshipLog, GitAiError> {
    let ref_name = format!("ai/authorship/{}", commit_sha);
    match get_reference_as_authorship_log_v3(repo, &ref_name) {
        Ok(log) => Ok(log),
        Err(_) => {
            // No authorship log found - return empty log
            Ok(AuthorshipLog::new())
        }
    }
}

/// Analyze authorship log to extract statistics
fn analyze_authorship_log(
    authorship_log: &AuthorshipLog,
) -> Result<(u32, u32, u32, u64), GitAiError> {
    let mut human_additions = 0u32;
    let mut ai_additions = 0u32;
    let mut ai_accepted = 0u32;
    let mut time_waiting_for_ai = 0u64;

    // Count lines by author type
    for file_attestation in &authorship_log.attestations {
        for entry in &file_attestation.entries {
            // Count lines in this entry
            let lines_in_entry: u32 = entry
                .line_ranges
                .iter()
                .map(|range| match range {
                    LineRange::Single(_) => 1,
                    LineRange::Range(start, end) => end - start + 1,
                })
                .sum();

            // Check if this is an AI-generated entry
            if let Some(prompt_record) = authorship_log.metadata.prompts.get(&entry.hash) {
                ai_additions += lines_in_entry;

                // Count accepted lines (this is a simplified approach)
                // In a real implementation, you might want to track acceptance more precisely
                ai_accepted += lines_in_entry; // For now, assume all AI lines are accepted

                // Calculate time waiting for AI from transcript
                // Create a transcript from the messages
                let transcript = crate::authorship::transcript::AiTranscript {
                    messages: prompt_record.messages.clone(),
                };
                time_waiting_for_ai += calculate_waiting_time(&transcript);
            } else {
                // Human-authored lines
                human_additions += lines_in_entry;
            }
        }
    }

    Ok((
        human_additions,
        ai_additions,
        ai_accepted,
        time_waiting_for_ai,
    ))
}

/// Calculate time waiting for AI from transcript messages
fn calculate_waiting_time(transcript: &crate::authorship::transcript::AiTranscript) -> u64 {
    let mut total_waiting_time = 0u64;
    let messages = transcript.messages();

    if messages.len() <= 1 {
        return 0;
    }

    // Check if last message is from human (don't count time if so)
    let last_message_is_human = matches!(messages.last(), Some(Message::User { .. }));
    if last_message_is_human {
        return 0;
    }

    // Sum time between user and AI messages
    let mut i = 0;
    while i < messages.len() - 1 {
        if let (
            Message::User {
                timestamp: Some(user_ts),
                ..
            },
            Message::Assistant {
                timestamp: Some(ai_ts),
                ..
            },
        ) = (&messages[i], &messages[i + 1])
        {
            // Parse timestamps and calculate difference
            if let (Ok(user_time), Ok(ai_time)) = (
                chrono::DateTime::parse_from_rfc3339(user_ts),
                chrono::DateTime::parse_from_rfc3339(ai_ts),
            ) {
                let duration = ai_time.signed_duration_since(user_time);
                if duration.num_seconds() > 0 {
                    total_waiting_time += duration.num_seconds() as u64;
                }
            }

            i += 2; // Skip to next user message
        } else {
            i += 1;
        }
    }

    total_waiting_time
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_utils::TmpRepo;

    #[test]
    fn test_stats_for_simple_ai_commit() {
        let tmp_repo = TmpRepo::new().unwrap();

        let mut file = tmp_repo.write_file("test.txt", "Line1\n", true).unwrap();

        tmp_repo
            .trigger_checkpoint_with_author("test_user")
            .unwrap();

        tmp_repo.commit_with_message("Initial commit").unwrap();

        // AI adds 2 lines
        file.append("Line 2\nLine 3\n").unwrap();

        tmp_repo
            .trigger_checkpoint_with_ai("Claude", Some("claude-3-sonnet"), Some("cursor"))
            .unwrap();

        tmp_repo.commit_with_message("AI adds lines").unwrap();

        // Get the commit SHA for the AI commit
        let head_sha = tmp_repo.get_head_commit_sha().unwrap();

        // Test our stats function
        let stats = stats_for_commit_stats(&tmp_repo.gitai_repo(), &head_sha, "HEAD").unwrap();

        println!("stats: {:?}", stats);

        // Verify the stats
        assert_eq!(
            stats.human_additions, 0,
            "No human additions in AI-only commit"
        );
        assert_eq!(stats.ai_additions, 2, "AI added 2 lines");
        assert_eq!(stats.ai_accepted, 2, "AI lines were accepted");
        assert_eq!(
            stats.git_diff_added_lines, 2,
            "Git diff shows 2 added lines"
        );
        assert_eq!(
            stats.git_diff_deleted_lines, 0,
            "Git diff shows 0 deleted lines"
        );
        assert_eq!(
            stats.time_waiting_for_ai, 0,
            "No waiting time recorded (no timestamps in test)"
        );
    }

    #[test]
    fn test_stats_for_mixed_commit() {
        let tmp_repo = TmpRepo::new().unwrap();

        let mut file = tmp_repo
            .write_file("test.txt", "Base line\n", true)
            .unwrap();

        tmp_repo
            .trigger_checkpoint_with_author("test_user")
            .unwrap();

        tmp_repo.commit_with_message("Initial commit").unwrap();

        // AI adds lines
        file.append("AI line 1\nAI line 2\n").unwrap();
        tmp_repo
            .trigger_checkpoint_with_ai("Claude", Some("claude-3-sonnet"), Some("cursor"))
            .unwrap();

        // Human adds lines
        file.append("Human line 1\nHuman line 2\n").unwrap();
        tmp_repo
            .trigger_checkpoint_with_author("test_user")
            .unwrap();

        tmp_repo.commit_with_message("Mixed commit").unwrap();

        let head_sha = tmp_repo.get_head_commit_sha().unwrap();
        let stats = stats_for_commit_stats(&tmp_repo.gitai_repo(), &head_sha, "HEAD").unwrap();

        // Verify the stats
        assert_eq!(stats.human_additions, 2, "Human added 2 lines");
        assert_eq!(stats.ai_additions, 2, "AI added 2 lines");
        assert_eq!(stats.ai_accepted, 2, "AI lines were accepted");
        assert_eq!(
            stats.git_diff_added_lines, 4,
            "Git diff shows 4 added lines total"
        );
        assert_eq!(
            stats.git_diff_deleted_lines, 0,
            "Git diff shows 0 deleted lines"
        );
    }

    #[test]
    fn test_stats_for_initial_commit() {
        let tmp_repo = TmpRepo::new().unwrap();

        let _file = tmp_repo
            .write_file("test.txt", "Line1\nLine2\nLine3\n", true)
            .unwrap();

        tmp_repo
            .trigger_checkpoint_with_author("test_user")
            .unwrap();

        tmp_repo.commit_with_message("Initial commit").unwrap();

        let head_sha = tmp_repo.get_head_commit_sha().unwrap();
        let stats = stats_for_commit_stats(&tmp_repo.gitai_repo(), &head_sha, "HEAD").unwrap();

        // For initial commit, everything should be additions
        assert_eq!(
            stats.human_additions, 3,
            "Human authored 3 lines in initial commit"
        );
        assert_eq!(stats.ai_additions, 0, "No AI additions in initial commit");
        assert_eq!(stats.ai_accepted, 0, "No AI lines to accept");
        assert_eq!(
            stats.git_diff_added_lines, 3,
            "Git diff shows 3 added lines (initial commit)"
        );
        assert_eq!(
            stats.git_diff_deleted_lines, 0,
            "Git diff shows 0 deleted lines"
        );
    }
}
