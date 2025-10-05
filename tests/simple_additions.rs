use git_ai::git::test_utils::TmpRepo;
use git_ai::log_fmt::authorship_log::LineRange;
use insta::assert_debug_snapshot;

#[test]
fn test_simple_additions_empty_repo() {
    let tmp_repo = TmpRepo::new().unwrap();

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
    let (tmp_repo, mut lines, _) = TmpRepo::new_with_base_commit().unwrap();

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
    let (tmp_repo, mut lines, _) = TmpRepo::new_with_base_commit().unwrap();

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
    let tmp_repo = TmpRepo::new().unwrap();

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

    file.append("Line 4 from AI\nLine 5 from AI\n").unwrap();

    tmp_repo
        .trigger_checkpoint_with_ai("Claude", Some("claude-3-sonnet"), Some("cursor"))
        .unwrap();

    // Git add and commit
    let authorship_log = tmp_repo.commit_with_message("Initial commit").unwrap();

    // The file should only have the AI lines attributed (lines 4-5)
    // Because the file wasn't tracked when test_user's checkpoint ran
    assert_debug_snapshot!(authorship_log);
}

#[test]
fn test_ai_human_interleaved_line_attribution() {
    let (tmp_repo, mut file, _) = TmpRepo::new_with_base_commit().unwrap();

    // AI adds line
    file.append("AI Line 1\n").unwrap();
    tmp_repo
        .trigger_checkpoint_with_ai("Claude", Some("claude-3-sonnet"), Some("cursor"))
        .unwrap();

    // Human adds line
    file.append("Human Line 1\n").unwrap();
    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    // AI adds another line
    file.append("AI Line 2\n").unwrap();
    tmp_repo
        .trigger_checkpoint_with_ai("Claude", Some("claude-3-sonnet"), Some("cursor"))
        .unwrap();

    tmp_repo.commit_with_message("Interleaved commit").unwrap();

    let blame = tmp_repo.blame_for_file(&file, None).unwrap();
    assert_debug_snapshot!(blame);
}

#[test]
fn test_simple_ai_then_human_deletion() {
    let tmp_repo = TmpRepo::new().unwrap();

    let mut file = tmp_repo
        .write_file("test.txt", "Line 1\nLine 2\nLine 3\nLine 4\nLine 5\n", true)
        .unwrap();

    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    tmp_repo.commit_with_message("Initial commit").unwrap();

    file.append("AI Line\n").unwrap();

    tmp_repo
        .trigger_checkpoint_with_ai("Claude", Some("claude-3-sonnet"), Some("cursor"))
        .unwrap();

    tmp_repo.commit_with_message("AI adds line").unwrap();

    // Human deletes all AI lines by replacing them with empty
    file.replace_range(6, 7, "").unwrap();

    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    let authorship_log = tmp_repo
        .commit_with_message("Human deletes AI line")
        .unwrap();

    // The authorship log should have no attestations since we only deleted lines
    assert_eq!(authorship_log.attestations.len(), 0);

    assert_debug_snapshot!(authorship_log);
}

#[test]
fn test_multiple_ai_checkpoints_with_human_deletions() {
    let tmp_repo = TmpRepo::new().unwrap();

    let mut file = tmp_repo.write_file("test.txt", "Base\n", true).unwrap();

    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    tmp_repo.commit_with_message("Initial commit").unwrap();

    // First AI session
    file.append("AI1 Line 1\nAI1 Line 2\n").unwrap();
    tmp_repo
        .trigger_checkpoint_with_ai("Claude", Some("claude-3-sonnet"), Some("cursor"))
        .unwrap();

    // Second AI session
    file.append("AI2 Line 1\nAI2 Line 2\n").unwrap();
    tmp_repo
        .trigger_checkpoint_with_ai("GPT-4", Some("gpt-4"), Some("openai"))
        .unwrap();

    // Human deletes first AI session's lines
    file.replace_range(2, 4, "").unwrap();
    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    let authorship_log = tmp_repo.commit_with_message("Complex commit").unwrap();

    // Should only have AI2's lines attributed
    assert_eq!(authorship_log.attestations.len(), 1);
    let file_att = &authorship_log.attestations[0];
    assert_eq!(file_att.entries.len(), 1); // Only one AI session

    assert_debug_snapshot!(authorship_log);
}

