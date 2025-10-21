use crate::error::GitAiError;
use std::fs;
use std::path::{Path, PathBuf};

const WORKFLOW_TEMPLATE: &str = include_str!("../../templates/squash-authorship.yml");

pub fn run(repo_path: Option<&str>) -> Result<(), GitAiError> {
    let repo_path = repo_path.unwrap_or(".");
    let repo_path = PathBuf::from(repo_path);

    // Ensure this is a git repository
    let git_dir = repo_path.join(".git");
    if !git_dir.exists() {
        return Err(GitAiError::Generic(format!(
            "Not a git repository: {}",
            repo_path.display()
        )));
    }

    // Create .github/workflows directory if it doesn't exist
    let workflows_dir = repo_path.join(".github").join("workflows");
    fs::create_dir_all(&workflows_dir)?;

    // Write the workflow file
    let workflow_path = workflows_dir.join("git-ai-squash-authorship.yml");
    fs::write(&workflow_path, WORKFLOW_TEMPLATE)?;

    println!("✅ Installed GitHub Action workflow:");
    println!("   {}", workflow_path.display());
    println!();
    println!("This workflow will automatically run git-ai squash-authorship");
    println!("when a pull request is squash merged.");
    println!();
    println!("Next steps:");
    println!("  1. Commit and push the workflow file:");
    println!("     git add .github/workflows/git-ai-squash-authorship.yml");
    println!("     git commit -m \"Add git-ai squash authorship workflow\"");
    println!("     git push");

    Ok(())
}
