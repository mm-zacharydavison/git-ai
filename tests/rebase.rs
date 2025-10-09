use git_ai::authorship::rebase_authorship::rewrite_authorship_after_rebase;
use git_ai::git::refs::get_reference_as_authorship_log_v3;
use git_ai::git::test_utils::TmpRepo;

/// Test simple rebase with no conflicts where trees are identical - multiple commits
#[test]
fn test_rebase_no_conflicts_identical_trees() {
    let tmp_repo = TmpRepo::new().unwrap();

    // Create initial commit (on default branch, usually master)
    tmp_repo
        .write_file("main.txt", "main line 1\nmain line 2\n", true)
        .unwrap();
    tmp_repo.trigger_checkpoint_with_author("human").unwrap();
    tmp_repo.commit_with_message("Initial commit").unwrap();

    // Get the default branch name
    let default_branch = tmp_repo.current_branch().unwrap();

    // Create feature branch with multiple AI commits
    tmp_repo.create_branch("feature").unwrap();

    // First AI commit
    tmp_repo
        .write_file(
            "feature1.txt",
            "// AI generated feature 1\nfeature line 1\n",
            true,
        )
        .unwrap();
    tmp_repo
        .trigger_checkpoint_with_ai("ai_agent", Some("gpt-4"), Some("cursor"))
        .unwrap();
    tmp_repo.commit_with_message("AI feature 1").unwrap();
    let feature_commit_1 = tmp_repo.get_head_commit_sha().unwrap();

    // Second AI commit
    tmp_repo
        .write_file(
            "feature2.txt",
            "// AI generated feature 2\nfeature line 2\n",
            true,
        )
        .unwrap();
    tmp_repo
        .trigger_checkpoint_with_ai("ai_agent", Some("claude"), Some("cursor"))
        .unwrap();
    tmp_repo.commit_with_message("AI feature 2").unwrap();
    let feature_commit_2 = tmp_repo.get_head_commit_sha().unwrap();

    // Advance default branch (non-conflicting)
    tmp_repo.checkout_branch(&default_branch).unwrap();
    tmp_repo
        .write_file("other.txt", "other content\n", true)
        .unwrap();
    tmp_repo.trigger_checkpoint_with_author("human").unwrap();
    tmp_repo.commit_with_message("Main advances").unwrap();

    // Rebase feature onto default branch
    tmp_repo.checkout_branch("feature").unwrap();
    tmp_repo
        .rebase_onto(&default_branch, &default_branch)
        .unwrap();

    // Get rebased commits
    let head = tmp_repo.get_head_commit_sha().unwrap();
    let repo = tmp_repo.gitai_repo();
    let mut rebased_commits = vec![];
    let mut current = repo.find_commit(head).unwrap();
    for _ in 0..2 {
        rebased_commits.push(current.id().to_string());
        current = current.parent(0).unwrap();
    }
    rebased_commits.reverse();

    // Run rewrite
    rewrite_authorship_after_rebase(
        &repo,
        &[feature_commit_1, feature_commit_2],
        &rebased_commits,
        "Test User <test@example.com>",
    )
    .unwrap();

    // Verify authorship logs were copied for both commits
    for rebased_commit in &rebased_commits {
        let authorship_log = get_reference_as_authorship_log_v3(&repo, rebased_commit).unwrap();
        assert_eq!(authorship_log.metadata.base_commit_sha, *rebased_commit);
        assert!(!authorship_log.attestations.is_empty());
    }
}

