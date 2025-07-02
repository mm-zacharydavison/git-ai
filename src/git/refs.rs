use crate::error::GitAiError;
use crate::log_fmt::authorship_log::AuthorshipLog;
use crate::log_fmt::working_log::Checkpoint;
use git2::Repository;
use serde_json;
use std::fs;
use std::io::{self, Write};

///
pub fn put_reference(
    repo: &Repository,
    ref_name: &str,
    content: &str,
    message: &str,
) -> Result<(), GitAiError> {
    // Create the AI namespace directory structure
    let git_dir = repo.path();
    let ai_refs_dir = git_dir.join("refs").join("ai");

    // Create the directory if it doesn't exist
    fs::create_dir_all(&ai_refs_dir)?;

    // Create the blob object
    let oid = repo.blob(content.as_bytes())?;

    // Create the reference
    let full_ref_name = format!("refs/{}", ref_name);
    repo.reference(&full_ref_name, oid, true, message)?;

    Ok(())
}

pub fn get_reference(repo: &Repository, ref_name: &str) -> Result<String, GitAiError> {
    let full_ref_name = format!("refs/{}", ref_name);

    // Get the reference
    let reference = repo.find_reference(&full_ref_name)?;

    // Get the object that the reference points to
    let object = reference.peel_to_blob()?;

    // Convert the blob content to a string, handling invalid UTF-8 gracefully
    let content = String::from_utf8_lossy(object.content());

    Ok(content.to_string())
}

#[allow(dead_code)]
pub fn get_reference_as_working_log(
    repo: &Repository,
    ref_name: &str,
) -> Result<Vec<Checkpoint>, GitAiError> {
    let content = get_reference(repo, ref_name)?;
    let working_log = serde_json::from_str(&content)?;
    Ok(working_log)
}

pub fn get_reference_as_authorship_log(
    repo: &Repository,
    ref_name: &str,
) -> Result<AuthorshipLog, GitAiError> {
    let content = get_reference(repo, ref_name)?;
    let authorship_log = serde_json::from_str(&content)?;
    Ok(authorship_log)
}

/// Set up refspecs for the AI namespace using git CLI for every remote
pub fn setup_ai_refspecs(repo: &Repository) -> Result<(), GitAiError> {
    let remotes = repo.remotes()?;

    for i in 0..remotes.len() {
        if let Some(remote_name) = remotes.get(i) {
            // Check if default fetch refspec exists, if not add it
            let fetch_output = std::process::Command::new("git")
                .args(["config", "--get", &format!("remote.{}.fetch", remote_name)])
                .output()?;

            if fetch_output.stdout.is_empty() {
                // No fetch refspec exists, add the default one first
                std::process::Command::new("git")
                    .args([
                        "config",
                        "--add",
                        &format!("remote.{}.fetch", remote_name),
                        "+refs/heads/*:refs/remotes/{}/*",
                    ])
                    .status()?;
            }

            // Add AI fetch refspec
            let fetch_status = std::process::Command::new("git")
                .args([
                    "config",
                    "--add",
                    &format!("remote.{}.fetch", remote_name),
                    "refs/ai/*:refs/ai/*",
                ])
                .status()?;

            // Check if default push refspec exists, if not add it
            let push_output = std::process::Command::new("git")
                .args(["config", "--get", &format!("remote.{}.push", remote_name)])
                .output()?;

            if push_output.stdout.is_empty() {
                // No push refspec exists, add the default one first
                std::process::Command::new("git")
                    .args([
                        "config",
                        "--add",
                        &format!("remote.{}.push", remote_name),
                        "refs/heads/*:refs/heads/*",
                    ])
                    .status()?;
            }

            // Add AI push refspec
            let push_status = std::process::Command::new("git")
                .args([
                    "config",
                    "--add",
                    &format!("remote.{}.push", remote_name),
                    "refs/ai/*:refs/ai/*",
                ])
                .status()?;

            if push_status.success() && fetch_status.success() {
                println!("Added AI refspecs for remote: {}", remote_name);
            }
        }
    }

    Ok(())
}

/// Set up pre-commit and post-commit hooks for git-ai
pub fn setup_git_hooks(repo: &Repository) -> Result<(), GitAiError> {
    let git_dir = repo.path();
    let hooks_dir = git_dir.join("hooks");

    // Create hooks directory if it doesn't exist
    fs::create_dir_all(&hooks_dir)?;

    let pre_commit_hook = hooks_dir.join("pre-commit");
    let post_commit_hook = hooks_dir.join("post-commit");
    let pre_commit_content = r#"#!/bin/sh
# git-ai pre-commit hook
git-ai pre-commit
"#;
    let post_commit_content = r#"#!/bin/sh
# git-ai post-commit hook
git-ai post-commit
"#;

    let pre_commit_exists = pre_commit_hook.exists();
    let post_commit_exists = post_commit_hook.exists();

    if !pre_commit_exists && !post_commit_exists {
        // Neither hook exists, write both
        fs::write(&pre_commit_hook, pre_commit_content)?;
        fs::write(&post_commit_hook, post_commit_content)?;
        // Make executable
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&pre_commit_hook)?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&pre_commit_hook, perms)?;
            let mut perms = fs::metadata(&post_commit_hook)?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&post_commit_hook, perms)?;
        }
        println!(
            "Git hooks set up successfully:\n  - pre-commit: git-ai pre-commit\n  - post-commit: git-ai post-commit"
        );
        return Ok(());
    }

    // If either exists, print instructions and prompt
    println!("One or both git hooks already exist.");
    println!("To enable git-ai, add the following lines to your hooks (if not already present):\n");
    println!("# In .git/hooks/pre-commit:\ngit-ai pre-commit\n");
    println!("# In .git/hooks/post-commit:\ngit-ai post-commit\n");
    println!(
        "Press 'a' to append these lines to the end of your hooks (risky), or any other key to exit."
    );
    print!("> ");
    io::stdout().flush().unwrap();
    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    if input.trim() == "a" {
        // Append to pre-commit
        if pre_commit_exists {
            let mut file = fs::OpenOptions::new().append(true).open(&pre_commit_hook)?;
            writeln!(file, "\ngit-ai pre-commit")?;
        } else {
            fs::write(&pre_commit_hook, pre_commit_content)?;
        }
        // Append to post-commit
        if post_commit_exists {
            let mut file = fs::OpenOptions::new()
                .append(true)
                .open(&post_commit_hook)?;
            writeln!(file, "\ngit-ai post-commit")?;
        } else {
            fs::write(&post_commit_hook, post_commit_content)?;
        }
        // Make executable
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&pre_commit_hook)?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&pre_commit_hook, perms)?;
            let mut perms = fs::metadata(&post_commit_hook)?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&post_commit_hook, perms)?;
        }
        println!("Appended git-ai commands to hooks.");
    } else {
        println!("No changes made to hooks.");
    }
    Ok(())
}