#[test]
fn test_complex_mixed_additions_and_deletions() {
    let tmp_repo = TmpRepo::new().unwrap();

    let mut file = tmp_repo
        .write_file(
            "test.txt",
            "Line 1\nLine 2\nLine 3\nLine 4\nLine 5\nLine 6\nLine 7\nLine 8\nLine 9\nLine 10\n",
            true,
        )
        .unwrap();

    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    tmp_repo.commit_with_message("Initial commit").unwrap();

    // AI deletes lines 2-3 and replaces with new content
    file.replace_range(2, 4, "NEW LINE A\nNEW LINE B\nNEW LINE C\n")
        .unwrap();

    tmp_repo
        .trigger_checkpoint_with_ai("Claude", Some("claude-3-sonnet"), Some("cursor"))
        .unwrap();

    // AI inserts at the end
    file.append("END LINE 1\nEND LINE 2\n").unwrap();

    tmp_repo
        .trigger_checkpoint_with_ai("Claude", Some("claude-3-sonnet"), Some("cursor"))
        .unwrap();

    let authorship_log = tmp_repo.commit_with_message("Complex edits").unwrap();

    // Should have lines 2-4 and the last 2 lines attributed to AI
    assert_eq!(authorship_log.attestations.len(), 1);

    assert_debug_snapshot!(authorship_log);
}

#[test]
fn test_ai_adds_lines_with_unstaged_modifications() {
    // Test that unstaged lines are NOT included in the authorship log after commit
    let tmp_repo = TmpRepo::new().unwrap();

    // Start with a base file
    let mut file = tmp_repo.write_file("test.ts", "base_line\n", true).unwrap();

    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    // Create initial commit
    tmp_repo.commit_with_message("Initial commit").unwrap();

    // AI adds 5 lines and we stage them
    tmp_repo
        .append_and_stage_file(
            &mut file,
            "ai_line1\nai_line2\nai_line3\nai_line4\nai_line5\n",
        )
        .unwrap();

    tmp_repo
        .trigger_checkpoint_with_ai("Claude", Some("claude-3-sonnet"), Some("cursor"))
        .unwrap();

    // Now add more AI lines that won't be staged
    tmp_repo
        .append_unstaged_file(&mut file, "unstaged_ai_line1\nunstaged_ai_line2\n")
        .unwrap();

    tmp_repo
        .trigger_checkpoint_with_ai("Claude", Some("claude-3-sonnet"), Some("cursor"))
        .unwrap();

    // Commit only the staged changes
    let authorship_log = tmp_repo
        .commit_staged_with_message("AI adds lines with unstaged modifications")
        .unwrap();

    // The authorship log should only include lines 2-6 (the staged AI lines)
    // Lines 7-8 (unstaged_ai_line1, unstaged_ai_line2) should NOT be in the authorship log
    assert_debug_snapshot!(authorship_log);

    // Verify the blame only includes the committed lines
    let blame = tmp_repo.blame_for_file(&file, Some((1, 6))).unwrap();
    assert_debug_snapshot!(blame);
}

#[test]
fn test_partial_staging_filters_unstaged_lines() {
    // Test where AI makes changes but only some are staged
    let tmp_repo = TmpRepo::new().unwrap();

    // Start with a base file
    let mut file = tmp_repo
        .write_file("partial.ts", "line1\nline2\nline3\n", true)
        .unwrap();

    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    tmp_repo.commit_with_message("Initial commit").unwrap();

    // AI modifies lines 2-3 and stage
    file.replace_range(2, 4, "ai_modified2\nai_modified3\n")
        .unwrap();
    tmp_repo.stage_file("partial.ts").unwrap();

    tmp_repo
        .trigger_checkpoint_with_ai("Claude", Some("claude-3-sonnet"), Some("cursor"))
        .unwrap();

    // Now AI adds more lines that won't be staged
    tmp_repo
        .append_unstaged_file(&mut file, "unstaged_line1\nunstaged_line2\n")
        .unwrap();

    tmp_repo
        .trigger_checkpoint_with_ai("Claude", Some("claude-3-sonnet"), Some("cursor"))
        .unwrap();

    let authorship_log = tmp_repo
        .commit_staged_with_message("Partial staging")
        .unwrap();

    // Should only include lines 2-3 (the modifications), not the unstaged additions
    assert_eq!(authorship_log.attestations.len(), 1);
    let entry = &authorship_log.attestations[0].entries[0];
    assert_eq!(entry.line_ranges, vec![LineRange::Range(2, 3)]);

    assert_debug_snapshot!(authorship_log);
}

