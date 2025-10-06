use crate::authorship::authorship_log::LineRange;
use crate::authorship::authorship_log_serialization::AuthorshipLog;
use crate::authorship::transcript::Message;
use crate::error::GitAiError;
use crate::git::refs::get_reference_as_authorship_log_v3;
use crate::git::repository::Repository;

#[derive(Debug, Clone)]
pub struct CommitStats {
    pub human_additions: u32,
    pub ai_additions: u32,
    pub ai_accepted: u32,
    pub time_waiting_for_ai: u64, // seconds
    pub git_diff_deleted_lines: u32,
    pub git_diff_added_lines: u32,
}

pub fn write_stats_to_terminal(stats: &CommitStats) {
    // Get terminal width, default to 80 if not available
    let terminal_width = terminal_width().unwrap_or(80);
    let bar_width = terminal_width.saturating_sub(10); // Leave some padding

    // Calculate total additions for the progress bar
    let total_additions = stats.human_additions + stats.ai_additions;

    // Calculate AI acceptance percentage
    let ai_acceptance_percentage = if stats.ai_additions > 0 {
        (stats.ai_accepted as f64 / stats.ai_additions as f64) * 100.0
    } else {
        0.0
    };

    // Create progress bar
    let ai_bars = if total_additions > 0 {
        ((stats.ai_additions as f64 / total_additions as f64) * bar_width as f64) as usize
    } else {
        0
    };
    let human_bars = bar_width.saturating_sub(ai_bars);

    // Build the progress bar with different characters for visual distinction
    let mut progress_bar = String::new();
    progress_bar.push_str("you  ");
    progress_bar.push_str(&"░".repeat(human_bars));
    progress_bar.push_str(&"▒".repeat(ai_bars.saturating_sub(ai_bars / 3)));
    progress_bar.push_str(&"█".repeat(ai_bars.saturating_sub(2 * ai_bars / 3)));
    progress_bar.push_str(" ai");

    // Format time waiting for AI
    let waiting_time_str = if stats.time_waiting_for_ai > 0 {
        let minutes = stats.time_waiting_for_ai / 60;
        let seconds = stats.time_waiting_for_ai % 60;
        if minutes > 0 {
            format!("{}m {}s", minutes, seconds)
        } else {
            format!("{}s", seconds)
        }
    } else {
        "0s".to_string()
    };

    // Print the stats
    println!("{}", progress_bar);
    println!(
        "+{} -{} (git diff stat) {:.0}% AI code accepted",
        stats.git_diff_added_lines, stats.git_diff_deleted_lines, ai_acceptance_percentage
    );

    // Only show waiting time if there was actual waiting
    if stats.time_waiting_for_ai > 0 {
        println!("{} waiting for ai", waiting_time_str);
    }
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
    // Use git show --numstat to get diff statistics
    let mut args = repo.global_args_for_exec();
    args.push("show".to_string());
    args.push("--numstat".to_string());
    args.push("--format=".to_string()); // No format, just the numstat
    args.push(commit_sha.to_string());

    let output = crate::git::repository::exec_git(&args)?;
    let stdout = String::from_utf8(output.stdout)?;

    let mut added_lines = 0u32;
    let mut deleted_lines = 0u32;

    // Parse numstat output
    for line in stdout.lines() {
        if line.trim().is_empty() {
            continue;
        }

        // Skip the commit message lines (they don't start with numbers)
        if !line.chars().next().map_or(false, |c| c.is_ascii_digit()) {
            continue;
        }

        // Parse numstat format: "added\tdeleted\tfilename"
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() >= 2 {
            // Parse added lines
            if let Ok(added) = parts[0].parse::<u32>() {
                added_lines += added;
            }

            // Parse deleted lines (handle "-" for binary files)
            if parts[1] != "-" {
                if let Ok(deleted) = parts[1].parse::<u32>() {
                    deleted_lines += deleted;
                }
            }
        }
    }

    Ok((added_lines, deleted_lines))
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

/// Get terminal width, with fallback to 80
fn terminal_width() -> Option<usize> {
    // Try to get terminal width from environment or use a reasonable default
    std::env::var("COLUMNS")
        .ok()
        .and_then(|s| s.parse().ok())
        .or_else(|| {
            // Try to get from terminfo or use default
            Some(80)
        })
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
    fn test_terminal_stats_display() {
        // Test with mixed human/AI stats
        let stats = CommitStats {
            human_additions: 50,
            ai_additions: 30,
            ai_accepted: 25,
            time_waiting_for_ai: 90, // 1 minute 30 seconds
            git_diff_deleted_lines: 15,
            git_diff_added_lines: 80,
        };

        // This test just ensures the function doesn't panic
        // The actual output would be:
        // you  ░░░░░░░░░░░░░░░░░░░░░▒▒▒▒▒▒▒ ai
        // +80 -15 (git diff stat) 83% AI code accepted
        // 1m 30s waiting for ai
        write_stats_to_terminal(&stats);

        // Test with AI-only stats
        let ai_stats = CommitStats {
            human_additions: 0,
            ai_additions: 100,
            ai_accepted: 95,
            time_waiting_for_ai: 45,
            git_diff_deleted_lines: 0,
            git_diff_added_lines: 100,
        };

        write_stats_to_terminal(&ai_stats);

        // Test with human-only stats
        let human_stats = CommitStats {
            human_additions: 75,
            ai_additions: 0,
            ai_accepted: 0,
            time_waiting_for_ai: 0,
            git_diff_deleted_lines: 10,
            git_diff_added_lines: 75,
        };

        write_stats_to_terminal(&human_stats);
    }

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
