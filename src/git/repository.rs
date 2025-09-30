use crate::error::GitAiError;
use git2::Repository;

pub fn find_repository() -> Result<Repository, GitAiError> {
    Repository::discover(".").map_err(GitAiError::GitError)
}

pub fn find_repository_in_path(path: &str) -> Result<Repository, GitAiError> {
    Repository::discover(path).map_err(GitAiError::GitError)
}
