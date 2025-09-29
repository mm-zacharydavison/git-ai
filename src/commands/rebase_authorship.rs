use crate::error::GitAiError;
use crate::git::find_repository;
use crate::log_fmt::authorship_log::AuthorshipLog;
use crate::utils::print_diff;
use git2::{Oid, Repository};
use serde_json;
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
    let old_shas = build_commit_path_to_base(repo, head_sha, &origin_base)?;

    // Step 3: Get the parent of the new commit
    let new_commit = repo.find_commit(Oid::from_str(new_sha)?)?;
    let new_commit_parent = new_commit.parent(0)?;

    // Step 4: Compute a diff between origin_base and new_commit_parent. Sometimes it's the same
    // sha. that's ok
    let origin_base_commit = repo.find_commit(Oid::from_str(&origin_base)?)?;
    let origin_base_tree = origin_base_commit.tree()?;
    let new_commit_parent_tree = new_commit_parent.tree()?;

    // Create diff between the two trees
    let diff =
        repo.diff_tree_to_tree(Some(&origin_base_tree), Some(&new_commit_parent_tree), None)?;

    // Print the diff in a readable format

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
        let authorship_json = if cfg!(debug_assertions) {
            serde_json::to_string_pretty(&new_authorship_log)?
        } else {
            serde_json::to_string(&new_authorship_log)?
        };

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
    // Get the commits
    let new_commit_parent_commit = repo.find_commit(Oid::from_str(new_commit_parent)?)?;
    let old_head_commit = repo.find_commit(Oid::from_str(old_head_sha)?)?;

    // Get the tree for the old head (we'll use this as the result tree)
    let old_head_tree = old_head_commit.tree()?;

    // Create a merge commit with the old head as the main parent and new commit parent as the second parent
    // This creates a merge commit that represents applying the diff to the old head
    let merge_commit = repo.commit(
        None, // Let git choose the reference (we'll create a hanging commit)
        &old_head_commit.author(),
        &old_head_commit.committer(),
        &format!(
            "Merge diff from {} to {} onto {}",
            origin_base, new_commit_parent, old_head_sha
        ),
        &old_head_tree, // Use old head tree as the result (Accept Theirs strategy)
        &[&old_head_commit, &new_commit_parent_commit], // Parents: old head first, then new commit parent
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
    let _commit = repo.find_commit(Oid::from_str(commit_sha)?)?;

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
    new_commit: &git2::Commit,
    new_commit_parent: &git2::Commit,
    hanging_commit_sha: &str,
) -> Result<AuthorshipLog, GitAiError> {
    use std::collections::HashMap;

    // Get the trees for the diff
    let new_tree = new_commit.tree()?;
    let parent_tree = new_commit_parent.tree()?;

    // Create diff between new_commit and new_commit_parent
    let diff = repo.diff_tree_to_tree(Some(&parent_tree), Some(&new_tree), None)?;

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
                    String::from_utf8_lossy(blob.content()).to_string()
                } else {
                    String::new()
                }
            } else {
                String::new()
            };

        let new_content = if let Ok(entry) = new_tree.get_path(std::path::Path::new(&file_path_str))
        {
            if let Ok(blob) = repo.find_blob(entry.id()) {
                String::from_utf8_lossy(blob.content()).to_string()
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
                    let insert_count = change.value().lines().count() as u32;

                    // For each inserted line, run blame in the context of the hanging commit
                    // We use the new_line position since we're working with the new file
                    for line_offset in 0..insert_count {
                        let line_number = new_line + line_offset;

                        // Run blame on this specific line in the context of the hanging commit
                        let blame_result = run_blame_in_context(
                            repo,
                            &file_path_str,
                            line_number,
                            hanging_commit_sha,
                        )?;

                        if let Some((author, prompt)) = blame_result {
                            // Add this line to our authorship log with prompt info
                            authorship_entries.push((
                                file_path_str.clone(),
                                line_number,
                                author,
                                prompt,
                            ));
                        }
                    }

                    // Inserted lines only advance the new line counter
                    new_line += insert_count;
                }
            }
        }
    }

    // Convert the collected entries into an AuthorshipLog
    let mut authorship_log = AuthorshipLog::new();

    // Store original entries for later reference
    let original_entries = authorship_entries.clone();

    // Group entries by file, author, and prompt for efficiency
    // Use a string key to avoid hash issues with complex types
    let mut file_authors: HashMap<String, HashMap<String, Vec<u32>>> = HashMap::new();

    for (file_path, line_number, author, prompt) in authorship_entries {
        // Create a unique key for this author+prompt combination
        let author_key = AuthorshipLog::generate_author_key(&author);
        let prompt_key = if let Some((prompt_record, turn)) = prompt {
            format!("{}:{}:{}", author_key, prompt_record.agent_id.id, turn)
        } else {
            author_key.clone()
        };

        file_authors
            .entry(file_path)
            .or_insert_with(HashMap::new)
            .entry(prompt_key)
            .or_insert_with(Vec::new)
            .push(line_number);
    }

    // Convert grouped entries to AuthorshipLog format
    for (file_path, prompt_key_lines) in file_authors {
        for (prompt_key, mut lines) in prompt_key_lines {
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

            // Parse the prompt key to extract author and prompt info
            let parts: Vec<&str> = prompt_key.split(':').collect();
            let (author_key, prompt_session_id, prompt_turn) = if parts.len() == 3 {
                // Has prompt info: "author_key:prompt_id:turn"
                (
                    parts[0].to_string(),
                    Some(parts[1].to_string()),
                    Some(parts[2].parse::<u32>().unwrap_or(0)),
                )
            } else {
                // No prompt info: just "author_key"
                (prompt_key.clone(), None, None)
            };

            // Find the author info from the original entries
            let author = original_entries
                .iter()
                .find(|(_, _, a, _)| AuthorshipLog::generate_author_key(a) == author_key)
                .map(|(_, _, a, _)| a.clone())
                .unwrap_or_else(|| {
                    // Fallback author if not found
                    crate::log_fmt::authorship_log::Author {
                        username: "unknown".to_string(),
                        email: "".to_string(),
                    }
                });

            // Store author info
            authorship_log.authors.insert(author_key.clone(), author);

            // Store prompt info if available
            if let Some(session_id) = &prompt_session_id {
                // Find the prompt record from the original entries
                if let Some((_, _, _, Some((prompt_record, _)))) =
                    original_entries.iter().find(|(_, _, _, p)| {
                        p.as_ref()
                            .map(|(pr, _)| pr.agent_id.id == *session_id)
                            .unwrap_or(false)
                    })
                {
                    authorship_log
                        .prompts
                        .insert(session_id.clone(), prompt_record.clone());
                }
            }

            // Add attribution entry
            let file_entries = authorship_log.get_or_create_file(&file_path);
            file_entries.push(crate::log_fmt::authorship_log::AttributionEntry {
                lines: ranges,
                author_key,
                prompt_session_id,
                prompt_turn,
            });
        }
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
    use crate::git::refs::get_reference_as_authorship_log;
    use git2::{BlameOptions, Oid};

    // println!(
    //     "Running blame in context for line {} in file {}",
    //     line_number, file_path
    // );

    // Find the hanging commit
    let hanging_commit = repo.find_commit(Oid::from_str(hanging_commit_sha)?)?;

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
        let authorship_log = match get_reference_as_authorship_log(repo, &ref_name) {
            Ok(log) => log,
            Err(_) => {
                // No AI authorship data for this commit, fall back to git author
                let commit = repo.find_commit(commit_id)?;
                let author = commit.author();
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
            Ok(Some((author.clone(), prompt.map(|(p, t)| (p.clone(), t)))))
        } else {
            // Line not found in authorship log, fall back to git author
            let commit = repo.find_commit(commit_id)?;
            let author = commit.author();
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
    let new_commit = repo.find_commit(Oid::from_str(new_sha)?)?;
    let head_commit = repo.find_commit(Oid::from_str(head_sha)?)?;

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
    let head_commit = repo.find_commit(Oid::from_str(head_sha)?)?;
    let origin_base_oid = Oid::from_str(origin_base)?;

    let mut commits = Vec::new();
    let mut current_commit = head_commit;

    // Walk backwards from head to origin_base
    loop {
        // If we've reached the origin base, we're done
        if current_commit.id() == origin_base_oid {
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

    // Find the git repository
    let repo = match find_repository() {
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
    use git2::Repository;
    use insta::assert_debug_snapshot;

    #[test]
    fn test_in_order() {
        let repo = Repository::discover("tests/gitflow-repo").unwrap();

        let new_sha = "78788430844d8ccc064e7da1327c374402efc232";
        let destination_branch = "origin/main";
        let head_sha: &'static str = "bd57bd9e25df41cf1f6c2875b787b3b4b01cbe5b"; // The HEAD of the original squashed commits

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

    #[test]
    fn test_with_out_of_band_commits() {
        let repo = Repository::discover("tests/gitflow-repo").unwrap();

        let new_sha = "09b999d49bf248aabb2cd9ef987e030551b7002e";
        let destination_branch = "origin/main";
        let head_sha = "87be297c9bebb904d877bc856c34419eeb0e979c"; // The HEAD of the original squashed commits

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
