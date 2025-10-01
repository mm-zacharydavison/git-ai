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

    lines.replace_range(34, 36, "HUMAN ON AI\n").unwrap();

    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    tmp_repo.commit_with_message("next commit 2").unwrap();

    let blame = tmp_repo.blame_for_file(&lines, Some((30, 34))).unwrap();
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

#[test]
fn test_ai_adds_then_human_deletes_all_with_empty_replacement() {
    let tmp_dir = tempdir().unwrap();
    let repo_path = tmp_dir.path().to_path_buf();

    let tmp_repo = TmpRepo::new(repo_path.clone()).unwrap();

    // Start with a base file
    let mut file = tmp_repo
        .write_file("empty_replacement.ts", "base_line\n", true)
        .unwrap();

    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    // AI adds lines
    file.append("ai_line1\nai_line2\nai_line3\nai_line4\n")
        .unwrap();

    tmp_repo
        .trigger_checkpoint_with_ai("Claude", Some("claude-3-sonnet"), Some("cursor"))
        .unwrap();

    // Human deletes ALL AI lines and replaces with empty string
    file.replace_range(2, 6, "").unwrap();

    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    let authorship_log = tmp_repo
        .commit_with_message("AI adds lines, human deletes all AI content with empty replacement")
        .unwrap();

    // has prompt, but not attestations
    assert_debug_snapshot!(authorship_log);
    let blame = tmp_repo.blame_for_file(&file, None).unwrap();
    assert_debug_snapshot!(blame);
}

#[test]
fn test_ai_adds_lines_and_human_deletes_most_of_them() {
    let tmp_dir = tempdir().unwrap();
    let repo_path = tmp_dir.path().to_path_buf();

    let tmp_repo = TmpRepo::new(repo_path.clone()).unwrap();

    // Start with a base file
    let mut file = tmp_repo
        .write_file("empty_replacement.ts", "base_line\n", true)
        .unwrap();

    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    // AI adds lines
    file.append("ai_line1\nai_line2\nai_line3\nai_line4\n")
        .unwrap();

    tmp_repo
        .trigger_checkpoint_with_ai("Claude", Some("claude-3-sonnet"), Some("cursor"))
        .unwrap();

    // Human deletes ALL AI lines and replaces with empty string
    file.replace_range(2, 3, "").unwrap();

    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    let authorship_log = tmp_repo
        .commit_with_message("AI adds lines, human deletes all AI content with empty replacement")
        .unwrap();

    assert_debug_snapshot!(authorship_log);

    // Assert on specific fields
    let prompt = authorship_log.metadata.prompts.values().next().unwrap();
    assert_eq!(prompt.total_additions, 4);
    assert_eq!(prompt.total_deletions, 0);
    assert_eq!(prompt.accepted_lines, 3); // Fixed: now correctly tracking all 3 remaining AI lines

    let blame = tmp_repo.blame_for_file(&file, None).unwrap();
    assert_debug_snapshot!(blame);
}

#[test]
fn test_ai_prepending_lines() {
    let tmp_dir = tempdir().unwrap();
    let repo_path = tmp_dir.path().to_path_buf();

    let tmp_repo = TmpRepo::new(repo_path.clone()).unwrap();

    // Start with a base file
    let mut file = tmp_repo
        .write_file("test.ts", "human_line1\nhuman_line2\n", true)
        .unwrap();

    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    // AI adds lines
    file.prepend("ai_line1\nai_line2\nai_line3\nai_line4\n")
        .unwrap();

    tmp_repo
        .trigger_checkpoint_with_ai("Claude", Some("claude-3-sonnet"), Some("cursor"))
        .unwrap();

    // Human deletes ALL AI lines and replaces with empty string
    file.replace_range(1, 2, "NEW HUMAN LINE").unwrap();

    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    let _authorship_log = tmp_repo.commit_with_message("test commit").unwrap();

    // println!("Authorship log: {:?}", authorship_log);
    let blame = tmp_repo.blame_for_file(&file, None).unwrap();
    // println!("Blame: {:?}", blame);
    assert_debug_snapshot!(blame);
}

#[test]
fn test_duplicate_prompt_entries_bug() {
    // This test reproduces the bug where the same AI session creates multiple
    // separate attestation entries instead of consolidating them
    let tmp_dir = tempdir().unwrap();
    let repo_path = tmp_dir.path().to_path_buf();

    let tmp_repo = TmpRepo::new(repo_path.clone()).unwrap();

    // Start with a base file
    let mut file = tmp_repo
        .write_file("file.txt", "line1\nline2\n", true)
        .unwrap();

    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    // AI adds lines 3-4
    file.append("ai_line3\nai_line4\n").unwrap();
    tmp_repo
        .trigger_checkpoint_with_ai("Claude", Some("claude-3-sonnet"), Some("some-ai"))
        .unwrap();

    // Human adds line 5
    file.append("human_line5\n").unwrap();
    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    // AI adds lines 6-8 (SAME AI SESSION as before)
    file.append("ai_line6\nai_line7\nai_line8\n").unwrap();
    tmp_repo
        .trigger_checkpoint_with_ai("Claude", Some("claude-3-sonnet"), Some("some-ai"))
        .unwrap();

    let authorship_log = tmp_repo.commit_with_message("Test commit").unwrap();

    // Debug: print the authorship log
    println!("{}", authorship_log.serialize_to_string().unwrap());

    // Check that there's only ONE prompt entry for this AI session
    assert_eq!(authorship_log.metadata.prompts.len(), 1);

    // Check that there's only ONE attestation entry for this file+prompt pair
    let file_attestation = authorship_log
        .attestations
        .iter()
        .find(|f| f.file_path == "file.txt")
        .expect("Should have attestation for file.txt");

    // BUG: This currently fails because we have multiple entries with the same hash!
    // Expected: 1 entry with consolidated line ranges like "3-4,6-8"
    // Actual: 2 entries both with the same hash
    assert_eq!(
        file_attestation.entries.len(),
        1,
        "Should have exactly 1 attestation entry, but got: {:?}",
        file_attestation.entries
    );

    let blame = tmp_repo.blame_for_file(&file, None).unwrap();
    assert_debug_snapshot!(authorship_log);
    assert_debug_snapshot!(blame);
}

