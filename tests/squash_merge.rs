#[macro_use]
mod repos;
use repos::test_file::ExpectedLineExt;
use repos::test_repo::TestRepo;

/// Test merge --squash with a simple feature branch containing AI and human edits
#[test]
fn test_prepare_working_log_simple_squash() {
    let repo = TestRepo::new();
    let mut file = repo.filename("main.txt");

    // Create master branch with initial content
    file.set_contents(lines!["line 1", "line 2", "line 3"]);
    repo.stage_all_and_commit("Initial commit on master")
        .unwrap();

    let default_branch = repo.current_branch();

    // Create feature branch
    repo.git(&["checkout", "-b", "feature"]).unwrap();

    // Add AI changes on feature branch
    file.insert_at(3, lines!["// AI added feature".ai()]);
    repo.stage_all_and_commit("Add AI feature").unwrap();

    // Add human changes on feature branch
    file.insert_at(4, lines!["// Human refinement"]);
    repo.stage_all_and_commit("Human refinement").unwrap();

    // Go back to master and squash merge
    repo.git(&["checkout", &default_branch]).unwrap();
    repo.git(&["merge", "--squash", "feature"]).unwrap();
    repo.commit("Squashed feature").unwrap();

    // Verify AI attribution is preserved
    file.assert_lines_and_blame(lines![
        "line 1".human(),
        "line 2".human(),
        "line 3".human(),
        "// AI added feature".ai(),
        "// Human refinement".human()
    ]);

    // Verify stats for squashed commit
    let stats = repo.stats().unwrap();
    assert_eq!(
        stats.git_diff_added_lines, 3,
        "Squash commit adds 3 lines total (includes newline)"
    );
    assert_eq!(stats.ai_additions, 1, "1 AI line from feature branch");
    assert_eq!(stats.ai_accepted, 1, "1 AI line accepted without edits");
    assert_eq!(
        stats.human_additions, 2,
        "2 human lines from feature branch"
    );
    assert_eq!(stats.mixed_additions, 0, "No mixed edits");
}

/// Test merge --squash with out-of-band changes on master (handles 3-way merge)
#[test]
fn test_prepare_working_log_squash_with_main_changes() {
    let repo = TestRepo::new();
    let mut file = repo.filename("document.txt");

    // Create master branch with initial content
    file.set_contents(lines!["section 1", "section 2", "section 3"]);
    repo.stage_all_and_commit("Initial commit").unwrap();

    let default_branch = repo.current_branch();

    // Create feature branch and add AI changes
    repo.git(&["checkout", "-b", "feature"]).unwrap();
    file.insert_at(3, lines!["// AI feature addition at end".ai()]);
    repo.stage_all_and_commit("AI adds feature").unwrap();

    // Switch back to master and make out-of-band changes
    repo.git(&["checkout", &default_branch]).unwrap();

    // Re-initialize file after checkout to get current master state
    let mut file = repo.filename("document.txt");
    file.insert_at(0, lines!["// Master update at top"]);
    repo.stage_all_and_commit("Out-of-band update on master")
        .unwrap();

    // Squash merge feature into master
    repo.git(&["merge", "--squash", "feature"]).unwrap();
    repo.stage_all_and_commit("Squashed feature with out-of-band")
        .unwrap();

    // Verify both changes are present with correct attribution
    file.assert_lines_and_blame(lines![
        "// Master update at top".human(),
        "section 1".human(),
        "section 2".human(),
        "section 3".human(),
        "// AI feature addition at end".ai()
    ]);

    // Verify stats for squashed commit
    let stats = repo.stats().unwrap();
    assert_eq!(
        stats.git_diff_added_lines, 2,
        "Squash commit adds 2 lines from feature (includes newline)"
    );
    assert_eq!(stats.ai_additions, 1, "1 AI line from feature branch");
    assert_eq!(stats.ai_accepted, 1, "1 AI line accepted without edits");
    assert_eq!(stats.human_additions, 1, "1 human line from feature branch");
    assert_eq!(stats.mixed_additions, 0, "No mixed edits");
}

