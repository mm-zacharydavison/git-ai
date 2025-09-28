use crate::error::GitAiError;
use crate::log_fmt::authorship_log::{AUTHORSHIP_LOG_VERSION, AuthorshipLog};
use crate::log_fmt::authorship_log_serialization::AuthorshipLogV3;
use crate::log_fmt::working_log::Checkpoint;
use git2::Repository;
use serde_json;
use std::fs;

pub const AI_AUTHORSHIP_REFSPEC: &str = "+refs/ai/authorship/*:refs/ai/authorship/*";
pub const DEFAULT_REFSPEC: &str = "+refs/heads/*:refs/heads/*";

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

pub fn get_reference_as_authorship_log_v3(
    repo: &Repository,
    ref_name: &str,
) -> Result<AuthorshipLogV3, GitAiError> {
    let content = get_reference(repo, ref_name)?;

    // Try to detect format: new text format vs old JSON format
    if content.contains("---") {
        // New text format - parse as AuthorshipLogV3
        match AuthorshipLogV3::deserialize_from_string(&content) {
            Ok(v3_log) => Ok(v3_log),
            Err(_) => Err(GitAiError::Generic(
                "Failed to parse new format authorship log".to_string(),
            )),
        }
    } else {
        // Old JSON format - convert to V3
        let version_check: serde_json::Value = serde_json::from_str(&content)?;
        if let Some(schema_version) = version_check.get("schema_version").and_then(|v| v.as_str()) {
            if schema_version != AUTHORSHIP_LOG_VERSION {
                return Err(GitAiError::Generic(format!(
                    "Unsupported authorship log version: {} (expected: {})",
                    schema_version, AUTHORSHIP_LOG_VERSION
                )));
            }
        } else {
            return Err(GitAiError::Generic(
                "No schema_version found in authorship log".to_string(),
            ));
        }

        // Parse old format and convert to V3
        let authorship_log: AuthorshipLog = serde_json::from_str(&content)?;
        Ok(AuthorshipLogV3::from_authorship_log(&authorship_log))
    }
}


pub fn delete_reference(repo: &Repository, ref_name: &str) -> Result<(), GitAiError> {
    let full_ref_name = format!("refs/{}", ref_name);

    // Try to find and delete the reference
    match repo.find_reference(&full_ref_name) {
        Ok(mut reference) => {
            reference.delete()?;
            Ok(())
        }
        Err(_) => {
            // Reference doesn't exist, which is fine for deletion
            Ok(())
        }
    }
}
