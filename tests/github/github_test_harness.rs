use std::process::Command;
use std::sync::OnceLock;
use crate::repos::test_repo::TestRepo;

/// Merge strategy for pull requests
#[derive(Debug, Clone, Copy)]
pub enum MergeStrategy {
    /// Squash all commits into one
    Squash,
    /// Create a merge commit
    Merge,
    /// Rebase and merge
    Rebase,
}

static GH_CLI_AVAILABLE: OnceLock<bool> = OnceLock::new();

/// Check if GitHub CLI is available and authenticated
pub fn is_gh_cli_available() -> bool {
    *GH_CLI_AVAILABLE.get_or_init(|| {
        let version_check = Command::new("gh")
            .arg("--version")
            .output();

        if version_check.is_err() || !version_check.unwrap().status.success() {
            return false;
        }

        let auth_check = Command::new("gh")
            .args(&["auth", "status"])
            .output();

        auth_check.is_ok() && auth_check.unwrap().status.success()
    })
}

/// GitHub test repository wrapper that extends TestRepo with GitHub operations
pub struct GitHubTestRepo {
    pub repo: TestRepo,
    pub github_repo_name: String,
    pub github_owner: String,
}

impl GitHubTestRepo {
    /// Create a new GitHub test repository with a name derived from the test
    /// Returns None if gh CLI is not available
    pub fn new(test_name: &str) -> Option<Self> {
        if !is_gh_cli_available() {
            println!("⏭️  Skipping GitHub test - gh CLI not available or not authenticated");
            return None;
        }

        let repo = TestRepo::new();
        let repo_name = generate_repo_name(test_name);

        let owner = match get_authenticated_user() {
            Some(user) => user,
            None => {
                println!("⏭️  Skipping GitHub test - could not get authenticated user");
                return None;
            }
        };

        Some(Self {
            repo,
            github_repo_name: repo_name,
            github_owner: owner,
        })
    }

