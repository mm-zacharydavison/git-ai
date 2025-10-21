use crate::repos::test_file::ExpectedLineExt;
use crate::lines;
use super::github_test_harness::{GitHubTestRepo, MergeStrategy};

#[test]
#[ignore] // Ignored by default - run with `cargo test --test github_integration -- --ignored`
fn test_merge_pr_with_mixed_authorship() {
    let test_repo = match GitHubTestRepo::new("test_merge_pr_with_mixed_authorship") {
        Some(repo) => repo,
        None => {
            println!("⏭️  Test skipped - GitHub CLI not available");
            return;
        }
    };

    println!("🚀 Starting merge PR test with mixed human+AI authorship");

    if let Err(e) = test_repo.create_on_github() {
        panic!("Failed to create GitHub repository: {}", e);
    }

    println!("📦 Installing GitHub Action workflow (should skip on merge commits)");
    test_repo.install_github_action()
        .expect("Failed to install GitHub Action");

    test_repo.commit_and_push_workflow()
        .expect("Failed to commit and push workflow");

    test_repo.create_branch("feature/basic-test")
        .expect("Failed to create feature branch");

    std::fs::create_dir(test_repo.repo.path().join("src"))
        .expect("Failed to create src directory");

    let mut test_file = test_repo.repo.filename("src/main.rs");
    test_file.set_contents(lines![
        "fn main() {",
        "    println!(\"Hello, world!\");".ai(),
        "}",
    ]);

    test_repo.repo.stage_all_and_commit("Add basic main function")
        .expect("Failed to create commit");

    test_file.insert_at(2, lines![
        "    // AI-generated greeting".ai(),
        "    println!(\"Welcome to git-ai!\");".ai(),
    ]);

    test_repo.repo.stage_all_and_commit("AI adds greeting")
        .expect("Failed to create AI commit");

    test_repo.push_branch("feature/basic-test")
        .expect("Failed to push branch");

    let pr_url = test_repo.create_pr(
        "Merge mixed authorship test",
        "Testing merge human + AI authorship tracking"
    ).expect("Failed to create PR");

    println!("✅ Pull request created: {}", pr_url);

    let pr_number = test_repo.extract_pr_number(&pr_url)
        .expect("Failed to extract PR number");

    test_repo.merge_pr(&pr_number, MergeStrategy::Merge)
        .expect("Failed to merge PR");

    println!("⏳ Waiting for GitHub Action to complete (should skip merge commits)...");
    match test_repo.wait_for_workflow_completion(120) {
        Ok(run_id) => {
            println!("✅ GitHub Action completed (run ID: {})", run_id);
            println!("   Note: Workflow should have skipped authorship processing for merge commit");
        }
        Err(e) => {
            eprintln!("⚠️  Warning: GitHub Action workflow issue: {}", e);
        }
    }

    test_repo.checkout_and_pull_default_branch()
        .expect("Failed to checkout and pull main branch");

    println!("✅ Test completed successfully");

    test_file.assert_lines_and_blame(lines![
        "fn main() {".human(),
        "    println!(\"Hello, world!\");".ai(),
        "    // AI-generated greeting".ai(),
        "    println!(\"Welcome to git-ai!\");".ai(),
        "}".human(),
    ]);
}
