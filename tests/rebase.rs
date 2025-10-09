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
