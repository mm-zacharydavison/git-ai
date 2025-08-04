use crate::error::GitAiError;
use crate::git::refs::get_reference;
use crate::git::refs::get_reference_as_working_log;
use crate::git::refs::put_reference;
use crate::log_fmt::authorship_log::AuthorshipLog;
use git2::Repository;
use std::collections::HashMap;

pub fn run(repo: &Repository) -> Result<(), GitAiError> {
    // Find all working log refs
    let working_log_refs = find_working_log_refs(repo)?;

    // Filter to only show refs with > 0 checkpoints
    let mut filtered_refs: HashMap<String, usize> = working_log_refs
        .into_iter()
        .filter(|(_, checkpoint_count)| *checkpoint_count > 0)
        .collect();

    // Build reverse lookup map for child commits (much faster than checking each commit individually)
    let mut parent_to_children: HashMap<String, Vec<String>> = HashMap::new();

    // Get all references and build the parent->children map
    let refs = repo.references()?;
    let mut processed_commits = std::collections::HashSet::new();

    for reference in refs {
        let reference = reference?;
        let ref_name = reference.name().unwrap_or("");

        // Skip ai-working-log refs to avoid self-references
        if ref_name.starts_with("refs/ai-working-log/") {
            continue;
        }

        // Check if this ref points to a commit
        if let Ok(commit) = reference.peel_to_commit() {
            // Limit to last 500 commits across all branches
            if processed_commits.len() >= 500 {
                continue;
            }

            let commit_id = commit.id().to_string();
            if processed_commits.insert(commit_id.clone()) {
                // Add this commit as a child of each of its parents
                for parent in commit.parents() {
                    let parent_id = parent.id().to_string();
                    parent_to_children
                        .entry(parent_id)
                        .or_insert_with(Vec::new)
                        .push(commit_id.clone());
                }
            }
        }
    }

    // Filter using the pre-built map
    filtered_refs.retain(|commit_hash, _| parent_to_children.contains_key(commit_hash));

    if filtered_refs.is_empty() {
        return Ok(());
    }

    // Sort by commit date (most recent first)
    let mut sorted_refs: Vec<(String, usize, i64)> = Vec::new();
    for (commit_hash, checkpoint_count) in filtered_refs {
        // Get commit date
        let commit_time = match repo.revparse_single(&commit_hash) {
            Ok(obj) => {
                if let Ok(commit) = obj.peel_to_commit() {
                    commit.time().seconds()
                } else {
                    0 // fallback
                }
            }
            Err(_) => 0, // fallback
        };
        sorted_refs.push((commit_hash, checkpoint_count, commit_time));
    }

    // Sort by commit time (most recent first)
    sorted_refs.sort_by(|a, b| b.2.cmp(&a.2));

    // Extract commit hashes for later use
    let commit_hashes: Vec<String> = sorted_refs
        .iter()
        .map(|(hash, _, _)| hash.clone())
        .collect();

    // Create authorship logs for direct children that don't already have one
    let mut authorship_created = 0;

    for commit_hash in &commit_hashes {
        // Get the working log for this commit
        let working_log =
            match get_reference_as_working_log(repo, &format!("ai-working-log/{}", commit_hash)) {
                Ok(checkpoints) => checkpoints,
                Err(_) => continue, // Skip if we can't get the working log
            };

        // Get direct children of this commit
        let empty_vec = Vec::new();
        let children = parent_to_children.get(commit_hash).unwrap_or(&empty_vec);

        if children.is_empty() {
            continue;
        }

        for child_commit in children {
            // Check if authorship log already exists for this child
            let authorship_ref = format!("ai/authorship/{}", child_commit);

            if get_reference(repo, &authorship_ref).is_err() {
                // No authorship log exists, create one
                let authorship_log = AuthorshipLog::from_working_log(&working_log);

                // Serialize the authorship log
                let authorship_json = serde_json::to_string(&authorship_log)?;

                // Create the authorship log reference
                put_reference(
                    repo,
                    &authorship_ref,
                    &authorship_json,
                    &format!("AI authorship attestation for commit {}", child_commit),
                )?;

                println!("refs/ai/authorship{}", child_commit);
                authorship_created += 1;
            }
        }
    }

    // Delete working logs after creating authorship logs
    for commit_hash in &commit_hashes {
        let working_log_ref = format!("ai-working-log/{}", commit_hash);

        let empty_vec = Vec::new();
        let children = parent_to_children.get(commit_hash).unwrap_or(&empty_vec);

        let all_children_have_authorship = children.iter().all(|child| {
            let authorship_ref = format!("ai/authorship/{}", child);
            get_reference(repo, &authorship_ref).is_ok()
        });

        if all_children_have_authorship && !children.is_empty() {
            // Delete the working log reference
            let full_ref_name = format!("refs/{}", working_log_ref);
            if let Ok(mut reference) = repo.find_reference(&full_ref_name) {
                reference.delete()?;
            }
        }
    }

    Ok(())
}

fn find_working_log_refs(repo: &Repository) -> Result<HashMap<String, usize>, GitAiError> {
    let mut working_log_refs = HashMap::new();

    // Get all references in the repository
    let refs = repo.references()?;

    for reference in refs {
        let reference = reference?;
        let ref_name = reference.name().unwrap_or("");

        if ref_name.starts_with("refs/ai-working-log/") {
            let base_commit = ref_name.trim_start_matches("refs/ai-working-log/");
            match get_reference_as_working_log(repo, &format!("ai-working-log/{}", base_commit)) {
                Ok(checkpoints) => {
                    working_log_refs.insert(base_commit.to_string(), checkpoints.len());
                }
                Err(_) => {
                    // If we can't parse it as a working log, still include it but with 0 count
                    working_log_refs.insert(base_commit.to_string(), 0);
                }
            }
        }
    }

    Ok(working_log_refs)
}
