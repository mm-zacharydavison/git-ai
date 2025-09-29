use git_ai::tmp_repo::TmpRepo;
use insta::assert_debug_snapshot;
use tempfile::tempdir;

#[test]
fn test_simple_additions_empty_repo() {
    // Create a temporary directory
    let tmp_dir = tempdir().unwrap();
    let repo_path = tmp_dir.path().to_path_buf();

    let tmp_repo = TmpRepo::new(repo_path.clone()).unwrap();

    let mut file = tmp_repo.write_file("test.txt", "Line1\n", true).unwrap();

    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    file.append("Line 2\nLine 3\n").unwrap();

    tmp_repo
        .trigger_checkpoint_with_ai("Claude", Some("claude-3-sonnet"), Some("cursor"))
        .unwrap();

    tmp_repo.commit_with_message("Initial commit").unwrap();

    let blame = tmp_repo.blame_for_file(&file, None).unwrap();
    assert_debug_snapshot!(blame);
}

#[test]
fn test_simple_additions_with_base_commit() {
    let tmp_dir = tempdir().unwrap();
    let repo_path = tmp_dir.path().to_path_buf();

    let (tmp_repo, mut lines, _) = TmpRepo::new_with_base_commit(repo_path.clone()).unwrap();

    lines
        .append("NEW LINEs From Claude!\nHello\nWorld\n")
        .unwrap();

    tmp_repo
        .trigger_checkpoint_with_ai("Claude", Some("claude-3-sonnet"), Some("cursor"))
        .unwrap();

    tmp_repo.commit_with_message("next commit").unwrap();

    let blame = tmp_repo.blame_for_file(&lines, None).unwrap();

    assert_debug_snapshot!(blame);
}

#[test]
fn test_simple_additions_on_top_of_ai_contributions() {
    let tmp_dir = tempdir().unwrap();
    let repo_path = tmp_dir.path().to_path_buf();

    let (tmp_repo, mut lines, _) = TmpRepo::new_with_base_commit(repo_path.clone()).unwrap();

    lines
        .append("NEW LINEs From Claude!\nHello\nWorld\n")
        .unwrap();

    tmp_repo
        .trigger_checkpoint_with_ai("Claude", Some("claude-3-sonnet"), Some("cursor"))
        .unwrap();

    tmp_repo.commit_with_message("next commit 1").unwrap();

    lines.replace_range(34, 35, "HUMAN ON AI\n").unwrap();

    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    tmp_repo.commit_with_message("next commit 2").unwrap();

    let blame = tmp_repo.blame_for_file(&lines, Some((30, 35))).unwrap();
    assert_debug_snapshot!(blame);
}

#[test]
fn test_simple_additions_new_file_not_git_added() {
    let tmp_dir = tempdir().unwrap();
    let repo_path = tmp_dir.path().to_path_buf();

    let tmp_repo = TmpRepo::new(repo_path.clone()).unwrap();

    // Create a new file that hasn't been git added yet
    let mut file = tmp_repo
        .write_file(
            "new_file.txt",
            "Line 1 from test_user\nLine 2 from test_user\nLine 3 from test_user\n",
            false,
        )
        .unwrap();

    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    // Append 3 more lines from Claude
    file.append("Line 4 from Claude\nLine 5 from Claude\nLine 6 from Claude\n")
        .unwrap();

    tmp_repo
        .trigger_checkpoint_with_ai("Claude", Some("claude-3-sonnet"), Some("cursor"))
        .unwrap();

    // Now commit (which will add all files including the new one)
    tmp_repo
        .commit_with_message("Add new file with mixed authorship")
        .unwrap();

    let blame = tmp_repo.blame_for_file(&file, None).unwrap();
    assert_debug_snapshot!(blame);
}