#[test]
fn test_human_stages_some_ai_lines() {
    // Test where AI adds multiple lines but human only stages some of them
    let tmp_repo = TmpRepo::new().unwrap();

    let mut file = tmp_repo
        .write_file("test.ts", "line1\nline2\nline3\n", true)
        .unwrap();

    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    tmp_repo.commit_with_message("Initial commit").unwrap();

    // AI adds lines 4-8
    file.append("ai_line4\nai_line5\nai_line6\nai_line7\nai_line8\n")
        .unwrap();

    tmp_repo
        .trigger_checkpoint_with_ai("Claude", Some("claude-3-sonnet"), Some("cursor"))
        .unwrap();

    // Stage only lines 4-6 (first 3 AI lines)
    tmp_repo.stage_lines_from_file(&file, &[(4, 6)]).unwrap();

    // Human adds an unstaged line
    file.append("human_unstaged\n").unwrap();

    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    let authorship_log = tmp_repo
        .commit_staged_with_message("Partial AI commit")
        .unwrap();

    // Should only have lines 4-6 from AI
    assert_eq!(authorship_log.attestations.len(), 1);
    let entry = &authorship_log.attestations[0].entries[0];
    assert_eq!(entry.line_ranges, vec![LineRange::Range(4, 6)]);

    assert_debug_snapshot!(authorship_log);
}

#[test]
fn test_multiple_ai_sessions_with_partial_staging() {
    // Multiple AI sessions, but only one has staged changes
    let tmp_repo = TmpRepo::new().unwrap();

    let mut file = tmp_repo
        .write_file("test.ts", "line1\nline2\n", true)
        .unwrap();

    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    tmp_repo.commit_with_message("Initial commit").unwrap();

    // First AI session adds lines and they get staged
    file.append("ai1_line1\nai1_line2\n").unwrap();

    tmp_repo
        .trigger_checkpoint_with_ai("Claude", Some("claude-3-sonnet"), Some("cursor"))
        .unwrap();

    tmp_repo.stage_file("test.ts").unwrap();

    // Second AI session adds lines but they DON'T get staged
    file.append("ai2_line1\nai2_line2\n").unwrap();

    tmp_repo
        .trigger_checkpoint_with_ai("GPT", Some("gpt-4"), Some("windsurf"))
        .unwrap();

    let authorship_log = tmp_repo
        .commit_staged_with_message("Commit first AI session only")
        .unwrap();

    // Should only have the first AI session's lines
    assert_eq!(authorship_log.attestations.len(), 1);
    assert_eq!(authorship_log.attestations[0].entries.len(), 1);

    // Verify the lines are 3-4
    let entry = &authorship_log.attestations[0].entries[0];
    assert_eq!(entry.line_ranges, vec![LineRange::Range(3, 4)]);

    // Verify the hash corresponds to the first AI session (Claude)
    let hash = &entry.hash;
    let prompt_record = authorship_log.metadata.prompts.get(hash).unwrap();
    assert_eq!(prompt_record.agent_id.tool, "cursor");

    assert_debug_snapshot!(authorship_log);
}

#[test]
fn test_stage_specific_lines_only() {
    // Test staging specific non-contiguous lines
    let tmp_repo = TmpRepo::new().unwrap();

    let mut file = tmp_repo
        .write_file("test.ts", "line1\nline2\nline3\nline4\n", true)
        .unwrap();

    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    tmp_repo.commit_with_message("Initial commit").unwrap();

    // AI adds lines 5-10
    file.append("ai_line5\nai_line6\nai_line7\nai_line8\nai_line9\nai_line10\n")
        .unwrap();

    tmp_repo
        .trigger_checkpoint_with_ai("Claude", Some("claude-3-sonnet"), Some("cursor"))
        .unwrap();

    // Stage only lines 5-7 (contiguous range)
    tmp_repo.stage_lines_from_file(&file, &[(5, 7)]).unwrap();

    let authorship_log = tmp_repo
        .commit_staged_with_message("Commit specific lines")
        .unwrap();

    // Should have lines 5-7
    assert_eq!(authorship_log.attestations.len(), 1);
    let entry = &authorship_log.attestations[0].entries[0];

    let lines: Vec<u32> = entry.line_ranges.iter().flat_map(|r| r.expand()).collect();
    assert_eq!(lines, vec![5, 6, 7]);

    assert_debug_snapshot!(authorship_log);
}

