// hi from AI
use crate::error::GitAiError;
use crate::git::refs::{setup_ai_refspecs, setup_git_hooks};
use git2::Repository;

pub fn run(repo: &Repository) -> Result<(), GitAiError> {
    setup_ai_refspecs(repo)?;
    setup_git_hooks(repo)?;
    Ok(())
}
