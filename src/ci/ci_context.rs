use crate::git::repository::Repository;
use crate::error::GitAiError;
use crate::authorship::rebase_authorship::rewrite_authorship_after_squash_or_rebase;
use crate::git::sync_authorship::fetch_authorship_notes;
use std::fs;
use std::path::PathBuf;

#[derive(Debug)]
pub enum CiEvent {
    Merge {
        merge_commit_sha: String,
        head_ref: String,
        head_sha: String,
        base_ref: String,
        base_sha: String,
    }
}

#[derive(Debug)]
pub struct CiContext {
    pub repo: Repository,
    pub event: CiEvent,
    pub temp_dir: PathBuf,
}

impl CiContext {
    pub fn run(&self) -> Result<(), GitAiError> {
        match &self.event {
            CiEvent::Merge { merge_commit_sha, head_ref, head_sha, base_ref, base_sha } => {
                // Ensure we have all the required commits from the base branch
                self.repo.fetch_branch(base_ref, "origin")?;
                // Ensure we have the full authorship history
                fetch_authorship_notes(&self.repo, "origin")?;
                // Rewrite authorship
                rewrite_authorship_after_squash_or_rebase(&self.repo, &head_ref, &head_sha, &merge_commit_sha, false)?;
                // Push authorship
                self.repo.push_authorship("origin")?;
                Ok(())
            }
        }
    }

    pub fn teardown(&self) -> Result<(), GitAiError> {
        fs::remove_dir_all(self.temp_dir.clone())?;
        Ok(())
    }
}