/// Test rebase where trees differ (parent changes result in different tree IDs) - multiple commits
#[test]
fn test_rebase_with_different_trees() {
    let tmp_repo = TmpRepo::new().unwrap();

    // Create initial commit
    tmp_repo
        .write_file("base.txt", "base content\n", true)
        .unwrap();
    tmp_repo.trigger_checkpoint_with_author("human").unwrap();
    tmp_repo.commit_with_message("Initial commit").unwrap();

    // Get default branch name
    let default_branch = tmp_repo.current_branch().unwrap();

    // Create feature branch with multiple AI commits
    tmp_repo.create_branch("feature").unwrap();

    // First AI commit
    tmp_repo
        .write_file("feature1.txt", "// AI added feature 1\n", true)
        .unwrap();
    tmp_repo
        .trigger_checkpoint_with_ai("ai_agent", Some("gpt-4"), Some("cursor"))
        .unwrap();
    tmp_repo.commit_with_message("AI changes 1").unwrap();
    let feature_commit_1 = tmp_repo.get_head_commit_sha().unwrap();

    // Second AI commit
    tmp_repo
        .write_file("feature2.txt", "// AI added feature 2\n", true)
        .unwrap();
    tmp_repo
        .trigger_checkpoint_with_ai("ai_agent", Some("claude"), Some("cursor"))
        .unwrap();
    tmp_repo.commit_with_message("AI changes 2").unwrap();
    let feature_commit_2 = tmp_repo.get_head_commit_sha().unwrap();

    // Go back to default branch and add a different file (non-conflicting)
    tmp_repo.checkout_branch(&default_branch).unwrap();
    tmp_repo
        .write_file("main.txt", "main content\n", true)
        .unwrap();
    tmp_repo.trigger_checkpoint_with_author("human").unwrap();
    tmp_repo.commit_with_message("Main changes").unwrap();

    // Rebase feature onto default branch (no conflicts, but trees will differ)
    tmp_repo.checkout_branch("feature").unwrap();
    tmp_repo
        .rebase_onto(&default_branch, &default_branch)
        .unwrap();

    // Get rebased commits
    let head = tmp_repo.get_head_commit_sha().unwrap();
    let repo = tmp_repo.gitai_repo();
    let mut rebased_commits = vec![];
    let mut current = repo.find_commit(head).unwrap();
    for _ in 0..2 {
        rebased_commits.push(current.id().to_string());
        current = current.parent(0).unwrap();
    }
    rebased_commits.reverse();

    // Run rewrite
    rewrite_authorship_after_rebase(
        &repo,
        &[feature_commit_1, feature_commit_2],
        &rebased_commits,
        "Test User <test@example.com>",
    )
    .unwrap();

    // Verify authorship log exists and is correct for both commits
    for rebased_commit in &rebased_commits {
        let result = get_reference_as_authorship_log_v3(&repo, rebased_commit);
        assert!(result.is_ok());

        let log = result.unwrap();
        assert_eq!(log.metadata.base_commit_sha, *rebased_commit);
        assert!(!log.attestations.is_empty());
    }
}

