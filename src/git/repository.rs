use crate::error::GitAiError;
use git2::Repository;

pub fn find_repository() -> Result<Repository, GitAiError> {
    Repository::open(".").map_err(GitAiError::GitError)
}
