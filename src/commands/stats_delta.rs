use crate::error::GitAiError;
use crate::git::refs::get_reference;
use crate::git::refs::get_reference_as_working_log;
use crate::git::refs::put_reference;
use crate::log_fmt::authorship_log::AuthorshipLog;
use git2::Repository;
use std::collections::HashMap;
use std::time::Instant;

pub fn run(repo: &Repository) -> Result<(), GitAiError> {
    let start_time = Instant::now();

    // Find all working log refs
    let find_start = Instant::now();
    let working_log_refs = find_working_log_refs(repo)?;
    let find_duration = find_start.elapsed();
    println!("Finding working log refs took: {:?}", find_duration);

    // Filter to only show refs with > 0 checkpoints
    let filter_start = Instant::now();
    let mut filtered_refs: HashMap<String, usize> = working_log_refs
        .into_iter()
        .filter(|(_, checkpoint_count)| *checkpoint_count > 0)
        .collect();
    let filter_duration = filter_start.elapsed();
    println!("Filtering by checkpoint count took: {:?}", filter_duration);

    // Build reverse lookup map for child commits (much faster than checking each commit individually)
    let child_filter_start = Instant::now();
    let mut parent_to_children: HashMap<String, Vec<String>> = HashMap::new();

    // Get all references and build the parent->children map
    let refs = repo.references()?;
    let mut ref_count = 0;
    let mut commit_count = 0;
    let mut processed_commits = std::collections::HashSet::new();

    for reference in refs {
        let reference = reference?;
        ref_count += 1;
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
                commit_count += 1;

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

    println!(
        "Built parent->children map: checked {} refs, {} commits",
        ref_count, commit_count
    );

    // Filter using the pre-built map
    let mut child_check_count = 0;
    filtered_refs.retain(|commit_hash, _| {
        child_check_count += 1;
        parent_to_children.contains_key(commit_hash)
    });
    let child_filter_duration = child_filter_start.elapsed();
    println!(
        "Filtering by child commits took: {:?} (checked {} commits)",
        child_filter_duration, child_check_count
    );

    if filtered_refs.is_empty() {
        println!("No working log refs with checkpoints and child commits found.");
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

    println!(
        "Found {} working log ref(s) with checkpoints and child commits:",
        sorted_refs.len()
    );
    println!();

    for (commit_hash, checkpoint_count, _) in &sorted_refs {
        println!("  {} ({} checkpoint(s))", commit_hash, checkpoint_count);
    }

    // Create authorship logs for direct children that don't already have one
    let authorship_start = Instant::now();
    let mut authorship_created = 0;

    let mut total_children = 0;
    let mut existing_authorship_logs = 0;
    let mut no_children = 0;

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
            no_children += 1;
            continue;
        }

        total_children += children.len();

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

                println!(
                    "Created authorship log for {} from working log of {}",
                    child_commit, commit_hash
                );
                authorship_created += 1;
            } else {
                existing_authorship_logs += 1;
            }
        }
    }

    println!("Debug breakdown:");
    println!("  - Working logs with no children: {}", no_children);
    println!("  - Total children found: {}", total_children);
    println!(
        "  - Children with existing authorship logs: {}",
        existing_authorship_logs
    );
    println!(
        "  - Children needing authorship logs: {}",
        authorship_created
    );

    let authorship_duration = authorship_start.elapsed();
    println!(
        "Authorship log creation took: {:?} (created {} logs)",
        authorship_duration, authorship_created
    );

    // Delete working logs after creating authorship logs
    let delete_start = Instant::now();
    let mut working_logs_deleted = 0;

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
                println!("Deleted working log for {}", commit_hash);
                working_logs_deleted += 1;
            }
        }
    }

    let delete_duration = delete_start.elapsed();
    println!(
        "Working log deletion took: {:?} (deleted {} logs)",
        delete_duration, working_logs_deleted
    );

    let total_duration = start_time.elapsed();
    println!("\nTotal execution time: {:?}", total_duration);

    Ok(())
}

fn find_working_log_refs(repo: &Repository) -> Result<HashMap<String, usize>, GitAiError> {
    let mut working_log_refs = HashMap::new();

    // Get all references in the repository
    let refs = repo.references()?;

    for reference in refs {
        let reference = reference?;
        let ref_name = reference.name().unwrap_or("");

        // Check if this is a working log ref (starts with refs/ai-working-log/)
        if ref_name.starts_with("refs/ai-working-log/") {
            // Extract the base commit from the ref name
            let base_commit = ref_name.trim_start_matches("refs/ai-working-log/");

            // Try to load the working log to get the checkpoint count
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