#[test]
fn test_stage_middle_lines_leave_edges_unstaged() {
    // AI adds lines, stage only the middle ones
    let tmp_repo = TmpRepo::new().unwrap();

    let mut file = tmp_repo
        .write_file("test.ts", "line1\nline2\n", true)
        .unwrap();

    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    tmp_repo.commit_with_message("Initial commit").unwrap();

    // AI adds 6 lines (lines 3-8)
    file.append("ai_line3\nai_line4\nai_line5\nai_line6\nai_line7\nai_line8\n")
        .unwrap();

    tmp_repo
        .trigger_checkpoint_with_ai("Claude", Some("claude-3-sonnet"), Some("cursor"))
        .unwrap();

    // Stage lines 3-5 (leave 6, 7, 8 unstaged)
    tmp_repo.stage_lines_from_file(&file, &[(3, 5)]).unwrap();

    let authorship_log = tmp_repo
        .commit_staged_with_message("Commit middle lines")
        .unwrap();

    // Should have lines 3-5
    assert_eq!(authorship_log.attestations.len(), 1);
    let entry = &authorship_log.attestations[0].entries[0];
    assert_eq!(entry.line_ranges, vec![LineRange::Range(3, 5)]);

    assert_debug_snapshot!(authorship_log);
}

#[test]
fn test_multiple_ai_sessions_with_line_level_staging() {
    // Multiple AI sessions with line-level staging
    let tmp_repo = TmpRepo::new().unwrap();

    let mut file = tmp_repo
        .write_file("test.ts", "line1\nline2\n", true)
        .unwrap();

    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    tmp_repo.commit_with_message("Initial commit").unwrap();

    // First AI session adds lines 3-5
    file.append("ai1_line3\nai1_line4\nai1_line5\n").unwrap();

    tmp_repo
        .trigger_checkpoint_with_ai("Claude", Some("claude-3-sonnet"), Some("cursor"))
        .unwrap();

    // Second AI session adds lines 6-8
    file.append("ai2_line6\nai2_line7\nai2_line8\n").unwrap();

    tmp_repo
        .trigger_checkpoint_with_ai("GPT", Some("gpt-4"), Some("windsurf"))
        .unwrap();

    // Stage lines 3-5 from first session (will capture all of AI1's changes)
    tmp_repo.stage_lines_from_file(&file, &[(3, 5)]).unwrap();

    let authorship_log = tmp_repo
        .commit_staged_with_message("First AI session only")
        .unwrap();

    // Should have 1 entry for the first AI session
    assert_eq!(authorship_log.attestations.len(), 1);
    assert_eq!(authorship_log.attestations[0].entries.len(), 1);

    // Verify it's attributed to Claude (first session)
    let hash = &authorship_log.attestations[0].entries[0].hash;
    let prompt_record = authorship_log.metadata.prompts.get(hash).unwrap();
    assert_eq!(prompt_record.agent_id.tool, "cursor");

    assert_debug_snapshot!(authorship_log);
}

#[test]
fn test_interleaved_staged_unstaged_hunks() {
    // Complex case with interleaved staged/unstaged content
    let tmp_repo = TmpRepo::new().unwrap();

    let mut file = tmp_repo
        .write_file("test.ts", "line1\nline2\nline3\nline4\nline5\n", true)
        .unwrap();

    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    tmp_repo.commit_with_message("Initial commit").unwrap();

    // AI modifies/adds throughout the file
    file.replace_range(2, 3, "ai_modified_line2\n").unwrap();
    file.replace_range(4, 5, "ai_modified_line4\n").unwrap();
    file.append("ai_line6\nai_line7\nai_line8\n").unwrap();

    tmp_repo
        .trigger_checkpoint_with_ai("Claude", Some("claude-3-sonnet"), Some("cursor"))
        .unwrap();

    // Stage lines 2 and 6-7 (leave line 4 and 8 unstaged)
    tmp_repo
        .stage_lines_from_file(&file, &[(2, 2), (6, 7)])
        .unwrap();

    let authorship_log = tmp_repo
        .commit_staged_with_message("Interleaved staging")
        .unwrap();

    // Should have lines 2, 6, 7
    assert_eq!(authorship_log.attestations.len(), 1);
    let file_att = &authorship_log.attestations[0];
    assert!(!file_att.entries.is_empty());

    // Verify line 5 is NOT in the ranges
    for entry in &file_att.entries {
        for range in &entry.line_ranges {
            match range {
                LineRange::Single(l) => {
                    assert_ne!(*l, 5, "Line 5 should not be attributed (it was unstaged)");
                }
                LineRange::Range(start, end) => {
                    if *start <= 5 && *end >= 5 {
                        panic!("Line 5 should not be in any range (it was unstaged)");
                    }
                }
            }
        }
    }

    assert_debug_snapshot!(authorship_log);

    let blame = tmp_repo.blame_for_file(&file, Some((1, 8))).unwrap();
    assert_debug_snapshot!(blame);
}

