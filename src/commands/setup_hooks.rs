use crate::error::GitAiError;
use crate::git::refs::setup_git_hooks;
use git2::Repository;

pub fn run(repo: &Repository) -> Result<(), GitAiError> {
    setup_git_hooks(repo)
}