#[test]
fn test_ai_human_interleaved_line_attribution() {
    // This test checks that line ranges are correctly attributed when
    // AI and human alternate making changes
    let tmp_dir = tempdir().unwrap();
    let repo_path = tmp_dir.path().to_path_buf();

    let tmp_repo = TmpRepo::new(repo_path.clone()).unwrap();

    let mut file = tmp_repo.write_file("file.txt", "", true).unwrap();

    // AI adds 3 lines (lines 1-3)
    file.append("ai1\nai2\nai3\n").unwrap();
    tmp_repo
        .trigger_checkpoint_with_ai("Claude", Some("claude-3-sonnet"), Some("some-ai"))
        .unwrap();

    // Human deletes line 2 and adds a human line
    // Expected: ai1, human, ai3
    file.replace_range(2, 3, "human\n").unwrap();
    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    // AI adds 3 more lines at the end (lines 4-6)
    file.append("ai4\nai5\nai6\n").unwrap();
    tmp_repo
        .trigger_checkpoint_with_ai("Claude", Some("claude-3-sonnet"), Some("some-ai"))
        .unwrap();

    // Human removes line 5
    file.replace_range(5, 6, "").unwrap();
    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    let authorship_log = tmp_repo.commit_with_message("Test commit").unwrap();

    // Debug: print the authorship log
    println!("{}", authorship_log.serialize_to_string().unwrap());

    // Final file should be:
    // 1: ai1 (AI)
    // 2: human (HUMAN)
    // 3: ai3 (AI)
    // 4: ai4 (AI)
    // 5: ai6 (AI) - was line 6, shifted up after line 5 deletion

    // Check the attestation entries
    let file_attestation = authorship_log
        .attestations
        .iter()
        .find(|f| f.file_path == "file.txt")
        .expect("Should have attestation for file.txt");

    println!("Attestation entries: {:?}", file_attestation.entries);

    let blame = tmp_repo.blame_for_file(&file, None).unwrap();
    assert_debug_snapshot!(authorship_log);
    assert_debug_snapshot!(blame);
}

#[test]
fn test_simple_ai_then_human_deletion() {
    // Simplified test: AI adds 3 lines, human deletes the middle one
    let tmp_dir = tempdir().unwrap();
    let repo_path = tmp_dir.path().to_path_buf();

    let tmp_repo = TmpRepo::new(repo_path.clone()).unwrap();
    let mut file = tmp_repo.write_file("file.txt", "", true).unwrap();

    // AI adds 3 lines (lines 1-3: ai1, ai2, ai3)
    file.append("ai1\nai2\nai3\n").unwrap();
    tmp_repo
        .trigger_checkpoint_with_ai("Claude", Some("claude-3-sonnet"), Some("some-ai"))
        .unwrap();

    println!("\n=== After AI adds 3 lines ===");
    println!("File content:\n{}", file.contents());

    // Human deletes line 2 (ai2)
    file.replace_range(2, 3, "").unwrap();
    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    println!("\n=== After human deletes line 2 ===");
    println!("File content:\n{}", file.contents());

    let authorship_log = tmp_repo.commit_with_message("Test commit").unwrap();

    println!("\n=== Authorship Log ===");
    println!("{}", authorship_log.serialize_to_string().unwrap());

    // Final file should be:
    // Line 1: ai1 (AI)
    // Line 2: ai3 (AI)

    // Check the attestation
    let file_attestation = authorship_log
        .attestations
        .iter()
        .find(|f| f.file_path == "file.txt")
        .expect("Should have attestation for file.txt");

    println!("\n=== Attestation entries ===");
    for entry in &file_attestation.entries {
        println!("  Hash: {}, Ranges: {:?}", entry.hash, entry.line_ranges);
    }

    let blame = tmp_repo.blame_for_file(&file, None).unwrap();
    println!("\n=== Blame ===");
    for (line, author) in &blame {
        println!("  Line {}: {}", line, author);
    }

    // Both lines should be attributed to AI
    assert_eq!(
        blame.get(&1),
        Some(&"some-ai".to_string()),
        "Line 1 should be AI"
    );
    assert_eq!(
        blame.get(&2),
        Some(&"some-ai".to_string()),
        "Line 2 should be AI"
    );

    assert_debug_snapshot!(authorship_log);
}