#[test]
fn test_unstaged_ai_lines_saved_to_working_log() {
    // Test that unstaged AI-authored lines are saved to the working log for the next commit
    let tmp_repo = TmpRepo::new().unwrap();

    let mut file = tmp_repo
        .write_file("test.ts", "line1\nline2\nline3\n", true)
        .unwrap();

    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    tmp_repo.commit_with_message("Initial commit").unwrap();

    // AI adds lines 4-7
    file.append("ai_line4\nai_line5\nai_line6\nai_line7\n")
        .unwrap();

    tmp_repo
        .trigger_checkpoint_with_ai("Claude", Some("claude-3-sonnet"), Some("cursor"))
        .unwrap();

    // Stage only lines 4-5, leaving 6-7 unstaged
    tmp_repo.stage_lines_from_file(&file, &[(4, 5)]).unwrap();

    // Commit only the staged lines
    let authorship_log = tmp_repo
        .commit_staged_with_message("Partial AI commit")
        .unwrap();

    // The authorship log should only have lines 4-5
    assert_eq!(authorship_log.attestations.len(), 1);
    let file_att = &authorship_log.attestations[0];
    assert_eq!(file_att.entries.len(), 1);

    // Lines 6-7 should have been saved to the working log
    // Let's verify by staging them and committing
    tmp_repo.stage_lines_from_file(&file, &[(6, 7)]).unwrap();

    let second_authorship_log = tmp_repo
        .commit_staged_with_message("Commit remaining AI lines")
        .unwrap();

    // The second commit should also attribute lines 6-7 to the AI
    // because they were saved to the working log
    assert_eq!(second_authorship_log.attestations.len(), 1);
    let second_file_att = &second_authorship_log.attestations[0];
    assert!(!second_file_att.entries.is_empty());

    // Verify the same AI session hash is used (from working log)
    let first_hash = &file_att.entries[0].hash;
    let second_hash = &second_file_att.entries[0].hash;
    assert_eq!(
        first_hash, second_hash,
        "Both commits should be attributed to the same AI session"
    );

    assert_debug_snapshot!((authorship_log, second_authorship_log));
}

/// Test: New file with partial staging across two commits
/// AI creates a new file with many lines, stage only some, then commit the rest
#[test]
fn test_new_file_partial_staging_two_commits() {
    let tmp_repo = TmpRepo::new().unwrap();
    tmp_repo.commit_with_message("Initial commit").unwrap();

    // AI creates a brand new file with a long list
    let mut file = tmp_repo
        .write_file(
            "planets.txt",
            "Mercury\nVenus\nEarth\nMars\nJupiter\nSaturn\nUranus\nNeptune\nPluto (dwarf)\n",
            false, // Don't stage yet
        )
        .unwrap();

    tmp_repo
        .trigger_checkpoint_with_ai("Claude", Some("gpt-4"), Some("cursor"))
        .unwrap();

    // Stage only the first 5 lines (inner planets + Jupiter)
    tmp_repo.stage_lines_from_file(&file, &[(1, 5)]).unwrap();

    // First commit with partial staging
    let log1 = tmp_repo
        .commit_staged_with_message("Add inner planets and Jupiter")
        .unwrap();

    // Check first commit's authorship log
    assert_eq!(log1.attestations.len(), 1);
    assert_eq!(log1.attestations[0].file_path, "planets.txt");

    // Should have lines 1-5
    let entry1 = &log1.attestations[0].entries[0];
    assert_eq!(entry1.line_ranges, vec![LineRange::Range(1, 5)]);

    // Now stage the remaining lines
    tmp_repo.stage_file("planets.txt").unwrap();

    // Second commit
    let log2 = tmp_repo
        .commit_staged_with_message("Add outer planets")
        .unwrap();

    // Check second commit's authorship log
    assert_eq!(log2.attestations.len(), 1);

    // Should have lines 6-9 (the previously unstaged lines)
    let entry2 = &log2.attestations[0].entries[0];
    assert_eq!(entry2.line_ranges, vec![LineRange::Range(6, 9)]);

    // Verify both commits use the same AI session hash
    assert_eq!(entry1.hash, entry2.hash);

    // Verify with blame
    let blame1 = tmp_repo.blame_for_file(&file, Some((1, 5))).unwrap();
    let blame2 = tmp_repo.blame_for_file(&file, Some((6, 9))).unwrap();

    assert_debug_snapshot!((log1, log2, blame1, blame2));
}
