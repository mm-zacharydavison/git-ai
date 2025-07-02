use crate::error::GitAiError;
use git2::Repository;

pub fn run(repo: &Repository, default_user_name: String) -> Result<(), GitAiError> {
    // Run checkpoint as human editor.
    let result = super::checkpoint::run(repo, &default_user_name, false, false, true, None, None);
    result.map(|_| ())
}
