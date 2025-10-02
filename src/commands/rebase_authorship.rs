use crate::error::GitAiError;
use crate::git::find_repository;
use crate::git::repository::{Commit, Repository};
use crate::log_fmt::authorship_log_serialization::AuthorshipLog;
use similar::{ChangeTag, TextDiff};

/// Rewrite authorship log after a squash merge or rebase
///
/// This function handles the complex case where multiple commits from a linear history
/// have been squashed into a single new commit (new_sha). It preserves AI authorship attribution
/// by analyzing the diff and applying blame logic to identify which lines were originally
/// authored by AI.
///
/// # Arguments
/// * `repo` - Git repository
/// * `head_sha` - SHA of the HEAD commit of the original history that was squashed
/// * `new_sha` - SHA of the new squash commit
///
/// # Returns
/// The authorship log for the new commit
pub fn rewrite_authorship_after_squash_or_rebase(
    repo: &Repository,
    _destination_branch: &str,
    head_sha: &str,
    new_sha: &str,
    dry_run: bool,
) -> Result<AuthorshipLog, GitAiError> {
    // Step 1: Find the common origin base
    let origin_base = find_common_origin_base_from_head(repo, head_sha, new_sha)?;

    // Step 2: Build the old_shas path from head_sha to origin_base
    let _old_shas = build_commit_path_to_base(repo, head_sha, &origin_base)?;

    // Step 3: Get the parent of the new commit
    let new_commit = repo.find_commit(new_sha.to_string())?;
    let new_commit_parent = new_commit.parent(0)?;

    // Step 4: Compute a diff between origin_base and new_commit_parent. Sometimes it's the same
    // sha. that's ok
    let origin_base_commit = repo.find_commit(origin_base.to_string())?;
    let origin_base_tree = origin_base_commit.tree()?;
    let new_commit_parent_tree = new_commit_parent.tree()?;

    // TODO Is this diff necessary? The result is unused
    // Create diff between the two trees
    let _diff =
        repo.diff_tree_to_tree(Some(&origin_base_tree), Some(&new_commit_parent_tree), None)?;

    // Step 5: Take this diff and apply it to the HEAD of the old shas history.
    // We want it to be a merge essentially, and Accept Theirs (OLD Head wins when there's conflicts)
    let hanging_commit_sha = apply_diff_as_merge_commit(
        repo,
        &origin_base,
        &new_commit_parent.id().to_string(),
        head_sha, // HEAD of old shas history
    )?;

    // Step 5: Now get the diff between between new_commit and new_commit_parent.
    // We want just the changes between the two commits.
    // We will iterate each file / hunk and then, we will run @blame logic in the context of
    // hanging_commit_sha
    // That way we can get the authorship log pre-squash.
    // Aggregate the results in a variable, then we'll dump a new authorship log.
    let new_authorship_log = reconstruct_authorship_from_diff(
        repo,
        &new_commit,
        &new_commit_parent,
        &hanging_commit_sha,
    )?;

    // println!("Reconstructed authorship log with {:?}", new_authorship_log);

    // Step (Last): Delete the hanging commit

    delete_hanging_commit(repo, &hanging_commit_sha)?;
    // println!("Deleted hanging commit: {}", hanging_commit_sha);

    if !dry_run {
        // Step (Save): Save the authorship log with the new sha as its id
        let authorship_json = new_authorship_log
            .serialize_to_string()
            .map_err(|_| GitAiError::Generic("Failed to serialize authorship log".to_string()))?;

        let ref_name = format!("ai/authorship/{}", new_sha);
        crate::git::refs::put_reference(
            repo,
            &ref_name,
            &authorship_json,
            &format!(
                "AI authorship attestation for squashed/rebased commit {}",
                new_sha
            ),
        )?;

        println!("Authorship log saved to refs/{}", ref_name);
    }

    Ok(new_authorship_log)
}