#[test]
fn test_ai_adds_then_human_deletes_and_replaces() {
    let tmp_dir = tempdir().unwrap();
    let repo_path = tmp_dir.path().to_path_buf();

    let tmp_repo = TmpRepo::new(repo_path.clone()).unwrap();

    // Start with a base file
    let mut file = tmp_repo
        .write_file("xyz.ts", "line1\nline2\n", true)
        .unwrap();

    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    // AI adds lines at the end
    file.append("ai_line1\nai_line2\nai_line3\nai_line4\n")
        .unwrap();

    tmp_repo
        .trigger_checkpoint_with_ai("Claude", Some("claude-3-sonnet"), Some("cursor"))
        .unwrap();

    // Human deletes the AI lines and adds their own
    file.replace_range(3, 7, "human_line1\n").unwrap();

    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    tmp_repo
        .commit_with_message("AI adds lines, human deletes and replaces")
        .unwrap();

    let blame = tmp_repo.blame_for_file(&file, None).unwrap();
    assert_debug_snapshot!(blame);
}

#[test]
fn test_ai_adds_middle_then_human_deletes_and_replaces() {
    let tmp_dir = tempdir().unwrap();
    let repo_path = tmp_dir.path().to_path_buf();

    let tmp_repo = TmpRepo::new(repo_path.clone()).unwrap();

    // Start with a base file
    let mut file = tmp_repo
        .write_file("middle_test.ts", "line1\nline2\nline3\nline4\n", true)
        .unwrap();

    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    // AI adds lines in the middle (after line 2)
    file.append("ai_middle1\nai_middle2\nai_middle3\n").unwrap();

    tmp_repo
        .trigger_checkpoint_with_ai("Claude", Some("claude-3-sonnet"), Some("cursor"))
        .unwrap();

    // Human deletes 2 AI lines and adds 1 human line
    file.replace_range(5, 8, "human_middle\n").unwrap();

    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    tmp_repo
        .commit_with_message("AI adds middle lines, human deletes and replaces")
        .unwrap();

    let blame = tmp_repo.blame_for_file(&file, None).unwrap();
    assert_debug_snapshot!(blame);
}

#[test]
fn test_multiple_ai_checkpoints_with_human_deletions() {
    let tmp_dir = tempdir().unwrap();
    let repo_path = tmp_dir.path().to_path_buf();

    let tmp_repo = TmpRepo::new(repo_path.clone()).unwrap();

    // Start with a base file
    let mut file = tmp_repo
        .write_file("multi_ai.ts", "base1\nbase2\n", true)
        .unwrap();

    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    // First AI session adds lines
    file.append("ai1_line1\nai1_line2\n").unwrap();
    tmp_repo
        .trigger_checkpoint_with_ai("Claude", Some("claude-3-sonnet"), Some("cursor"))
        .unwrap();

    // Second AI session adds more lines
    file.append("ai2_line1\nai2_line2\n").unwrap();
    tmp_repo
        .trigger_checkpoint_with_ai("GPT-4", Some("gpt-4"), Some("windsurf"))
        .unwrap();

    // Human deletes lines from both AI sessions
    file.replace_range(3, 6, "human_replacement\n").unwrap();

    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    tmp_repo
        .commit_with_message("Multiple AI sessions with selective human deletions")
        .unwrap();

    let blame = tmp_repo.blame_for_file(&file, None).unwrap();

    assert_debug_snapshot!(blame);
}