    /// Initialize the repository and create it on GitHub
    pub fn create_on_github(&self) -> Result<(), String> {
        let repo_path = self.repo.path();

        // Create initial commit (required for gh repo create)
        std::fs::write(repo_path.join("README.md"), "# GitHub Test Repository\n")
            .map_err(|e| format!("Failed to create README: {}", e))?;

        self.repo.git(&["add", "."])
            .map_err(|e| format!("Failed to add files: {}", e))?;

        self.repo.git(&["commit", "-m", "Initial commit"])
            .map_err(|e| format!("Failed to create initial commit: {}", e))?;

        // Create GitHub repository
        let output = Command::new("gh")
            .args(&[
                "repo", "create",
                &self.github_repo_name,
                "--public",
                "--source", repo_path.to_str().unwrap(),
                "--push"
            ])
            .current_dir(repo_path)
            .output()
            .map_err(|e| format!("Failed to execute gh repo create: {}", e))?;

        if !output.status.success() {
            return Err(format!(
                "Failed to create GitHub repository:\n{}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        println!("✅ Created GitHub repository: {}/{}", self.github_owner, self.github_repo_name);
        Ok(())
    }

    /// Create a new branch
    pub fn create_branch(&self, branch_name: &str) -> Result<(), String> {
        self.repo.git(&["checkout", "-b", branch_name]).map(|_| ())
    }

    /// Push current branch to GitHub
    pub fn push_branch(&self, branch_name: &str) -> Result<(), String> {
        self.repo.git(&["push", "--set-upstream", "origin", branch_name]).map(|_| ())
    }

    /// Create a pull request
    pub fn create_pr(&self, title: &str, body: &str) -> Result<String, String> {
        let repo_path = self.repo.path();

        let output = Command::new("gh")
            .args(&[
                "pr", "create",
                "--title", title,
                "--body", body
            ])
            .current_dir(repo_path)
            .output()
            .map_err(|e| format!("Failed to execute gh pr create: {}", e))?;

        if !output.status.success() {
            return Err(format!(
                "Failed to create PR:\n{}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        let pr_url = String::from_utf8_lossy(&output.stdout).trim().to_string();
        println!("✅ Created pull request: {}", pr_url);
        Ok(pr_url)
    }

    /// Merge a pull request with the specified strategy
    pub fn merge_pr(&self, pr_number: &str, strategy: MergeStrategy) -> Result<(), String> {
        let repo_path = self.repo.path();

        let strategy_flag = match strategy {
            MergeStrategy::Squash => "--squash",
            MergeStrategy::Merge => "--merge",
            MergeStrategy::Rebase => "--rebase",
        };

        let output = Command::new("gh")
            .args(&[
                "pr", "merge",
                pr_number,
                strategy_flag,
                "--delete-branch"
            ])
            .current_dir(repo_path)
            .output()
            .map_err(|e| format!("Failed to execute gh pr merge: {}", e))?;

        if !output.status.success() {
            return Err(format!(
                "Failed to merge PR:\n{}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        println!("✅ Merged pull request #{} using {:?} strategy", pr_number, strategy);
        Ok(())
    }

    /// Get the PR number from a PR URL
    pub fn extract_pr_number(&self, pr_url: &str) -> Option<String> {
        pr_url.split('/').last().map(|s| s.to_string())
    }

    /// Get the default branch name from the remote repository
    pub fn get_default_branch(&self) -> Result<String, String> {
        let repo_path = self.repo.path();
        let full_repo = format!("{}/{}", self.github_owner, self.github_repo_name);

        let output = Command::new("gh")
            .args(&["repo", "view", &full_repo, "--json", "defaultBranchRef", "--jq", ".defaultBranchRef.name"])
            .current_dir(repo_path)
            .output()
            .map_err(|e| format!("Failed to get default branch: {}", e))?;

        if !output.status.success() {
            return Err(format!(
                "Failed to get default branch:\n{}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Checkout default branch and pull latest changes from remote
    pub fn checkout_and_pull_default_branch(&self) -> Result<(), String> {
        let default_branch = self.get_default_branch()?;
        self.repo.git(&["checkout", &default_branch])?;
        self.repo.git(&["pull", "origin", &default_branch])?;
        println!("✅ Checked out and pulled latest {} branch", default_branch);
        Ok(())
    }

    /// Delete the GitHub repository
    pub fn delete_from_github(&self) -> Result<(), String> {
        let full_repo = format!("{}/{}", self.github_owner, self.github_repo_name);

        let output = Command::new("gh")
            .args(&[
                "repo", "delete",
                &full_repo,
                "--yes"
            ])
            .output()
            .map_err(|e| format!("Failed to execute gh repo delete: {}", e))?;

        if !output.status.success() {
            return Err(format!(
                "Failed to delete GitHub repository:\n{}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        println!("✅ Deleted GitHub repository: {}", full_repo);
        Ok(())
    }
}

impl Drop for GitHubTestRepo {
    fn drop(&mut self) {
        if std::env::var("GIT_AI_TEST_NO_CLEANUP").is_ok() {
            eprintln!("⚠️  Cleanup disabled - repository preserved: {}/{}",
                self.github_owner, self.github_repo_name);
            eprintln!("   URL: https://github.com/{}/{}",
                self.github_owner, self.github_repo_name);
            return;
        }

        if let Err(e) = self.delete_from_github() {
            eprintln!("⚠️  Failed to cleanup GitHub repository: {}", e);
            eprintln!("   Manual cleanup required: {}/{}", self.github_owner, self.github_repo_name);
        }
    }
}

/// Generate a unique repository name for testing based on test name
fn generate_repo_name(test_name: &str) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    // Sanitize test name: lowercase, replace special chars with hyphens
    let sanitized_name = test_name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string();

    // Truncate if too long (GitHub has a 100 char limit for repo names)
    let max_name_len = 50;
    let truncated_name = if sanitized_name.len() > max_name_len {
        &sanitized_name[..max_name_len]
    } else {
        &sanitized_name
    };

    format!("git-ai-{}-{}", truncated_name, timestamp)
}

/// Get the authenticated GitHub user
fn get_authenticated_user() -> Option<String> {
    let output = Command::new("gh")
        .args(&["api", "user", "--jq", ".login"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