/// Apply a diff as a merge commit, creating a hanging commit that's not attached to any branch
///
/// This function takes the diff between origin_base and new_commit_parent and applies it
/// to the old_head_sha, creating a merge commit where conflicts are resolved by accepting
/// the old head's version (Accept Theirs strategy).
///
/// # Arguments
/// * `repo` - Git repository
/// * `origin_base` - The common base commit SHA
/// * `new_commit_parent` - The new commit's parent SHA
/// * `old_head_sha` - The HEAD of the old shas history
///
/// # Returns
/// The SHA of the created hanging commit
fn apply_diff_as_merge_commit(
    repo: &Repository,
    origin_base: &str,
    new_commit_parent: &str,
    old_head_sha: &str,
) -> Result<String, GitAiError> {
    // Resolve the merge as a real three-way merge of trees
    // base: origin_base, ours: old_head_sha, theirs: new_commit_parent
    // Favor OURS (old_head) on conflicts per comment "OLD Head wins when there's conflicts"
    let base_commit = repo.find_commit(origin_base.to_string())?;
    let ours_commit = repo.find_commit(old_head_sha.to_string())?;
    let theirs_commit = repo.find_commit(new_commit_parent.to_string())?;

    let base_tree = base_commit.tree()?;
    let ours_tree = ours_commit.tree()?;
    let theirs_tree = theirs_commit.tree()?;

    // NOTE: Below is the libgit2 version of the logic (merge, write, find)
    // Perform the merge of trees to an index
    // let mut index = repo.merge_trees_favor_ours(&base_tree, &ours_tree, &theirs_tree)?;

    // Write the index to a tree object
    // let tree_oid = index.write_tree_to(repo)?;
    // let merged_tree = repo.find_tree(tree_oid)?;

    // TODO Verify new version is correct (we should be getting a tree oid straight back from merge_trees_favor_ours)
    let tree_oid = repo.merge_trees_favor_ours(&base_tree, &ours_tree, &theirs_tree)?;
    let merged_tree = repo.find_tree(tree_oid)?;

    // Create the hanging merge commit with parents [ours, theirs]
    let merge_commit = repo.commit(
        None,
        &ours_commit.author()?,
        &ours_commit.committer()?,
        &format!(
            "Merge diff from {} to {} onto {}",
            origin_base, new_commit_parent, old_head_sha
        ),
        &merged_tree,
        &[&ours_commit, &theirs_commit],
    )?;

    Ok(merge_commit.to_string())
}

/// Delete a hanging commit that's not attached to any branch
///
/// This function removes a commit from the git object database. Since the commit
/// is hanging (not referenced by any branch or tag), it will be garbage collected
/// by git during the next gc operation.
///
/// # Arguments
/// * `repo` - Git repository
/// * `commit_sha` - SHA of the commit to delete
fn delete_hanging_commit(repo: &Repository, commit_sha: &str) -> Result<(), GitAiError> {
    // Find the commit to verify it exists
    let _commit = repo.find_commit(commit_sha.to_string())?;

    // Delete the commit using git command
    let _output = std::process::Command::new(crate::config::Config::get().git_cmd())
        .arg("update-ref")
        .arg("-d")
        .arg(format!("refs/heads/temp-{}", commit_sha))
        .current_dir(repo.path().parent().unwrap())
        .output()?;

    Ok(())
}

