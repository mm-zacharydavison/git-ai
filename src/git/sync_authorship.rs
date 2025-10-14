use crate::git::refs::{
    AI_AUTHORSHIP_PUSH_REFSPEC, copy_ref, merge_notes_from_ref, ref_exists, tracking_ref_for_remote,
};
use crate::{
    error::GitAiError,
    git::{cli_parser::ParsedGitInvocation, repository::exec_git},
    utils::debug_log,
};

use super::repository::Repository;

pub fn fetch_remote_from_args(
    repository: &Repository,
    parsed_args: &ParsedGitInvocation,
) -> Result<String, GitAiError> {
    let remotes = repository.remotes().ok();
    let remote_names: Vec<String> = remotes
        .as_ref()
        .map(|r| {
            (0..r.len())
                .filter_map(|i| r.get(i).map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    // 2) Fetch authorship refs from the appropriate remote
    // Try to detect remote (named remote, URL, or local path) from args first
    let positional_remote = extract_remote_from_fetch_args(&parsed_args.command_args);
    let specified_remote = positional_remote.or_else(|| {
        parsed_args
            .command_args
            .iter()
            .find(|a| remote_names.iter().any(|r| r == *a))
            .cloned()
    });

    let remote = specified_remote
        .or_else(|| repository.upstream_remote().ok().flatten())
        .or_else(|| repository.get_default_remote().ok().flatten());

    Ok(remote.unwrap().to_string())
}

// for use with post-fetch and post-pull and post-clone hooks
pub fn fetch_authorship_notes(
    repository: &Repository,
    parsed_args: &ParsedGitInvocation,
    remote_name: &str,
) -> Result<(), GitAiError> {
    // Generate tracking ref for this remote
    let tracking_ref = tracking_ref_for_remote(&remote_name);
    let fetch_refspec = format!("+refs/notes/ai:{}", tracking_ref);

    // Build the internal authorship fetch with explicit flags and disabled hooks
    // IMPORTANT: run in the same repo context by prefixing original global args (e.g., -C <path>)
    let mut fetch_authorship: Vec<String> = parsed_args.global_args.clone();
    fetch_authorship.push("-c".to_string());
    fetch_authorship.push("core.hooksPath=/dev/null".to_string());
    fetch_authorship.push("fetch".to_string());
    fetch_authorship.push("--no-tags".to_string());
    fetch_authorship.push("--recurse-submodules=no".to_string());
    fetch_authorship.push("--no-write-fetch-head".to_string());
    fetch_authorship.push("--no-write-commit-graph".to_string());
    fetch_authorship.push("--no-auto-maintenance".to_string());
    fetch_authorship.push(remote_name.to_string());
    fetch_authorship.push(fetch_refspec.clone());

    debug_log(&format!(
        "fetching authorship refs: {:?}",
        &fetch_authorship
    ));

    if let Err(e) = exec_git(&fetch_authorship) {
        // Treat as best-effort; do not fail the user command if authorship sync fails
        debug_log(&format!("authorship fetch skipped due to error: {}", e));
        return Err(e);
    }

    // After successful fetch, merge the tracking ref into refs/notes/ai
    let local_notes_ref = "refs/notes/ai";

    if ref_exists(&repository, &tracking_ref) {
        if ref_exists(&repository, local_notes_ref) {
            // Both exist - merge them
            debug_log(&format!(
                "merging {} into {}",
                tracking_ref, local_notes_ref
            ));
            if let Err(e) = merge_notes_from_ref(&repository, &tracking_ref) {
                debug_log(&format!("notes merge failed: {}", e));
                return Err(e);
            }
        } else {
            // Only tracking ref exists - copy it to local
            debug_log(&format!(
                "initializing {} from {}",
                local_notes_ref, tracking_ref
            ));
            if let Err(e) = copy_ref(&repository, &tracking_ref, local_notes_ref) {
                debug_log(&format!("notes copy failed: {}", e));
                return Err(e);
            }
        }
    }

    Ok(())
}
// for use with post-push hook
pub fn push_authorship_notes(
    repository: &Repository,
    parsed_args: &ParsedGitInvocation,
    remote_name: &str,
) -> Result<(), GitAiError> {
    // STEP 1: Fetch remote notes into tracking ref and merge before pushing
    // This ensures we don't lose notes from other branches/clones
    let tracking_ref = tracking_ref_for_remote(&remote_name);
    let fetch_refspec = format!("+refs/notes/ai:{}", tracking_ref);

    let mut fetch_before_push: Vec<String> = parsed_args.global_args.clone();
    fetch_before_push.push("-c".to_string());
    fetch_before_push.push("core.hooksPath=/dev/null".to_string());
    fetch_before_push.push("fetch".to_string());
    fetch_before_push.push("--no-tags".to_string());
    fetch_before_push.push("--recurse-submodules=no".to_string());
    fetch_before_push.push("--no-write-fetch-head".to_string());
    fetch_before_push.push("--no-write-commit-graph".to_string());
    fetch_before_push.push("--no-auto-maintenance".to_string());
    fetch_before_push.push(remote_name.to_string());
    fetch_before_push.push(fetch_refspec);

    debug_log(&format!(
        "pre-push authorship fetch: {:?}",
        &fetch_before_push
    ));

    // Fetch is best-effort; if it fails (e.g., no remote notes yet), continue
    if exec_git(&fetch_before_push).is_ok() {
        // Merge fetched notes into local refs/notes/ai
        let local_notes_ref = "refs/notes/ai";

        if ref_exists(repository, &tracking_ref) {
            if ref_exists(repository, local_notes_ref) {
                // Both exist - merge them
                debug_log(&format!(
                    "pre-push: merging {} into {}",
                    tracking_ref, local_notes_ref
                ));
                if let Err(e) = merge_notes_from_ref(repository, &tracking_ref) {
                    debug_log(&format!("pre-push notes merge failed: {}", e));
                }
            } else {
                // Only tracking ref exists - copy it to local
                debug_log(&format!(
                    "pre-push: initializing {} from {}",
                    local_notes_ref, tracking_ref
                ));
                if let Err(e) = copy_ref(repository, &tracking_ref, local_notes_ref) {
                    debug_log(&format!("pre-push notes copy failed: {}", e));
                }
            }
        }
    }

    // STEP 2: Push notes without force (requires fast-forward)
    let mut push_authorship: Vec<String> = parsed_args.global_args.clone();
    push_authorship.push("-c".to_string());
    push_authorship.push("core.hooksPath=/dev/null".to_string());
    push_authorship.push("push".to_string());
    push_authorship.push("--quiet".to_string());
    push_authorship.push("--no-recurse-submodules".to_string());
    push_authorship.push("--no-verify".to_string());
    push_authorship.push(remote_name.to_string());
    push_authorship.push(AI_AUTHORSHIP_PUSH_REFSPEC.to_string());

    debug_log(&format!(
        "pushing authorship refs (no force): {:?}",
        &push_authorship
    ));
    if let Err(e) = exec_git(&push_authorship) {
        // Best-effort; don't fail user operation due to authorship sync issues
        debug_log(&format!("authorship push skipped due to error: {}", e));
        return Err(e);
    }

    Ok(())
}

fn extract_remote_from_fetch_args(args: &[String]) -> Option<String> {
    let mut after_double_dash = false;

    for arg in args {
        if !after_double_dash {
            if arg == "--" {
                after_double_dash = true;
                continue;
            }
            if arg.starts_with('-') {
                // Option; skip
                continue;
            }
        }

        // Candidate positional arg; determine if it's a repository URL/path
        let s = arg.as_str();

        // 1) URL forms (https://, ssh://, file://, git://, etc.)
        if s.contains("://") || s.starts_with("file://") {
            return Some(arg.clone());
        }

        // 2) SCP-like syntax: user@host:path
        if s.contains('@') && s.contains(':') && !s.contains("://") {
            return Some(arg.clone());
        }

        // 3) Local path forms
        if s.starts_with('/') || s.starts_with("./") || s.starts_with("../") || s.starts_with("~/")
        {
            return Some(arg.clone());
        }

        // Heuristic: bare repo directories often end with .git
        if s.ends_with(".git") {
            return Some(arg.clone());
        }

        // 4) As a last resort, if the path exists on disk, treat as local path
        if std::path::Path::new(s).exists() {
            return Some(arg.clone());
        }

        // Otherwise, do not treat this positional token as a repository; likely a refspec
        break;
    }

    None
}
