#[macro_use]
mod repos;
use repos::test_file::ExpectedLineExt;
use repos::test_repo::TestRepo;

// Helper function to extract author names from blame output
fn extract_authors(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| {
            // Extract author name from blame line format
            // Format: sha (author date line) code
            if let Some(start) = line.find('(') {
                if let Some(end) = line[start..].find(' ') {
                    Some(line[start + 1..start + end].trim().to_string())
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect()
}

// Helper function to normalize blame output for comparison
// This replaces author names with consistent placeholders to avoid drift from author names
fn normalize_for_snapshot(output: &str) -> String {
    output
        .lines()
        .map(|line| {
            // Handle porcelain format lines
            if line.starts_with("author-mail") || line.starts_with("committer-mail") {
                // Keep these lines as-is for porcelain format
                line.to_string()
            } else if line.starts_with("author ") || line.starts_with("committer ") {
                // Keep author/committer lines as-is for porcelain format
                line.to_string()
            } else if line.starts_with("author-time")
                || line.starts_with("author-tz")
                || line.starts_with("committer-time")
                || line.starts_with("committer-tz")
            {
                // Keep time/tz lines as-is for porcelain format
                line.to_string()
            } else if line.starts_with("summary")
                || line.starts_with("boundary")
                || line.starts_with("filename")
            {
                // Keep metadata lines as-is for porcelain format
                line.to_string()
            } else if line.starts_with('\t') {
                // Keep content lines (starting with tab) as-is for porcelain format
                line.to_string()
            } else if let Some(start) = line.find('(') {
                if let Some(end) = line[start..].find(')') {
                    // Replace the entire author/date/line section with a consistent placeholder
                    let before = &line[..start + 1];
                    let after = &line[start + end..];
                    format!("{}<AUTHOR_INFO>{}", before, after)
                } else {
                    line.to_string()
                }
            } else {
                line.to_string()
            }
        })
        .map(|line| {
            // Remove the ^ prefix that git adds for boundary commits
            if line.starts_with('^') {
                line[1..].to_string()
            } else {
                line
            }
        })
        .map(|line| {
            // Only normalize hash length for lines that look like blame output (start with hash)
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                let first_part = parts[0];
                // Only apply hash normalization if the first part looks like a hash (hex chars)
                if first_part.chars().all(|c| c.is_ascii_hexdigit()) && first_part.len() >= 7 {
                    let rest = &parts[1..];
                    // Truncate hash to 7 characters for consistent comparison (git blame default)
                    let normalized_hash = if first_part.len() > 7 {
                        &first_part[..7]
                    } else {
                        first_part
                    };
                    format!("{} {}", normalized_hash, rest.join(" "))
                } else {
                    line
                }
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn test_blame_basic_format() {
    let repo = TestRepo::new();
    let mut file = repo.filename("test.txt");

    file.set_contents(lines!["Line 1", "Line 2", "Line 3".ai(), "Line 4".ai()]);

    repo.stage_all_and_commit("Initial commit").unwrap();

    // Run git blame and git-ai blame
    let git_output = repo.git(&["blame", "test.txt"]).unwrap();
    let git_ai_output = repo.git_ai(&["blame", "test.txt"]).unwrap();

    // Compare normalized outputs
    let git_norm = normalize_for_snapshot(&git_output);
    let git_ai_norm = normalize_for_snapshot(&git_ai_output);
    println!("\n[DEBUG] Normalized git blame output:\n{}", git_norm);
    println!("\n[DEBUG] Normalized git-ai blame output:\n{}", git_ai_norm);
    assert_eq!(
        git_norm, git_ai_norm,
        "Normalized blame outputs should match exactly"
    );
}

#[test]
fn test_blame_line_range() {
    let repo = TestRepo::new();
    let mut file = repo.filename("test.txt");

    file.set_contents(lines![
        "Line 1",
        "Line 2",
        "Line 3",
        "Line 4",
        "Line 5".ai(),
        "Line 6".ai()
    ]);

    repo.stage_all_and_commit("Initial commit").unwrap();

    // Test -L flag
    let git_output = repo.git(&["blame", "-L", "2,4", "test.txt"]).unwrap();
    let git_ai_output = repo.git_ai(&["blame", "-L", "2,4", "test.txt"]).unwrap();

    let git_norm = normalize_for_snapshot(&git_output);
    let git_ai_norm = normalize_for_snapshot(&git_ai_output);
    println!("\n[DEBUG] Normalized git blame output:\n{}", git_norm);
    println!("\n[DEBUG] Normalized git-ai blame output:\n{}", git_ai_norm);
    assert_eq!(
        git_norm, git_ai_norm,
        "Normalized blame outputs should match exactly"
    );
}

#[test]
fn test_blame_porcelain_format() {
    let repo = TestRepo::new();
    let mut file = repo.filename("test.txt");

    file.set_contents(lines!["Line 1", "Line 2".ai()]);

    repo.stage_all_and_commit("Initial commit").unwrap();

    let git_output = repo.git(&["blame", "--porcelain", "test.txt"]).unwrap();
    let git_ai_output = repo.git_ai(&["blame", "--porcelain", "test.txt"]).unwrap();

    let git_norm = normalize_for_snapshot(&git_output);
    let git_ai_norm = normalize_for_snapshot(&git_ai_output);
    println!("\n[DEBUG] Normalized git blame output:\n{}", git_norm);
    println!("\n[DEBUG] Normalized git-ai blame output:\n{}", git_ai_norm);
    assert_eq!(
        git_norm, git_ai_norm,
        "Normalized blame outputs should match exactly"
    );
}

#[test]
fn test_blame_show_email() {
    let repo = TestRepo::new();
    let mut file = repo.filename("test.txt");

    file.set_contents(lines!["Line 1", "Line 2".ai()]);

    repo.stage_all_and_commit("Initial commit").unwrap();

    let git_output = repo.git(&["blame", "-e", "test.txt"]).unwrap();
    let git_ai_output = repo.git_ai(&["blame", "-e", "test.txt"]).unwrap();

    let git_norm = normalize_for_snapshot(&git_output);
    let git_ai_norm = normalize_for_snapshot(&git_ai_output);
    println!("\n[DEBUG] Normalized git blame output:\n{}", git_norm);
    println!("\n[DEBUG] Normalized git-ai blame output:\n{}", git_ai_norm);
    assert_eq!(
        git_norm, git_ai_norm,
        "Normalized blame outputs should match exactly"
    );

    // Verify both contain email addresses
    assert!(git_output.contains("@"), "Git output should contain email");
    assert!(
        git_ai_output.contains("@"),
        "Git-ai output should contain email"
    );
}

#[test]
fn test_blame_show_name() {
    let repo = TestRepo::new();
    let mut file = repo.filename("test.txt");

    file.set_contents(lines!["Line 1", "Line 2".ai()]);

    repo.stage_all_and_commit("Initial commit").unwrap();

    let git_output = repo.git(&["blame", "-f", "test.txt"]).unwrap();
    let git_ai_output = repo.git_ai(&["blame", "-f", "test.txt"]).unwrap();

    let git_norm = normalize_for_snapshot(&git_output);
    let git_ai_norm = normalize_for_snapshot(&git_ai_output);
    println!("\n[DEBUG] Normalized git blame output:\n{}", git_norm);
    println!("\n[DEBUG] Normalized git-ai blame output:\n{}", git_ai_norm);
    assert_eq!(
        git_norm, git_ai_norm,
        "Normalized blame outputs should match exactly"
    );

    // Verify both contain filename information
    assert!(
        git_output.contains("test.txt"),
        "Git output should contain filename"
    );
    assert!(
        git_ai_output.contains("test.txt"),
        "Git-ai output should contain filename"
    );
}

#[test]
fn test_blame_show_number() {
    let repo = TestRepo::new();
    let mut file = repo.filename("test.txt");

    file.set_contents(lines!["Line 1", "Line 2".ai()]);

    repo.stage_all_and_commit("Initial commit").unwrap();

    let git_output = repo.git(&["blame", "-n", "test.txt"]).unwrap();
    let git_ai_output = repo.git_ai(&["blame", "-n", "test.txt"]).unwrap();

    let git_norm = normalize_for_snapshot(&git_output);
    let git_ai_norm = normalize_for_snapshot(&git_ai_output);
    println!("\n[DEBUG] Normalized git blame output:\n{}", git_norm);
    println!("\n[DEBUG] Normalized git-ai blame output:\n{}", git_ai_norm);
    assert_eq!(
        git_norm, git_ai_norm,
        "Normalized blame outputs should match exactly"
    );
}

#[test]
fn test_blame_suppress_author() {
    let repo = TestRepo::new();
    let mut file = repo.filename("test.txt");

    file.set_contents(lines!["Line 1", "Line 2".ai()]);

    repo.stage_all_and_commit("Initial commit").unwrap();

    let git_output = repo.git(&["blame", "-s", "test.txt"]).unwrap();
    let git_ai_output = repo.git_ai(&["blame", "-s", "test.txt"]).unwrap();

    let git_norm = normalize_for_snapshot(&git_output);
    let git_ai_norm = normalize_for_snapshot(&git_ai_output);
    println!("\n[DEBUG] Normalized git blame output:\n{}", git_norm);
    println!("\n[DEBUG] Normalized git-ai blame output:\n{}", git_ai_norm);
    assert_eq!(
        git_norm, git_ai_norm,
        "Normalized blame outputs should match exactly"
    );

    // Verify both suppress author information (should not contain "Test User")
    assert!(
        !git_output.contains("Test User"),
        "Git output should suppress author"
    );
    assert!(
        !git_ai_output.contains("Test User"),
        "Git-ai output should suppress author"
    );
}

#[test]
fn test_blame_long_rev() {
    let repo = TestRepo::new();
    let mut file = repo.filename("test.txt");

    file.set_contents(lines!["Line 1", "Line 2".ai()]);

    repo.stage_all_and_commit("Initial commit").unwrap();

    let git_output = repo.git(&["blame", "-l", "test.txt"]).unwrap();
    let git_ai_output = repo.git_ai(&["blame", "-l", "test.txt"]).unwrap();

    let git_norm = normalize_for_snapshot(&git_output);
    let git_ai_norm = normalize_for_snapshot(&git_ai_output);
    println!("\n[DEBUG] Normalized git blame output:\n{}", git_norm);
    println!("\n[DEBUG] Normalized git-ai blame output:\n{}", git_ai_norm);
    assert_eq!(
        git_norm, git_ai_norm,
        "Normalized blame outputs should match exactly"
    );

    // Verify both show long revision hashes
    let git_sha_len = git_output
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .len();
    let git_ai_sha_len = git_ai_output
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .len();

    assert!(git_sha_len > 8, "Git should show long revision");
    assert!(git_ai_sha_len > 8, "Git-ai should show long revision");
}

#[test]
fn test_blame_raw_timestamp() {
    let repo = TestRepo::new();
    let mut file = repo.filename("test.txt");

    file.set_contents(lines!["Line 1", "Line 2".ai()]);

    repo.stage_all_and_commit("Initial commit").unwrap();

    let git_output = repo.git(&["blame", "-t", "test.txt"]).unwrap();
    let git_ai_output = repo.git_ai(&["blame", "-t", "test.txt"]).unwrap();

    let git_norm = normalize_for_snapshot(&git_output);
    let git_ai_norm = normalize_for_snapshot(&git_ai_output);
    println!("\n[DEBUG] Normalized git blame output:\n{}", git_norm);
    println!("\n[DEBUG] Normalized git-ai blame output:\n{}", git_ai_norm);
    assert_eq!(
        git_norm, git_ai_norm,
        "Normalized blame outputs should match exactly"
    );

    // Verify both contain raw timestamps (Unix timestamps)
    assert!(
        git_output.chars().any(|c| c.is_numeric()),
        "Git output should contain timestamps"
    );
    assert!(
        git_ai_output.chars().any(|c| c.is_numeric()),
        "Git-ai output should contain timestamps"
    );
}

#[test]
fn test_blame_abbrev() {
    let repo = TestRepo::new();
    let mut file = repo.filename("test.txt");

    file.set_contents(lines!["Line 1", "Line 2".ai()]);

    repo.stage_all_and_commit("Initial commit").unwrap();

    // Note: git requires --abbrev=4 format, git-ai accepts --abbrev 4
    let git_output = repo.git(&["blame", "--abbrev=4", "test.txt"]).unwrap();
    let git_ai_output = repo
        .git_ai(&["blame", "--abbrev", "4", "test.txt"])
        .unwrap();

    let git_norm = normalize_for_snapshot(&git_output);
    let git_ai_norm = normalize_for_snapshot(&git_ai_output);
    println!("\n[DEBUG] Normalized git blame output:\n{}", git_norm);
    println!("\n[DEBUG] Normalized git-ai blame output:\n{}", git_ai_norm);
    assert_eq!(
        git_norm, git_ai_norm,
        "Normalized blame outputs should match exactly"
    );
}

#[test]
fn test_blame_blank_boundary() {
    let repo = TestRepo::new();
    let mut file = repo.filename("test.txt");

    file.set_contents(lines!["Line 1", "Line 2".ai()]);

    repo.stage_all_and_commit("Initial commit").unwrap();

    let git_output = repo.git(&["blame", "-b", "test.txt"]).unwrap();
    let git_ai_output = repo.git_ai(&["blame", "-b", "test.txt"]).unwrap();

    let git_norm = normalize_for_snapshot(&git_output);
    let git_ai_norm = normalize_for_snapshot(&git_ai_output);
    println!("\n[DEBUG] Normalized git blame output:\n{}", git_norm);
    println!("\n[DEBUG] Normalized git-ai blame output:\n{}", git_ai_norm);
    assert_eq!(
        git_norm, git_ai_norm,
        "Normalized blame outputs should match exactly"
    );
}

#[test]
fn test_blame_show_root() {
    let repo = TestRepo::new();
    let mut file = repo.filename("test.txt");

    file.set_contents(lines!["Line 1", "Line 2".ai()]);

    repo.stage_all_and_commit("Initial commit").unwrap();

    let git_output = repo.git(&["blame", "--root", "test.txt"]).unwrap();
    let git_ai_output = repo.git_ai(&["blame", "--root", "test.txt"]).unwrap();

    let git_norm = normalize_for_snapshot(&git_output);
    let git_ai_norm = normalize_for_snapshot(&git_ai_output);
    println!("\n[DEBUG] Normalized git blame output:\n{}", git_norm);
    println!("\n[DEBUG] Normalized git-ai blame output:\n{}", git_ai_norm);
    assert_eq!(
        git_norm, git_ai_norm,
        "Normalized blame outputs should match exactly"
    );

    // Verify both handle root commits
    assert!(
        git_output.lines().count() > 0,
        "Git should handle root commits"
    );
    assert!(
        git_ai_output.lines().count() > 0,
        "Git-ai should handle root commits"
    );
}

// #[test]
// fn test_blame_show_stats() {
//     let tmp_dir = tempdir().unwrap();
//     let repo_path = tmp_dir.path().to_path_buf();

//     let tmp_repo = TmpRepo::new().unwrap();
//     let mut file = tmp_repo.write_file("test.txt", "Line 1\n", true).unwrap();

//     tmp_repo
//         .trigger_checkpoint_with_author("test_user")
//         .unwrap();
//     file.append("Line 2\n").unwrap();
//     tmp_repo.trigger_checkpoint_with_ai("Claude", Some("claude-3-sonnet"), Some("cursor")).unwrap();
//     tmp_repo.commit_with_message("Initial commit").unwrap();

//     let git_output = run_git_blame(tmp_repo.path(), "test.txt", &["--show-stats"]);
//     let git_ai_output = run_git_ai_blame(tmp_repo.path(), "test.txt", &["--show-stats"]);

//     let _comparison = create_blame_comparison(&git_output, &git_ai_output, "show_stats");
//     let git_norm = normalize_for_snapshot(&git_output);
//     let git_ai_norm = normalize_for_snapshot(&git_ai_output);
//     println!("\n[DEBUG] Normalized git blame output:\n{}", git_norm);
//     println!("\n[DEBUG] Normalized git-ai blame output:\n{}", git_ai_norm);
//     assert_eq!(
//         git_norm, git_ai_norm,
//         "Normalized blame outputs should match exactly"
//     );

//     // Verify both show statistics
//     assert!(
//         git_output.contains("%"),
//         "Git output should contain statistics"
//     );
//     assert!(
//         git_ai_output.contains("%"),
//         "Git-ai output should contain statistics"
//     );
// }
#[test]
fn test_blame_date_format() {
    let repo = TestRepo::new();
    let mut file = repo.filename("test.txt");

    file.set_contents(lines!["Line 1", "Line 2".ai()]);

    repo.stage_all_and_commit("Initial commit").unwrap();

    // Note: git requires --date=short format, git-ai accepts --date short
    let git_output = repo.git(&["blame", "--date=short", "test.txt"]).unwrap();
    let git_ai_output = repo
        .git_ai(&["blame", "--date", "short", "test.txt"])
        .unwrap();

    let git_norm = normalize_for_snapshot(&git_output);
    let git_ai_norm = normalize_for_snapshot(&git_ai_output);
    println!("\n[DEBUG] Normalized git blame output:\n{}", git_norm);
    println!("\n[DEBUG] Normalized git-ai blame output:\n{}", git_ai_norm);
    assert_eq!(
        git_norm, git_ai_norm,
        "Normalized blame outputs should match exactly"
    );

    // Verify both use short date format
    assert!(git_output.contains("-"), "Git output should contain date");
    assert!(
        git_ai_output.contains("-"),
        "Git-ai output should contain date"
    );
}

#[test]
fn test_blame_multiple_flags() {
    let repo = TestRepo::new();
    let mut file = repo.filename("test.txt");

    file.set_contents(lines![
        "Line 1",
        "Line 2",
        "Line 3",
        "Line 4".ai(),
        "Line 5".ai()
    ]);

    repo.stage_all_and_commit("Initial commit").unwrap();

    // Test multiple flags together
    let git_output = repo
        .git(&["blame", "-L", "2,4", "-e", "-n", "test.txt"])
        .unwrap();
    let git_ai_output = repo
        .git_ai(&["blame", "-L", "2,4", "-e", "-n", "test.txt"])
        .unwrap();

    let git_norm = normalize_for_snapshot(&git_output);
    let git_ai_norm = normalize_for_snapshot(&git_ai_output);
    println!("\n[DEBUG] Normalized git blame output:\n{}", git_norm);
    println!("\n[DEBUG] Normalized git-ai blame output:\n{}", git_ai_norm);
    assert_eq!(
        git_norm, git_ai_norm,
        "Normalized blame outputs should match exactly"
    );

    // Verify both handle multiple flags
    assert!(
        git_output.lines().count() > 0,
        "Git should handle multiple flags"
    );
    assert!(
        git_ai_output.lines().count() > 0,
        "Git-ai should handle multiple flags"
    );

    // Verify both contain email and line numbers
    assert!(git_output.contains("@"), "Git output should contain email");
    assert!(
        git_ai_output.contains("@"),
        "Git-ai output should contain email"
    );
}

#[test]
fn test_blame_incremental_format() {
    let repo = TestRepo::new();
    let mut file = repo.filename("test.txt");

    file.set_contents(lines!["Line 1", "Line 2".ai()]);

    repo.stage_all_and_commit("Initial commit").unwrap();

    let git_output = repo.git(&["blame", "--incremental", "test.txt"]).unwrap();
    let git_ai_output = repo
        .git_ai(&["blame", "--incremental", "test.txt"])
        .unwrap();

    let git_norm = normalize_for_snapshot(&git_output);
    let git_ai_norm = normalize_for_snapshot(&git_ai_output);
    println!("\n[DEBUG] Normalized git blame output:\n{}", git_norm);
    println!("\n[DEBUG] Normalized git-ai blame output:\n{}", git_ai_norm);
    assert_eq!(
        git_norm, git_ai_norm,
        "Normalized blame outputs should match exactly"
    );
}

#[test]
fn test_blame_line_porcelain() {
    let repo = TestRepo::new();
    let mut file = repo.filename("test.txt");

    file.set_contents(lines!["Line 1", "Line 2".ai()]);

    repo.stage_all_and_commit("Initial commit").unwrap();

    let git_output = repo
        .git(&["blame", "--line-porcelain", "test.txt"])
        .unwrap();
    let git_ai_output = repo
        .git_ai(&["blame", "--line-porcelain", "test.txt"])
        .unwrap();

    let git_norm = normalize_for_snapshot(&git_output);
    let git_ai_norm = normalize_for_snapshot(&git_ai_output);
    println!("\n[DEBUG] Normalized git blame output:\n{}", git_norm);
    println!("\n[DEBUG] Normalized git-ai blame output:\n{}", git_ai_norm);
    assert_eq!(
        git_norm, git_ai_norm,
        "Normalized blame outputs should match exactly"
    );
}

#[test]
fn test_blame_with_ai_authorship() {
    let repo = TestRepo::new();
    let mut file = repo.filename("test.txt");

    file.set_contents(lines!["Line 1", "Line 2", "Line 3".ai(), "Line 4"]);

    repo.stage_all_and_commit("Mixed authorship commit")
        .unwrap();

    let git_output = repo.git(&["blame", "test.txt"]).unwrap();
    let git_ai_output = repo.git_ai(&["blame", "test.txt"]).unwrap();

    let git_norm = normalize_for_snapshot(&git_output);
    let git_ai_norm = normalize_for_snapshot(&git_ai_output);
    println!("\n[DEBUG] Normalized git blame output:\n{}", git_norm);
    println!("\n[DEBUG] Normalized git-ai blame output:\n{}", git_ai_norm);
    assert_eq!(
        git_norm, git_ai_norm,
        "Normalized blame outputs should match exactly"
    );

    // Extract authors from both outputs
    let git_authors = extract_authors(&git_output);
    let git_ai_authors = extract_authors(&git_ai_output);

    // Git should show the same author for all lines (the committer)
    // Git-ai should show different authors based on AI authorship
    assert_ne!(
        git_authors, git_ai_authors,
        "AI authorship should change the output"
    );

    // Verify git-ai shows AI authors where appropriate
    assert!(
        git_ai_authors
            .iter()
            .any(|a| a.contains("some-ai") || a.contains("mock_ai")),
        "Should show AI as author. Got: {:?}",
        git_ai_authors
    );
}
