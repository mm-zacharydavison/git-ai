use crate::authorship::authorship_log_serialization::{AUTHORSHIP_LOG_VERSION, AuthorshipLog};
use crate::authorship::working_log::Checkpoint;
use crate::error::GitAiError;
use crate::git::repository::{Repository, exec_git, exec_git_stdin};
use serde_json;
use std::fs;

pub const AI_AUTHORSHIP_REFSPEC: &str = "+refs/notes/ai/*:refs/notes/ai/*";

pub fn notes_add(
    repo: &Repository,
    commit_sha: &str,
    note_content: &str,
) -> Result<(), GitAiError> {
    let mut args = repo.global_args_for_exec();
    args.push("notes".to_string());
    args.push("--ref=ai".to_string());
    args.push("add".to_string());
    args.push("-f".to_string()); // Always force overwrite
    args.push("-F".to_string());
    args.push("-".to_string()); // Read note content from stdin
    args.push(commit_sha.to_string());

    // Use stdin to provide the note content to avoid command line length limits
    exec_git_stdin(&args, note_content.as_bytes())?;
    Ok(())
}

// Show an authorship note and return its JSON content if found, or None if it doesn't exist.
pub fn show_authorship_note(repo: &Repository, commit_sha: &str) -> Option<String> {
    let mut args = repo.global_args_for_exec();
    args.push("notes".to_string());
    args.push("--ref=ai".to_string());
    args.push("show".to_string());
    args.push(commit_sha.to_string());

    match exec_git(&args) {
        Ok(output) => String::from_utf8(output.stdout)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        Err(GitAiError::GitCliError { code: Some(1), .. }) => None,
        Err(_) => None,
    }
}

#[allow(dead_code)]
pub fn get_reference_as_working_log(
    repo: &Repository,
    commit_sha: &str,
) -> Result<Vec<Checkpoint>, GitAiError> {
    let content = show_authorship_note(repo, commit_sha)
        .ok_or_else(|| GitAiError::Generic("No authorship note found".to_string()))?;
    let working_log = serde_json::from_str(&content)?;
    Ok(working_log)
}

pub fn get_reference_as_authorship_log_v3(
    repo: &Repository,
    commit_sha: &str,
) -> Result<AuthorshipLog, GitAiError> {
    let content = show_authorship_note(repo, commit_sha)
        .ok_or_else(|| GitAiError::Generic("No authorship note found".to_string()))?;

    // Try to deserialize as AuthorshipLog
    let authorship_log = match AuthorshipLog::deserialize_from_string(&content) {
        Ok(log) => log,
        Err(_) => {
            return Err(GitAiError::Generic(
                "Failed to parse authorship log".to_string(),
            ));
        }
    };

    // Check version compatibility
    if authorship_log.metadata.schema_version != AUTHORSHIP_LOG_VERSION {
        return Err(GitAiError::Generic(format!(
            "Unsupported authorship log version: {} (expected: {})",
            authorship_log.metadata.schema_version, AUTHORSHIP_LOG_VERSION
        )));
    }

    Ok(authorship_log)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_utils::TmpRepo;

    #[test]
    fn test_notes_add_and_show_authorship_note() {
        // Create a temporary repository
        let tmp_repo = TmpRepo::new().expect("Failed to create tmp repo");

        // Create a commit first
        tmp_repo
            .commit_with_message("Initial commit")
            .expect("Failed to create initial commit");

        // Get the commit SHA
        let commit_sha = tmp_repo
            .get_head_commit_sha()
            .expect("Failed to get head commit SHA");

        // Test data - simple string content
        let note_content = "This is a test authorship note with some random content!";

        // Add the authorship note (force overwrite since commit_with_message already created one)
        notes_add(tmp_repo.gitai_repo(), &commit_sha, note_content)
            .expect("Failed to add authorship note");

        // Read the note back
        let retrieved_content = show_authorship_note(tmp_repo.gitai_repo(), &commit_sha)
            .expect("Failed to retrieve authorship note");

        // Assert the content matches exactly
        assert_eq!(retrieved_content, note_content);

        // Test that non-existent commit returns None
        let non_existent_content = show_authorship_note(
            tmp_repo.gitai_repo(),
            "0000000000000000000000000000000000000000",
        );
        assert!(non_existent_content.is_none());
    }
}