/// Test merge --squash with multiple AI sessions and human edits
#[test]
fn test_prepare_working_log_squash_multiple_sessions() {
    let repo = TestRepo::new();
    let mut file = repo.filename("file.txt");

    // Create master branch
    file.set_contents(lines!["header", "body", "footer"]);
    repo.stage_all_and_commit("Initial").unwrap();

    let default_branch = repo.current_branch();

    // Create feature branch
    repo.git(&["checkout", "-b", "feature"]).unwrap();

    // First AI session
    file.insert_at(1, lines!["// AI session 1".ai()]);
    repo.stage_all_and_commit("AI session 1").unwrap();

    // Human edit
    file.insert_at(3, lines!["// Human addition"]);
    repo.stage_all_and_commit("Human edit").unwrap();

    // Second AI session (different agent - simulated by new checkpoint)
    file.insert_at(5, lines!["// AI session 2".ai()]);
    repo.stage_all_and_commit("AI session 2").unwrap();

    // Squash merge into master
    repo.git(&["checkout", &default_branch]).unwrap();
    repo.git(&["merge", "--squash", "feature"]).unwrap();
    repo.commit("Squashed multiple sessions").unwrap();

    // Verify all authorship is preserved
    file.assert_lines_and_blame(lines![
        "header".human(),
        "// AI session 1".ai(),
        "body".human(),
        "// Human addition".human(),
        "footer".human(),
        "// AI session 2".ai()
    ]);

    // Verify stats for squashed commit with multiple sessions
    let stats = repo.stats().unwrap();
    assert_eq!(
        stats.git_diff_added_lines, 4,
        "Squash commit adds 4 lines total (includes newline)"
    );
    assert_eq!(
        stats.ai_additions, 2,
        "2 AI lines from feature branch (both sessions)"
    );
    assert_eq!(stats.ai_accepted, 2, "2 AI lines accepted without edits");
    assert_eq!(
        stats.human_additions, 2,
        "2 human lines from feature branch"
    );
    assert_eq!(stats.mixed_additions, 0, "No mixed edits");
}

/// Test merge --squash with mixed additions (AI code edited by human before commit)
#[test]
fn test_prepare_working_log_squash_with_mixed_additions() {
    let repo = TestRepo::new();
    let mut file = repo.filename("code.txt");

    // Create master branch with initial content
    file.set_contents(lines!["function start() {", "  // initial code", "}"]);
    repo.stage_all_and_commit("Initial commit").unwrap();

    let default_branch = repo.current_branch();

    // Create feature branch
    repo.git(&["checkout", "-b", "feature"]).unwrap();

    // AI adds 3 lines (without committing)
    file.insert_at(
        2,
        lines![
            "  const x = 1;".ai(),
            "  const y = 2;".ai(),
            "  const z = 3;".ai()
        ],
    );

    // Human immediately edits the middle AI line (before committing)
    // This creates a "mixed addition" - AI generated, human edited
    file.replace_at(3, "  const y = 20; // human modified");

    // Now commit with both AI and human changes together
    repo.stage_all_and_commit("AI adds variables, human refines")
        .unwrap();

    // Squash merge back to master
    repo.git(&["checkout", &default_branch]).unwrap();
    repo.git(&["merge", "--squash", "feature"]).unwrap();
    repo.commit("Squashed feature with mixed edits").unwrap();

    // Verify attribution - edited line should be human
    file.assert_lines_and_blame(lines![
        "function start() {".human(),
        "  // initial code".human(),
        "  const x = 1;".ai(),
        "  const y = 20; // human modified".human(), // Human edited AI line
        "  const z = 3;".ai(),
        "}".human()
    ]);

    // Verify stats show mixed additions
    let stats = repo.stats().unwrap();
    assert_eq!(
        stats.git_diff_added_lines, 3,
        "Squash commit adds 3 lines total"
    );
    assert_eq!(stats.ai_additions, 3, "3 AI lines total (2 pure + 1 mixed)");
    assert_eq!(stats.ai_accepted, 2, "2 AI lines accepted without edits");
    assert_eq!(
        stats.mixed_additions, 1,
        "1 AI line was edited by human before commit"
    );
    assert_eq!(
        stats.human_additions, 1,
        "1 human addition (the overridden AI line)"
    );
}