/// Test rebase with multiple commits
#[test]
fn test_rebase_multiple_commits() {
    let tmp_repo = TmpRepo::new().unwrap();

    // Create initial commit
    tmp_repo
        .write_file("main.txt", "main content\n", true)
        .unwrap();
    tmp_repo.trigger_checkpoint_with_author("human").unwrap();
    tmp_repo.commit_with_message("Initial").unwrap();

    // Get default branch name
    let default_branch = tmp_repo.current_branch().unwrap();

    // Create feature branch with multiple commits
    tmp_repo.create_branch("feature").unwrap();

    // First AI commit
    tmp_repo
        .write_file("feature1.txt", "// AI feature 1\n", true)
        .unwrap();
    tmp_repo
        .trigger_checkpoint_with_ai("ai_agent_1", Some("gpt-4"), Some("cursor"))
        .unwrap();
    tmp_repo.commit_with_message("AI feature 1").unwrap();
    let feature_commit_1 = tmp_repo.get_head_commit_sha().unwrap();

    // Second AI commit
    tmp_repo
        .write_file("feature2.txt", "// AI feature 2\n", true)
        .unwrap();
    tmp_repo
        .trigger_checkpoint_with_ai("ai_agent_2", Some("claude"), Some("cursor"))
        .unwrap();
    tmp_repo.commit_with_message("AI feature 2").unwrap();
    let feature_commit_2 = tmp_repo.get_head_commit_sha().unwrap();

    // Third AI commit
    tmp_repo
        .write_file("feature3.txt", "// AI feature 3\n", true)
        .unwrap();
    tmp_repo
        .trigger_checkpoint_with_ai("ai_agent_3", Some("gpt-4"), Some("cursor"))
        .unwrap();
    tmp_repo.commit_with_message("AI feature 3").unwrap();
    let feature_commit_3 = tmp_repo.get_head_commit_sha().unwrap();

    // Advance default branch
    tmp_repo.checkout_branch(&default_branch).unwrap();
    tmp_repo
        .write_file("main2.txt", "more main content\n", true)
        .unwrap();
    tmp_repo.trigger_checkpoint_with_author("human").unwrap();
    tmp_repo.commit_with_message("Main advances").unwrap();

    // Rebase feature onto default branch
    tmp_repo.checkout_branch("feature").unwrap();
    tmp_repo
        .rebase_onto(&default_branch, &default_branch)
        .unwrap();

    // Get the rebased commits (walk back 3 commits from HEAD)
    let head = tmp_repo.get_head_commit_sha().unwrap();
    let repo = tmp_repo.gitai_repo();
    let mut rebased_commits = vec![];
    let mut current = repo.find_commit(head).unwrap();
    for _ in 0..3 {
        rebased_commits.push(current.id().to_string());
        current = current.parent(0).unwrap();
    }
    rebased_commits.reverse(); // oldest first

    let original_commits = vec![
        feature_commit_1.clone(),
        feature_commit_2.clone(),
        feature_commit_3.clone(),
    ];

    // Run rewrite
    rewrite_authorship_after_rebase(
        &repo,
        &original_commits,
        &rebased_commits,
        "Test User <test@example.com>",
    )
    .unwrap();

    // Verify all commits have authorship logs
    for rebased_commit in &rebased_commits {
        let result = get_reference_as_authorship_log_v3(&repo, rebased_commit);
        assert!(
            result.is_ok(),
            "Authorship log should exist for {}",
            rebased_commit
        );
    }
}

/// Test rebase where only some commits have authorship logs
#[test]
fn test_rebase_mixed_authorship() {
    let tmp_repo = TmpRepo::new().unwrap();

    // Create initial commit
    tmp_repo
        .write_file("main.txt", "main content\n", true)
        .unwrap();
    tmp_repo.trigger_checkpoint_with_author("human").unwrap();
    tmp_repo.commit_with_message("Initial").unwrap();

    // Get default branch name
    let default_branch = tmp_repo.current_branch().unwrap();

    // Create feature branch
    tmp_repo.create_branch("feature").unwrap();

    // Human commit (no AI authorship)
    tmp_repo
        .write_file("human.txt", "human work\n", true)
        .unwrap();
    tmp_repo.trigger_checkpoint_with_author("human").unwrap();
    tmp_repo.commit_with_message("Human work").unwrap();
    let human_commit = tmp_repo.get_head_commit_sha().unwrap();

    // AI commit
    tmp_repo.write_file("ai.txt", "// AI work\n", true).unwrap();
    tmp_repo
        .trigger_checkpoint_with_ai("ai_agent", Some("gpt-4"), Some("cursor"))
        .unwrap();
    tmp_repo.commit_with_message("AI work").unwrap();
    let ai_commit = tmp_repo.get_head_commit_sha().unwrap();

    // Advance default branch
    tmp_repo.checkout_branch(&default_branch).unwrap();
    tmp_repo
        .write_file("main2.txt", "more main\n", true)
        .unwrap();
    tmp_repo.trigger_checkpoint_with_author("human").unwrap();
    tmp_repo.commit_with_message("Main advances").unwrap();

    // Rebase feature onto default branch
    tmp_repo.checkout_branch("feature").unwrap();
    tmp_repo
        .rebase_onto(&default_branch, &default_branch)
        .unwrap();

    // Get rebased commits
    let head = tmp_repo.get_head_commit_sha().unwrap();
    let repo = tmp_repo.gitai_repo();
    let mut rebased_commits = vec![];
    let mut current = repo.find_commit(head).unwrap();
    for _ in 0..2 {
        rebased_commits.push(current.id().to_string());
        current = current.parent(0).unwrap();
    }
    rebased_commits.reverse();

    // Run rewrite
    rewrite_authorship_after_rebase(
        &repo,
        &[human_commit, ai_commit],
        &rebased_commits,
        "Test User <test@example.com>",
    )
    .unwrap();

    // Verify AI commit has authorship log
    let ai_result = get_reference_as_authorship_log_v3(&repo, &rebased_commits[1]);
    assert!(ai_result.is_ok());

    // Human commit might not have authorship log (that's ok)
    // The function should handle this gracefully
}

