use crate::authorship::authorship_log_serialization::AuthorshipLog;
use crate::error::GitAiError;
use crate::git::refs::get_reference_as_authorship_log_v3;
use crate::git::repository::Repository;
use std::collections::HashMap;

/// A cache for authorship logs that wraps a HashMap and provides lazy loading.
///
/// This struct is designed to be passed through function call stacks to avoid
/// redundant git note lookups when the same commit's authorship log is needed
/// multiple times during an operation.
pub struct AuthorshipLogCache {
    cache: HashMap<String, AuthorshipLog>,
}

impl AuthorshipLogCache {
    /// Create a new empty cache
    pub fn new() -> Self {
        AuthorshipLogCache {
            cache: HashMap::new(),
        }
    }

    /// Get or fetch an authorship log, caching the result for future lookups.
    ///
    /// # Arguments
    /// * `repo` - Git repository
    /// * `commit_sha` - SHA of the commit to fetch the authorship log for
    ///
    /// # Returns
    /// A reference to the cached `AuthorshipLog`, or an error if fetching failed
    pub fn get_or_fetch(
        &mut self,
        repo: &Repository,
        commit_sha: &str,
    ) -> Result<&AuthorshipLog, GitAiError> {
        // Check if we have this cached
        if !self.cache.contains_key(commit_sha) {
            // Fetch and cache the log (only cache successful fetches)
            let log = get_reference_as_authorship_log_v3(repo, commit_sha)?;
            self.cache.insert(commit_sha.to_string(), log);
        }

        // Safe to unwrap since we just ensured it exists
        Ok(self.cache.get(commit_sha).unwrap())
    }

    /// Get a cached authorship log without fetching.
    ///
    /// # Arguments
    /// * `commit_sha` - SHA of the commit to look up
    ///
    /// # Returns
    /// A reference to the cached `AuthorshipLog`, or `None` if not cached
    pub fn get_cached(&self, commit_sha: &str) -> Option<&AuthorshipLog> {
        self.cache.get(commit_sha)
    }

    /// Check if a commit's authorship log is in the cache.
    ///
    /// # Arguments
    /// * `commit_sha` - SHA of the commit to check
    ///
    /// # Returns
    /// `true` if the log is cached, `false` otherwise
    pub fn is_cached(&self, commit_sha: &str) -> bool {
        self.cache.contains_key(commit_sha)
    }

    /// Get the number of entries in the cache
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    /// Check if the cache is empty
    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }
}

impl Default for AuthorshipLogCache {
    fn default() -> Self {
        Self::new()
    }
}