#[test]
fn test_ai_adds_then_human_deletes_all_ai_lines() {
    let tmp_dir = tempdir().unwrap();
    let repo_path = tmp_dir.path().to_path_buf();

    let tmp_repo = TmpRepo::new(repo_path.clone()).unwrap();

    // Start with a base file
    let mut file = tmp_repo
        .write_file("delete_all.ts", "human_base\n", true)
        .unwrap();

    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    // AI adds lines
    file.append("ai_line1\nai_line2\nai_line3\n").unwrap();
    tmp_repo
        .trigger_checkpoint_with_ai("Claude", Some("claude-3-sonnet"), Some("cursor"))
        .unwrap();

    // Human deletes ALL AI lines and adds only human content
    file.replace_range(2, 5, "human_only1\nhuman_only2\n")
        .unwrap();

    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    tmp_repo
        .commit_with_message("AI adds lines, human deletes all AI content")
        .unwrap();

    let blame = tmp_repo.blame_for_file(&file, None).unwrap();
    println!("blame: {:?}", blame);

    // Debug: Print the authorship log content
    let authorship_log = tmp_repo.get_authorship_log().unwrap();
    println!("authorship_log: {:?}", authorship_log);

    // Debug: Print the working log to see what entries were created
    let working_log_ref = format!("ai-working-log/{}", "initial");
    let working_log_content =
        std::fs::read_to_string(format!(".git/refs/{}", working_log_ref)).unwrap_or_default();
    println!("working_log_content: {}", working_log_content);

    // Debug: Let's also check what the working log entries look like by parsing them
    if !working_log_content.is_empty() {
        let working_log: Result<Vec<git_ai::log_fmt::working_log::Checkpoint>, _> =
            serde_json::from_str(&working_log_content);
        if let Ok(log) = working_log {
            println!("Parsed working log: {:?}", log);
        }
    }

    // Debug: Let's also check what the file content looks like at each step
    println!("Final file content: {:?}", file.contents());

    assert_debug_snapshot!(blame);
}

#[test]
fn test_human_adds_then_ai_modifies_then_human_deletes() {
    let tmp_dir = tempdir().unwrap();
    let repo_path = tmp_dir.path().to_path_buf();

    let tmp_repo = TmpRepo::new(repo_path.clone()).unwrap();

    // Start with a base file
    let mut file = tmp_repo
        .write_file("reverse_test.ts", "base\n", true)
        .unwrap();

    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    // Human adds lines
    file.append("human_line1\nhuman_line2\nhuman_line3\n")
        .unwrap();
    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    // AI modifies some human lines
    file.replace_range(2, 3, "ai_modified_human1\n").unwrap();
    tmp_repo
        .trigger_checkpoint_with_ai("Claude", Some("claude-3-sonnet"), Some("cursor"))
        .unwrap();

    // Human deletes the AI modifications and adds their own
    file.replace_range(2, 4, "human_final1\nhuman_final2\n")
        .unwrap();

    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    tmp_repo
        .commit_with_message("Human adds, AI modifies, human deletes AI changes")
        .unwrap();

    let blame = tmp_repo.blame_for_file(&file, None).unwrap();
    assert_debug_snapshot!(blame);
}

#[test]
fn test_complex_mixed_additions_and_deletions() {
    let tmp_dir = tempdir().unwrap();
    let repo_path = tmp_dir.path().to_path_buf();

    let tmp_repo = TmpRepo::new(repo_path.clone()).unwrap();

    // Start with a base file
    let mut file = tmp_repo.write_file("complex.ts", "start\n", true).unwrap();

    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    // AI adds lines
    file.append("ai1\nai2\nai3\n").unwrap();
    tmp_repo
        .trigger_checkpoint_with_ai("Claude", Some("claude-3-sonnet"), Some("cursor"))
        .unwrap();

    // Human adds lines
    file.append("human1\nhuman2\n").unwrap();
    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    // AI adds more lines
    file.append("ai4\nai5\n").unwrap();
    tmp_repo
        .trigger_checkpoint_with_ai("GPT-4", Some("gpt-4"), Some("windsurf"))
        .unwrap();

    // Human deletes some AI lines and some human lines, adds new content
    file.replace_range(2, 5, "mixed1\nmixed2\n").unwrap();

    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    // AI modifies the mixed content
    file.replace_range(2, 3, "ai_final\n").unwrap();
    tmp_repo
        .trigger_checkpoint_with_ai("Claude", Some("claude-3-sonnet"), Some("cursor"))
        .unwrap();

    tmp_repo
        .commit_with_message("Complex mixed additions and deletions")
        .unwrap();

    let blame = tmp_repo.blame_for_file(&file, None).unwrap();
    assert_debug_snapshot!(blame);
}