/// Test empty rebase (fast-forward)
#[test]
fn test_rebase_fast_forward() {
    let tmp_repo = TmpRepo::new().unwrap();

    // Create initial commit
    tmp_repo
        .write_file("main.txt", "main content\n", true)
        .unwrap();
    tmp_repo.trigger_checkpoint_with_author("human").unwrap();
    tmp_repo.commit_with_message("Initial").unwrap();

    // Get default branch name
    let default_branch = tmp_repo.current_branch().unwrap();

    // Create feature branch
    tmp_repo.create_branch("feature").unwrap();

    // Add commit on feature
    tmp_repo
        .write_file("feature.txt", "// AI feature\n", true)
        .unwrap();
    tmp_repo
        .trigger_checkpoint_with_ai("ai_agent", Some("gpt-4"), Some("cursor"))
        .unwrap();
    tmp_repo.commit_with_message("AI feature").unwrap();
    let feature_commit = tmp_repo.get_head_commit_sha().unwrap();

    // Rebase onto default branch (should be fast-forward, no changes)
    tmp_repo
        .rebase_onto(&default_branch, &default_branch)
        .unwrap();
    let after_rebase = tmp_repo.get_head_commit_sha().unwrap();

    // In a fast-forward, the commit SHA stays the same
    // Call rewrite anyway to verify it handles this gracefully (shouldn't crash)
    rewrite_authorship_after_rebase(
        &tmp_repo.gitai_repo(),
        &[feature_commit.clone()],
        &[after_rebase.clone()],
        "Test User <test@example.com>",
    )
    .unwrap();

    // Verify authorship log still exists
    let result = get_reference_as_authorship_log_v3(&tmp_repo.gitai_repo(), &after_rebase);
    assert!(
        result.is_ok(),
        "Authorship should exist even in fast-forward case"
    );
}