/// Reconstruct authorship history from a diff by running blame in the context of a hanging commit
///
/// This is the core logic that takes the diff between new_commit and new_commit_parent,
/// iterates through each file and hunk, and runs blame in the context of the hanging_commit_sha
/// to reconstruct the pre-squash authorship information.
///
/// # Arguments
/// * `repo` - Git repository
/// * `new_commit` - The new squashed commit
/// * `new_commit_parent` - The parent of the new commit
/// * `hanging_commit_sha` - The hanging commit that contains the pre-squash history
///
/// # Returns
/// A new AuthorshipLog with reconstructed authorship information
fn reconstruct_authorship_from_diff(
    repo: &Repository,
    new_commit: &Commit,
    new_commit_parent: &Commit,
    hanging_commit_sha: &str,
) -> Result<AuthorshipLog, GitAiError> {
    use std::collections::{HashMap, HashSet};

    // Get the trees for the diff
    let new_tree = new_commit.tree()?;
    let parent_tree = new_commit_parent.tree()?;

    // Create diff between new_commit and new_commit_parent
    let diff = repo.diff_tree_to_tree(Some(&parent_tree), Some(&new_tree), None)?;
    // Debug visibility for test runs
    #[cfg(test)]
    {
        eprintln!("files changed: {}", diff.deltas().len());
    }

    let mut authorship_entries = Vec::new();

    // Iterate through each file in the diff
    for delta in diff.deltas() {
        let old_file_path = delta.old_file().path();
        let new_file_path = delta.new_file().path();

        // Use the new file path if available, otherwise old file path
        let file_path = new_file_path
            .or(old_file_path)
            .ok_or_else(|| GitAiError::Generic("File path not available".to_string()))?;

        let file_path_str = file_path.to_string_lossy().to_string();

        // Get the content of the file from both trees
        let old_content =
            if let Ok(entry) = parent_tree.get_path(std::path::Path::new(&file_path_str)) {
                if let Ok(blob) = repo.find_blob(entry.id()) {
                    let content = blob.content()?;
                    String::from_utf8_lossy(&content).to_string()
                } else {
                    String::new()
                }
            } else {
                String::new()
            };

        let new_content = if let Ok(entry) = new_tree.get_path(std::path::Path::new(&file_path_str))
        {
            if let Ok(blob) = repo.find_blob(entry.id()) {
                let content = blob.content()?;
                String::from_utf8_lossy(&content).to_string()
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        // Pull the file content from the hanging commit to map inserted text to historical lines
        let hanging_commit = repo.find_commit(hanging_commit_sha.to_string())?;
        let hanging_tree = hanging_commit.tree()?;
        let hanging_content =
            if let Ok(entry) = hanging_tree.get_path(std::path::Path::new(&file_path_str)) {
                if let Ok(blob) = repo.find_blob(entry.id()) {
                    let content = blob.content()?;
                    String::from_utf8_lossy(&content).to_string()
                } else {
                    String::new()
                }
            } else {
                String::new()
            };

        // Create a text diff between the old and new content
        let diff = TextDiff::from_lines(&old_content, &new_content);
        let mut _old_line = 1u32;
        let mut new_line = 1u32;
        let hanging_lines: Vec<&str> = hanging_content.lines().collect();
        let mut used_hanging_line_numbers: HashSet<u32> = HashSet::new();

        for change in diff.iter_all_changes() {
            match change.tag() {
                ChangeTag::Equal => {
                    let line_count = change.value().lines().count() as u32;
                    _old_line += line_count;
                    new_line += line_count;
                }
                ChangeTag::Delete => {
                    // Deleted lines only advance the old line counter
                    _old_line += change.value().lines().count() as u32;
                }
                ChangeTag::Insert => {
                    let inserted: Vec<&str> = change.value().lines().collect();

                    // For each inserted line, try to find the same content in the hanging commit
                    for (i, inserted_line) in inserted.iter().enumerate() {
                        // Find a matching line number in hanging content, prefer the first not yet used
                        let mut matched_hanging_line: Option<u32> = None;
                        for (idx, h_line) in hanging_lines.iter().enumerate() {
                            if h_line == inserted_line {
                                let candidate = (idx as u32) + 1; // 1-indexed
                                if !used_hanging_line_numbers.contains(&candidate) {
                                    matched_hanging_line = Some(candidate);
                                    break;
                                }
                            }
                        }

                        let blame_line_number = if let Some(h_line_no) = matched_hanging_line {
                            used_hanging_line_numbers.insert(h_line_no);
                            h_line_no
                        } else {
                            // Fallback: use the position in the new file
                            new_line + (i as u32)
                        };

                        let blame_result = run_blame_in_context(
                            repo,
                            &file_path_str,
                            blame_line_number,
                            hanging_commit_sha,
                        )?;

                        if let Some((author, prompt)) = blame_result {
                            #[cfg(test)]
                            {
                                eprintln!(
                                    "ins line {} {} -> ai? {}",
                                    blame_line_number,
                                    author.username,
                                    prompt.is_some()
                                );
                            }
                            authorship_entries.push((
                                file_path_str.clone(),
                                blame_line_number,
                                author,
                                prompt,
                            ));
                        }
                    }

                    new_line += inserted.len() as u32;
                }
            }
        }
    }

    // Convert the collected entries into an AuthorshipLog
    let mut authorship_log = AuthorshipLog::new();

    // Group entries by file and prompt session ID for efficiency
    let mut file_attestations: HashMap<String, HashMap<String, Vec<u32>>> = HashMap::new();
    let mut prompt_records: HashMap<String, crate::log_fmt::authorship_log::PromptRecord> =
        HashMap::new();

    for (file_path, line_number, _author, prompt) in authorship_entries {
        // Only process AI-generated content (entries with prompt)
        if let Some((prompt_record, _turn)) = prompt {
            let prompt_session_id = prompt_record.agent_id.id.clone();

            // Store prompt record (preserving total_additions and total_deletions from original)
            prompt_records.insert(prompt_session_id.clone(), prompt_record);

            file_attestations
                .entry(file_path)
                .or_insert_with(HashMap::new)
                .entry(prompt_session_id)
                .or_insert_with(Vec::new)
                .push(line_number);
        }
    }

    // Convert grouped entries to AuthorshipLog format
    for (file_path, prompt_session_lines) in file_attestations {
        for (prompt_session_id, mut lines) in prompt_session_lines {
            // Sort lines and create ranges
            lines.sort();
            let mut ranges = Vec::new();
            let mut current_start = lines[0];
            let mut current_end = lines[0];

            for &line in &lines[1..] {
                if line == current_end + 1 {
                    // Extend current range
                    current_end = line;
                } else {
                    // Start new range
                    if current_start == current_end {
                        ranges.push(crate::log_fmt::authorship_log::LineRange::Single(
                            current_start,
                        ));
                    } else {
                        ranges.push(crate::log_fmt::authorship_log::LineRange::Range(
                            current_start,
                            current_end,
                        ));
                    }
                    current_start = line;
                    current_end = line;
                }
            }

            // Add the last range
            if current_start == current_end {
                ranges.push(crate::log_fmt::authorship_log::LineRange::Single(
                    current_start,
                ));
            } else {
                ranges.push(crate::log_fmt::authorship_log::LineRange::Range(
                    current_start,
                    current_end,
                ));
            }

            // Create attestation entry with the prompt session ID
            let attestation_entry =
                crate::log_fmt::authorship_log_serialization::AttestationEntry::new(
                    prompt_session_id.clone(),
                    ranges,
                );

            // Add to authorship log
            let file_attestation = authorship_log.get_or_create_file(&file_path);
            file_attestation.add_entry(attestation_entry);
        }
    }

    // Store prompt records in metadata (preserving total_additions and total_deletions)
    for (prompt_session_id, prompt_record) in prompt_records {
        authorship_log
            .metadata
            .prompts
            .insert(prompt_session_id, prompt_record);
    }

    // Sort attestation entries by hash for deterministic ordering
    for file_attestation in &mut authorship_log.attestations {
        file_attestation.entries.sort_by(|a, b| a.hash.cmp(&b.hash));
    }

    // Calculate accepted_lines for each prompt based on final attestation log
    let mut session_accepted_lines: HashMap<String, u32> = HashMap::new();
    for file_attestation in &authorship_log.attestations {
        for attestation_entry in &file_attestation.entries {
            let accepted_count: u32 = attestation_entry
                .line_ranges
                .iter()
                .map(|range| match range {
                    crate::log_fmt::authorship_log::LineRange::Single(_) => 1,
                    crate::log_fmt::authorship_log::LineRange::Range(start, end) => end - start + 1,
                })
                .sum();
            *session_accepted_lines
                .entry(attestation_entry.hash.clone())
                .or_insert(0) += accepted_count;
        }
    }

    // Update accepted_lines for all PromptRecords
    // Note: total_additions and total_deletions are preserved from the original prompt records
    for (session_id, prompt_record) in authorship_log.metadata.prompts.iter_mut() {
        prompt_record.accepted_lines = *session_accepted_lines.get(session_id).unwrap_or(&0);
    }

    Ok(authorship_log)
}

/// Run blame on a specific line in the context of a hanging commit and return AI authorship info
///
/// This function runs blame on a specific line number in a file, then looks up the AI authorship
/// log for the blamed commit to get the full authorship information including prompt details.
///
/// # Arguments
/// * `repo` - Git repository
/// * `file_path` - Path to the file
/// * `line_number` - Line number to blame (1-indexed)
/// * `hanging_commit_sha` - SHA of the hanging commit to use as context
///
/// # Returns
/// The AI authorship information (author and prompt) for the line, or None if not found
fn run_blame_in_context(
    repo: &Repository,
    file_path: &str,
    line_number: u32,
    hanging_commit_sha: &str,
) -> Result<
    Option<(
        crate::log_fmt::authorship_log::Author,
        Option<(crate::log_fmt::authorship_log::PromptRecord, u32)>,
    )>,
    GitAiError,
> {
    use crate::git::refs::get_reference_as_authorship_log_v3;
    use git2::BlameOptions;

    // println!(
    //     "Running blame in context for line {} in file {}",
    //     line_number, file_path
    // );

    // Find the hanging commit
    let hanging_commit = repo.find_commit(hanging_commit_sha.to_string())?;

    // Create blame options for the specific line
    let mut blame_opts = BlameOptions::new();
    blame_opts.min_line(line_number as usize);
    blame_opts.max_line(line_number as usize);
    blame_opts.newest_commit(hanging_commit.id()); // Set the hanging commit as the newest commit for blame

    // Run blame on the file in the context of the hanging commit
    let blame = repo.blame_file(std::path::Path::new(file_path), Some(&mut blame_opts))?;

    if blame.len() > 0 {
        let hunk = blame
            .get_index(0)
            .ok_or_else(|| GitAiError::Generic("Failed to get blame hunk".to_string()))?;

        let commit_id = hunk.final_commit_id();
        let commit_sha = commit_id.to_string();

        // Look up the AI authorship log for this commit
        let ref_name = format!("ai/authorship/{}", commit_sha);
        let authorship_log = match get_reference_as_authorship_log_v3(repo, &ref_name) {
            Ok(log) => log,
            Err(_) => {
                // No AI authorship data for this commit, fall back to git author
                let commit = repo.find_commit(commit_id)?;
                let author = commit.author()?;
                let author_name = author.name().unwrap_or("unknown");
                let author_email = author.email().unwrap_or("");

                let author_info = crate::log_fmt::authorship_log::Author {
                    username: author_name.to_string(),
                    email: author_email.to_string(),
                };

                return Ok(Some((author_info, None)));
            }
        };

        // Get the line attribution from the AI authorship log
        if let Some((author, prompt)) = authorship_log.get_line_attribution(file_path, line_number)
        {
            Ok(Some((author.clone(), prompt.map(|p| (p.clone(), 0)))))
        } else {
            // Line not found in authorship log, fall back to git author
            let commit = repo.find_commit(commit_id)?;
            let author = commit.author()?;
            let author_name = author.name().unwrap_or("unknown");
            let author_email = author.email().unwrap_or("");

            let author_info = crate::log_fmt::authorship_log::Author {
                username: author_name.to_string(),
                email: author_email.to_string(),
            };

            Ok(Some((author_info, None)))
        }
    } else {
        Ok(None)
    }
}

/// Find the common origin base between the head commit and the new commit's branch
fn find_common_origin_base_from_head(
    repo: &Repository,
    head_sha: &str,
    new_sha: &str,
) -> Result<String, GitAiError> {
    let new_commit = repo.find_commit(new_sha.to_string())?;
    let head_commit = repo.find_commit(head_sha.to_string())?;

    // Find the merge base between the head commit and the new commit
    let merge_base = repo.merge_base(head_commit.id(), new_commit.id())?;

    Ok(merge_base.to_string())
}

/// Build a path of commit SHAs from head_sha to the origin base
///
/// This function walks the commit history from head_sha backwards until it reaches
/// the origin_base, collecting all commit SHAs in the path. If no valid linear path
/// exists (incompatible lineage), it returns an error.
///
/// # Arguments
/// * `repo` - Git repository
/// * `head_sha` - SHA of the HEAD commit to start from
/// * `origin_base` - SHA of the origin base commit to walk to
///
/// # Returns
/// A vector of commit SHAs in chronological order (oldest first) representing
/// the path from just after origin_base to head_sha
fn build_commit_path_to_base(
    repo: &Repository,
    head_sha: &str,
    origin_base: &str,
) -> Result<Vec<String>, GitAiError> {
    let head_commit = repo.find_commit(head_sha.to_string())?;

    let mut commits = Vec::new();
    let mut current_commit = head_commit;

    // Walk backwards from head to origin_base
    loop {
        // If we've reached the origin base, we're done
        if current_commit.id() == origin_base.to_string() {
            break;
        }

        // Add current commit to our path
        commits.push(current_commit.id().to_string());

        // Move to parent commit
        match current_commit.parent(0) {
            Ok(parent) => current_commit = parent,
            Err(_) => {
                return Err(GitAiError::Generic(format!(
                    "Incompatible lineage: no path from {} to {}. Reached end of history without finding origin base.",
                    head_sha, origin_base
                )));
            }
        }

        // Safety check: avoid infinite loops in case of circular references
        if commits.len() > 10000 {
            return Err(GitAiError::Generic(
                "Incompatible lineage: path too long, possible circular reference".to_string(),
            ));
        }
    }

    // If we have no commits, head_sha and origin_base are the same
    if commits.is_empty() {
        return Err(GitAiError::Generic(format!(
            "Incompatible lineage: head_sha ({}) and origin_base ({}) are the same commit",
            head_sha, origin_base
        )));
    }

    // Reverse to get chronological order (oldest first)
    commits.reverse();

    Ok(commits)
}

pub fn handle_squash_authorship(args: &[String]) {
    // Parse squash-authorship-specific arguments
    let mut dry_run = false;
    let mut branch = None;
    let mut new_sha = None;
    let mut old_sha = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--dry-run" => {
                dry_run = true;
                i += 1;
            }
            _ => {
                // Positional arguments: branch, new_sha, old_sha
                if branch.is_none() {
                    branch = Some(args[i].clone());
                } else if new_sha.is_none() {
                    new_sha = Some(args[i].clone());
                } else if old_sha.is_none() {
                    old_sha = Some(args[i].clone());
                } else {
                    eprintln!("Unknown squash-authorship argument: {}", args[i]);
                    std::process::exit(1);
                }
                i += 1;
            }
        }
    }

    // Validate required arguments
    let branch = match branch {
        Some(b) => b,
        None => {
            eprintln!("Error: branch argument is required");
            eprintln!("Usage: git-ai squash-authorship <branch> <new_sha> <old_sha> [--dry-run]");
            std::process::exit(1);
        }
    };

    let new_sha = match new_sha {
        Some(s) => s,
        None => {
            eprintln!("Error: new_sha argument is required");
            eprintln!("Usage: git-ai squash-authorship <branch> <new_sha> <old_sha> [--dry-run]");
            std::process::exit(1);
        }
    };

    let old_sha = match old_sha {
        Some(s) => s,
        None => {
            eprintln!("Error: old_sha argument is required");
            eprintln!("Usage: git-ai squash-authorship <branch> <new_sha> <old_sha> [--dry-run]");
            std::process::exit(1);
        }
    };

    // TODO Think about whether or not path should be an optional argument

    // Find the git repository
    let repo = match find_repository(vec!["-C".to_string(), ".".to_string()]) {
        Ok(repo) => repo,
        Err(e) => {
            eprintln!("Failed to find repository: {}", e);
            std::process::exit(1);
        }
    };

    if let Err(e) =
        rewrite_authorship_after_squash_or_rebase(&repo, &branch, &old_sha, &new_sha, dry_run)
    {
        eprintln!("Squash authorship failed: {}", e);
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use insta::assert_debug_snapshot;

    #[test]
    fn test_in_order() {
        let repo =
            find_repository(vec!["-C".to_string(), "tests/gitflow-repo".to_string()]).unwrap();

        let new_sha = "22ab8a64bee45d9292f680f07a8f79234c2cb9d5";
        let destination_branch = "origin/main";
        let head_sha: &'static str = "245606961b859e5ab5593b276f47e26f7c32e4d6"; // The HEAD of the original squashed commits

        let authorship_log = rewrite_authorship_after_squash_or_rebase(
            &repo,
            &destination_branch,
            &head_sha,
            &new_sha,
            true,
        )
        .unwrap();

        print!("{:?}", authorship_log);

        assert_debug_snapshot!(authorship_log);
    }

    #[test]
    fn test_with_out_of_band_commits() {
        let repo =
            find_repository(vec!["-C".to_string(), "tests/gitflow-repo".to_string()]).unwrap();

        let new_sha = "13b1a9736d8caaec665b29b2c6792b2eba1fb169";
        let destination_branch = "origin/main";
        let head_sha = "6d781a058490d5c7ba10902492f65f85e4f4c3bb"; // The HEAD of the original squashed commits

        let authorship_log = rewrite_authorship_after_squash_or_rebase(
            &repo,
            &destination_branch,
            &head_sha,
            &new_sha,
            true,
        )
        .unwrap();

        assert_debug_snapshot!(authorship_log);
    }
}
