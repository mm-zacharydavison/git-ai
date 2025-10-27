use crate::ci::ci_context::{CiContext, CiEvent};
use crate::error::GitAiError;
use crate::git::repository::exec_git;
use serde::{Deserialize, Serialize};
use crate::git::repository::find_repository_in_path;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
struct GithubCiEventPayload {
    #[serde(default)]
    pull_request: Option<GithubCiPullRequest>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
struct GithubCiPullRequest {
    base: GithubCiPullRequestReference,
    head: GithubCiPullRequestReference,
    merged: bool,
    merge_commit_sha: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
struct GithubCiPullRequestReference {
    #[serde(rename = "ref")]
    ref_name: String,
    sha: String,
    repo: GithubCiRepository,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
struct GithubCiRepository {
    clone_url: String,
}

pub fn get_github_ci_context() -> Result<Option<CiContext>, GitAiError> {
    let env_event_name = std::env::var("GITHUB_EVENT_NAME").unwrap_or_default();
    let env_event_path = std::env::var("GITHUB_EVENT_PATH").unwrap_or_default();

    if env_event_name != "pull_request" {
        return Ok(None);
    }

    let event_payload = serde_json::from_str::<GithubCiEventPayload>(&std::fs::read_to_string(env_event_path)?).unwrap_or_default();
    if event_payload.pull_request.is_none() {
        return Ok(None);
    }

    let pull_request = event_payload.pull_request.unwrap();

    if !pull_request.merged || pull_request.merge_commit_sha.is_none() {
        return Ok(None);
    }

    let head_ref = pull_request.head.ref_name;
    let head_sha = pull_request.head.sha;
    let base_ref = pull_request.base.ref_name;
    let clone_url = pull_request.base.repo.clone_url.clone();

    let clone_dir = "git-ai-ci-clone".to_string();

    // Clone the repo
    exec_git(&[
        "clone".to_string(),
        "--branch".to_string(),
        base_ref.clone(),
        clone_url,
        clone_dir.clone(),
    ])?;

    let repo = find_repository_in_path(&clone_dir.clone())?;

    Ok(Some(CiContext {
        repo,
        event: CiEvent::Merge {
            merge_commit_sha: pull_request.merge_commit_sha.unwrap(),
            head_ref: head_ref.clone(),
            head_sha: head_sha.clone(),
            base_ref: base_ref.clone(),
            base_sha: pull_request.base.sha.clone(),
        },
        temp_dir: PathBuf::from(clone_dir),
    }))
}