/// Test interactive rebase with commit reordering - verifies interactive rebase works
#[test]
fn test_rebase_interactive_reorder() {
    let tmp_repo = TmpRepo::new().unwrap();

    // Create initial commit
    tmp_repo
        .write_file("base.txt", "base content\n", true)
        .unwrap();
    tmp_repo.trigger_checkpoint_with_author("human").unwrap();
    tmp_repo.commit_with_message("Initial commit").unwrap();

    let default_branch = tmp_repo.current_branch().unwrap();
    tmp_repo.create_branch("feature").unwrap();

    // Create 2 AI commits - we'll rebase these interactively
    tmp_repo
        .write_file("feature1.txt", "// AI feature 1\n", true)
        .unwrap();
    tmp_repo
        .trigger_checkpoint_with_ai("ai_agent_1", Some("gpt-4"), Some("cursor"))
        .unwrap();
    tmp_repo.commit_with_message("AI commit 1").unwrap();
    let commit1 = tmp_repo.get_head_commit_sha().unwrap();

    tmp_repo
        .write_file("feature2.txt", "// AI feature 2\n", true)
        .unwrap();
    tmp_repo
        .trigger_checkpoint_with_ai("ai_agent_2", Some("claude"), Some("cursor"))
        .unwrap();
    tmp_repo.commit_with_message("AI commit 2").unwrap();
    let commit2 = tmp_repo.get_head_commit_sha().unwrap();

    // Advance main branch
    tmp_repo.checkout_branch(&default_branch).unwrap();
    tmp_repo
        .write_file("main.txt", "main work\n", true)
        .unwrap();
    tmp_repo.trigger_checkpoint_with_author("human").unwrap();
    tmp_repo.commit_with_message("Main advances").unwrap();
    let base_commit = tmp_repo.get_head_commit_sha().unwrap();

    // Perform interactive rebase (just pick all, tests that -i flag works)
    tmp_repo.checkout_branch("feature").unwrap();

    use std::process::Command;
    let output = Command::new("git")
        .current_dir(tmp_repo.path())
        .env("GIT_SEQUENCE_EDITOR", "true") // Just accept the default picks
        .env("GIT_EDITOR", "true") // Auto-accept commit messages
        .args(&["rebase", "-i", &base_commit])
        .output()
        .unwrap();

    if !output.status.success() {
        eprintln!(
            "git rebase output: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        panic!("Interactive rebase failed");
    }

    // Get the rebased commits
    let head = tmp_repo.get_head_commit_sha().unwrap();
    let repo = tmp_repo.gitai_repo();
    let mut rebased_commits = vec![];
    let mut current = repo.find_commit(head).unwrap();
    for _ in 0..2 {
        rebased_commits.push(current.id().to_string());
        current = current.parent(0).unwrap();
    }
    rebased_commits.reverse();

    // Rewrite authorship for the rebased commits
    rewrite_authorship_after_rebase(
        &repo,
        &[commit1, commit2],
        &rebased_commits,
        "Test User <test@example.com>",
    )
    .unwrap();

    // Verify both commits have authorship
    for rebased_commit in &rebased_commits {
        let result = get_reference_as_authorship_log_v3(&repo, rebased_commit);
        assert!(
            result.is_ok(),
            "Interactive rebased commit should have authorship"
        );

        let log = result.unwrap();
        assert!(!log.attestations.is_empty(), "Should have AI attestations");
    }
}

/// Test rebase with conflicts - verifies reconstruction works after conflict resolution
#[test]
fn test_rebase_with_conflicts() {
    let tmp_repo = TmpRepo::new().unwrap();

    // Create initial commit with a file
    tmp_repo
        .write_file("conflict.txt", "line 1\nline 2\nline 3\n", true)
        .unwrap();
    tmp_repo.trigger_checkpoint_with_author("human").unwrap();
    tmp_repo.commit_with_message("Initial commit").unwrap();

    let default_branch = tmp_repo.current_branch().unwrap();

    // Create feature branch with AI changes
    tmp_repo.create_branch("feature").unwrap();
    tmp_repo
        .write_file("conflict.txt", "line 1\nAI FEATURE\nline 3\n", false)
        .unwrap();
    tmp_repo
        .trigger_checkpoint_with_ai("ai_agent", Some("gpt-4"), Some("cursor"))
        .unwrap();
    tmp_repo.commit_with_message("AI changes line 2").unwrap();
    let feature_commit = tmp_repo.get_head_commit_sha().unwrap();

    // Add second AI commit
    tmp_repo
        .write_file("feature2.txt", "// AI feature 2\n", true)
        .unwrap();
    tmp_repo
        .trigger_checkpoint_with_ai("ai_agent", Some("gpt-4"), Some("cursor"))
        .unwrap();
    tmp_repo.commit_with_message("AI adds feature2").unwrap();
    let feature_commit_2 = tmp_repo.get_head_commit_sha().unwrap();

    // Go back to main and make conflicting change to the same line
    tmp_repo.checkout_branch(&default_branch).unwrap();
    tmp_repo
        .write_file("conflict.txt", "line 1\nMAIN CHANGE\nline 3\n", false)
        .unwrap();
    tmp_repo.trigger_checkpoint_with_author("human").unwrap();
    tmp_repo.commit_with_message("Main changes line 2").unwrap();

    // Try to rebase - will conflict
    tmp_repo.checkout_branch("feature").unwrap();

    use std::process::Command;
    let output = Command::new("git")
        .current_dir(tmp_repo.path())
        .args(&["rebase", &default_branch])
        .output()
        .unwrap();

    // Should have a conflict
    assert!(!output.status.success(), "Rebase should conflict");

    // Resolve conflict - keep AI's version
    tmp_repo
        .write_file("conflict.txt", "line 1\nAI FEATURE\nline 3\n", false)
        .unwrap();

    // Stage the resolved file
    Command::new("git")
        .current_dir(tmp_repo.path())
        .args(&["add", "conflict.txt"])
        .output()
        .unwrap();

    // Continue rebase with a commit message (non-interactive)
    let output = Command::new("git")
        .current_dir(tmp_repo.path())
        .env("GIT_EDITOR", "true") // Auto-accept commit message
        .args(&["rebase", "--continue"])
        .output()
        .unwrap();

    if !output.status.success() {
        eprintln!(
            "rebase --continue failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        panic!("Rebase --continue failed");
    }

    // Get the rebased commits
    let head = tmp_repo.get_head_commit_sha().unwrap();
    let repo = tmp_repo.gitai_repo();
    let mut rebased_commits = vec![];
    let mut current = repo.find_commit(head).unwrap();
    for _ in 0..2 {
        rebased_commits.push(current.id().to_string());
        current = current.parent(0).unwrap();
    }
    rebased_commits.reverse();

    // Run rewrite authorship
    rewrite_authorship_after_rebase(
        &repo,
        &[feature_commit, feature_commit_2],
        &rebased_commits,
        "Test User <test@example.com>",
    )
    .unwrap();

    // Verify authorship was reconstructed despite conflicts
    for rebased_commit in &rebased_commits {
        let result = get_reference_as_authorship_log_v3(&repo, rebased_commit);
        assert!(
            result.is_ok(),
            "Authorship should be reconstructed even after conflict resolution"
        );

        let log = result.unwrap();
        assert!(!log.attestations.is_empty());
    }
}

/// Test rebase with commit splitting (fewer original commits than new commits)
/// This tests the bug fix where zip() would truncate and lose authorship for extra commits
#[test]
fn test_rebase_commit_splitting() {
    let tmp_repo = TmpRepo::new().unwrap();

    // Create initial commit
    tmp_repo
        .write_file("base.txt", "base content\n", true)
        .unwrap();
    tmp_repo.trigger_checkpoint_with_author("human").unwrap();
    tmp_repo.commit_with_message("Initial commit").unwrap();

    let default_branch = tmp_repo.current_branch().unwrap();

    // Create feature branch with 2 AI commits that modify the same file
    tmp_repo.create_branch("feature").unwrap();

    // First AI commit - adds initial content to features.txt
    tmp_repo
        .write_file(
            "features.txt",
            "// AI feature 1\nfunction feature1() {}\n",
            true,
        )
        .unwrap();
    tmp_repo
        .trigger_checkpoint_with_ai("ai_agent_1", Some("gpt-4"), Some("cursor"))
        .unwrap();
    tmp_repo.commit_with_message("AI feature 1").unwrap();
    let original_commit_1 = tmp_repo.get_head_commit_sha().unwrap();

    // Second AI commit - adds more content to the same file
    tmp_repo
        .write_file(
            "features.txt",
            "// AI feature 1\nfunction feature1() {}\n// AI feature 2\nfunction feature2() {}\n",
            false,
        )
        .unwrap();
    tmp_repo
        .trigger_checkpoint_with_ai("ai_agent_2", Some("claude"), Some("cursor"))
        .unwrap();
    tmp_repo.commit_with_message("AI feature 2").unwrap();
    let original_commit_2 = tmp_repo.get_head_commit_sha().unwrap();

    // Advance main branch
    tmp_repo.checkout_branch(&default_branch).unwrap();
    tmp_repo
        .write_file("main.txt", "main content\n", true)
        .unwrap();
    tmp_repo.trigger_checkpoint_with_author("human").unwrap();
    tmp_repo.commit_with_message("Main advances").unwrap();
    let main_head = tmp_repo.get_head_commit_sha().unwrap();

    // Simulate commit splitting by manually creating 3 new commits that represent
    // the rebased and split versions of the original 2 commits
    // Use git commands directly to checkout the commit (create detached HEAD)
    use std::process::Command;
    let output = Command::new("git")
        .current_dir(tmp_repo.path())
        .args(&["checkout", &main_head])
        .output()
        .unwrap();

    if !output.status.success() {
        panic!(
            "Failed to checkout commit: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // New commit 1 (partial content from original - feature1 only)
    tmp_repo
        .write_file(
            "features.txt",
            "// AI feature 1\nfunction feature1() {}\n",
            true,
        )
        .unwrap();
    tmp_repo.trigger_checkpoint_with_author("human").unwrap(); // Don't add AI authorship yet
    tmp_repo.commit_with_message("Add feature 1").unwrap();
    let new_commit_1 = tmp_repo.get_head_commit_sha().unwrap();

    // New commit 2 (adds a helper function that wasn't in original - "splitting" the work)
    tmp_repo
        .write_file(
            "features.txt",
            "// AI feature 1\nfunction feature1() {}\n// Helper\nfunction helper() {}\n",
            false,
        )
        .unwrap();
    tmp_repo.trigger_checkpoint_with_author("human").unwrap();
    tmp_repo.commit_with_message("Add helper").unwrap();
    let new_commit_2 = tmp_repo.get_head_commit_sha().unwrap();

    // New commit 3 (adds feature2 - from original commit 2)
    tmp_repo
        .write_file("features.txt", "// AI feature 1\nfunction feature1() {}\n// Helper\nfunction helper() {}\n// AI feature 2\nfunction feature2() {}\n", false)
        .unwrap();
    tmp_repo.trigger_checkpoint_with_author("human").unwrap();
    tmp_repo.commit_with_message("Add feature 2").unwrap();
    let new_commit_3 = tmp_repo.get_head_commit_sha().unwrap();

    // Now test the authorship rewriting with 2 original commits -> 3 new commits
    // This is the scenario that would have failed with the zip() bug
    let repo = tmp_repo.gitai_repo();
    let original_commits = vec![original_commit_1, original_commit_2];
    let new_commits = vec![
        new_commit_1.clone(),
        new_commit_2.clone(),
        new_commit_3.clone(),
    ];

    // Run rewrite authorship - this should handle all 3 new commits
    rewrite_authorship_after_rebase(
        &repo,
        &original_commits,
        &new_commits,
        "Test User <test@example.com>",
    )
    .unwrap();

    // Verify ALL 3 new commits have authorship logs
    // With the bug, only the first 2 would have been processed (due to zip truncation)
    for (i, new_commit) in new_commits.iter().enumerate() {
        let result = get_reference_as_authorship_log_v3(&repo, new_commit);
        assert!(
            result.is_ok(),
            "New commit {} at index {} should have authorship log (bug: zip truncation would skip this)",
            new_commit,
            i
        );

        let log = result.unwrap();
        assert_eq!(
            log.metadata.base_commit_sha, *new_commit,
            "Authorship log should reference the correct commit"
        );
    }

    // Additional verification: ensure the 3rd commit (which would have been skipped by the bug)
    // actually has authorship attribution
    let log_3 = get_reference_as_authorship_log_v3(&repo, &new_commits[2]).unwrap();
    assert_eq!(
        log_3.metadata.base_commit_sha, new_commits[2],
        "Third commit should have proper authorship log"
    );
}

/// Test interactive rebase with squashing - verifies authorship from all commits is preserved
/// This tests the bug fix where only the last commit's authorship was kept during squashing
#[test]
fn test_rebase_squash_preserves_all_authorship() {
    let tmp_repo = TmpRepo::new().unwrap();

    // Create initial commit
    tmp_repo
        .write_file("base.txt", "base content\n", true)
        .unwrap();
    tmp_repo.trigger_checkpoint_with_author("human").unwrap();
    tmp_repo.commit_with_message("Initial commit").unwrap();

    let default_branch = tmp_repo.current_branch().unwrap();
    tmp_repo.create_branch("feature").unwrap();

    // Create 3 AI commits with different content - we'll squash these
    tmp_repo
        .write_file("feature1.txt", "// AI feature 1\nline 1\n", true)
        .unwrap();
    tmp_repo
        .trigger_checkpoint_with_ai("ai_agent_1", Some("gpt-4"), Some("cursor"))
        .unwrap();
    tmp_repo.commit_with_message("AI commit 1").unwrap();
    let commit1 = tmp_repo.get_head_commit_sha().unwrap();

    tmp_repo
        .write_file("feature2.txt", "// AI feature 2\nline 2\n", true)
        .unwrap();
    tmp_repo
        .trigger_checkpoint_with_ai("ai_agent_2", Some("claude"), Some("cursor"))
        .unwrap();
    tmp_repo.commit_with_message("AI commit 2").unwrap();
    let commit2 = tmp_repo.get_head_commit_sha().unwrap();

    tmp_repo
        .write_file("feature3.txt", "// AI feature 3\nline 3\n", true)
        .unwrap();
    tmp_repo
        .trigger_checkpoint_with_ai("ai_agent_3", Some("gpt-4"), Some("cursor"))
        .unwrap();
    tmp_repo.commit_with_message("AI commit 3").unwrap();
    let commit3 = tmp_repo.get_head_commit_sha().unwrap();

    // Advance main branch
    tmp_repo.checkout_branch(&default_branch).unwrap();
    tmp_repo
        .write_file("main.txt", "main work\n", true)
        .unwrap();
    tmp_repo.trigger_checkpoint_with_author("human").unwrap();
    tmp_repo.commit_with_message("Main advances").unwrap();
    let base_commit = tmp_repo.get_head_commit_sha().unwrap();

    // Perform interactive rebase with squashing: pick first, squash second and third
    tmp_repo.checkout_branch("feature").unwrap();

    use std::io::Write;
    use std::process::Command;

    // Create a script that modifies the rebase-todo to squash commits 2 and 3 into 1
    let script_content = r#"#!/bin/sh
sed -i.bak '2s/pick/squash/' "$1"
sed -i.bak '3s/pick/squash/' "$1"
"#;

    let script_path = tmp_repo.path().join("squash_script.sh");
    let mut script_file = std::fs::File::create(&script_path).unwrap();
    script_file.write_all(script_content.as_bytes()).unwrap();
    drop(script_file);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script_path, perms).unwrap();
    }

    let output = Command::new("git")
        .current_dir(tmp_repo.path())
        .env("GIT_SEQUENCE_EDITOR", script_path.to_str().unwrap())
        .env("GIT_EDITOR", "true") // Auto-accept commit message
        .args(&["rebase", "-i", &base_commit])
        .output()
        .unwrap();

    if !output.status.success() {
        eprintln!(
            "git rebase output: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        panic!("Interactive rebase with squash failed");
    }

    // After squashing, we should have only 1 commit on top of base
    let head = tmp_repo.get_head_commit_sha().unwrap();
    let repo = tmp_repo.gitai_repo();

    // Verify we have exactly 1 commit (the squashed one)
    let squashed_commit = head.clone();
    let parent = repo.find_commit(head).unwrap().parent(0).unwrap();
    assert_eq!(
        parent.id().to_string(),
        base_commit,
        "Should have exactly 1 commit after squashing 3 commits"
    );

    // Now rewrite authorship: 3 original commits -> 1 new commit
    rewrite_authorship_after_rebase(
        &repo,
        &[commit1, commit2, commit3],
        &[squashed_commit.clone()],
        "Test User <test@example.com>",
    )
    .unwrap();

    // Verify the squashed commit has authorship
    let result = get_reference_as_authorship_log_v3(&repo, &squashed_commit);
    assert!(
        result.is_ok(),
        "Squashed commit should have authorship from all original commits"
    );

    let log = result.unwrap();
    assert!(
        !log.attestations.is_empty(),
        "Squashed commit should have AI attestations"
    );

    // Verify all 3 files exist (proving all commits were included)
    assert!(
        tmp_repo.path().join("feature1.txt").exists(),
        "feature1.txt from commit 1 should exist"
    );
    assert!(
        tmp_repo.path().join("feature2.txt").exists(),
        "feature2.txt from commit 2 should exist"
    );
    assert!(
        tmp_repo.path().join("feature3.txt").exists(),
        "feature3.txt from commit 3 should exist"
    );
}
