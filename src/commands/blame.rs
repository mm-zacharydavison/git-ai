use crate::error::GitAiError;
use crate::git::refs::get_reference_as_authorship_log;
use crate::log_fmt::authorship_log::AuthorshipLog;
use git2::{BlameOptions, Repository};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
#[derive(Debug, Clone)]
pub struct BlameHunk {
    /// Line range [start, end] (inclusive)
    pub range: (u32, u32),
    /// Commit SHA that introduced this hunk
    pub commit_sha: String,
    /// Original author from Git blame
    pub original_author: String,
    // ai_author: Option<String>,
}

pub fn run(
    repo: &Repository,
    file_path: &str,
    line_range: Option<(u32, u32)>,
) -> Result<HashMap<u32, String>, GitAiError> {
    // Use repo root for file system operations
    let repo_root = repo
        .workdir()
        .ok_or_else(|| GitAiError::Generic("Repository has no working directory".to_string()))?;
    let abs_file_path = repo_root.join(file_path);

    // Validate that the file exists
    if !abs_file_path.exists() {
        return Err(GitAiError::Generic(format!(
            "File not found: {}",
            abs_file_path.display()
        )));
    }

    // Read the current file content
    let file_content = fs::read_to_string(&abs_file_path)?;
    let lines: Vec<&str> = file_content.lines().collect();
    let total_lines = lines.len() as u32;

    // Determine the line range to process
    let (start_line, end_line) = match line_range {
        Some((start, end)) => {
            if start == 0 || end == 0 || start > end || end > total_lines {
                return Err(GitAiError::Generic(format!(
                    "Invalid line range: {}:{}. File has {} lines",
                    start, end, total_lines
                )));
            }
            (start, end)
        }
        None => (1, total_lines),
    };

    // Step 1: Get Git's native blame (still use relative path)
    let blame_hunks = get_git_blame_hunks(repo, file_path, start_line, end_line)?;

    // Step 2: Overlay AI authorship information
    let line_authors = overlay_ai_authorship(repo, &blame_hunks, file_path)?;

    // Calculate the maximum author name length for dynamic column width
    let mut max_author_len = 0;
    for line_num in start_line..=end_line {
        let author = line_authors
            .get(&line_num)
            .map(|s| s.as_str())
            .unwrap_or("unknown");
        max_author_len = max_author_len.max(author.len());
    }

    // Print the blame output
    println!(
        "Blame for {} (lines {}-{})",
        file_path, start_line, end_line
    );
    println!("{}", "=".repeat(80));

    for line_num in start_line..=end_line {
        let line_index = (line_num - 1) as usize;
        let line_content = if line_index < lines.len() {
            lines[line_index]
        } else {
            ""
        };

        // Get the author for this specific line
        let author = line_authors
            .get(&line_num)
            .map(|s| s.as_str())
            .unwrap_or("unknown");

        // Format: author line_number line_content
        println!(
            "{:>width$} {:>4}) {}",
            author,
            line_num,
            line_content,
            width = max_author_len
        );
    }

    // Print summary
    print_blame_summary(&line_authors, start_line, end_line);

    Ok(line_authors)
}

fn get_git_blame_hunks(
    repo: &Repository,
    file_path: &str,
    start_line: u32,
    end_line: u32,
) -> Result<Vec<BlameHunk>, GitAiError> {
    let mut blame_opts = BlameOptions::new();
    blame_opts.min_line(start_line.try_into().unwrap());
    blame_opts.max_line(end_line.try_into().unwrap());

    let blame = repo.blame_file(Path::new(file_path), Some(&mut blame_opts))?;
    let mut hunks = Vec::new();

    let num_hunks = blame.len();
    for i in 0..num_hunks {
        let hunk = blame
            .get_index(i)
            .ok_or_else(|| GitAiError::Generic("Failed to get blame hunk".to_string()))?;

        let start = hunk.final_start_line(); // Already 1-indexed
        let end = start + hunk.lines_in_hunk() - 1;

        let commit = match repo.find_commit(hunk.final_commit_id()) {
            Ok(commit) => commit,
            Err(_) => {
                continue; // Skip this hunk if we can't find the commit
            }
        };

        let author = commit.author().name().unwrap_or("unknown").to_string();
        let commit_sha = hunk.final_commit_id().to_string();

        hunks.push(BlameHunk {
            range: (start.try_into().unwrap(), end.try_into().unwrap()),
            commit_sha,
            original_author: author,
        });
    }

    Ok(hunks)
}

fn overlay_ai_authorship(
    repo: &Repository,
    blame_hunks: &[BlameHunk],
    file_path: &str,
) -> Result<HashMap<u32, String>, GitAiError> {
    let mut line_authors: HashMap<u32, String> = HashMap::new();

    // Group hunks by commit SHA to avoid repeated lookups
    let mut commit_authorship_cache: HashMap<String, Option<AuthorshipLog>> = HashMap::new();

    for hunk in blame_hunks {
        // Check if we've already looked up this commit's authorship
        let authorship_log = if let Some(cached) = commit_authorship_cache.get(&hunk.commit_sha) {
            cached.clone()
        } else {
            // Try to get authorship log for this commit
            let ref_name = format!("ai/authorship/{}", hunk.commit_sha);
            let authorship = match get_reference_as_authorship_log(repo, &ref_name) {
                Ok(log) => Some(log),
                Err(_) => None, // No AI authorship data for this commit
            };
            commit_authorship_cache.insert(hunk.commit_sha.clone(), authorship.clone());
            authorship
        };

        // If we have AI authorship data, look up the author for lines in this hunk
        if let Some(authorship_log) = authorship_log {
            if let Some(file_authorship) = authorship_log.files.get(file_path) {
                // Check each line in this hunk for AI authorship
                for line_num in hunk.range.0..=hunk.range.1 {
                    if let Some(author) = file_authorship.get_author(line_num) {
                        line_authors.insert(line_num, author.to_string());
                    } else {
                        // Fall back to original author if no AI authorship
                        line_authors.insert(line_num, hunk.original_author.clone());
                    }
                }
            } else {
                // No file authorship data, use original author for all lines in hunk
                for line_num in hunk.range.0..=hunk.range.1 {
                    line_authors.insert(line_num, hunk.original_author.clone());
                }
            }
        } else {
            // No authorship log, use original author for all lines in hunk
            for line_num in hunk.range.0..=hunk.range.1 {
                line_authors.insert(line_num, hunk.original_author.clone());
            }
        }
    }

    Ok(line_authors)
}

#[allow(unused_variables)]
fn print_blame_summary(line_authors: &HashMap<u32, String>, start_line: u32, end_line: u32) {
    println!("{}", "=".repeat(80));

    let mut author_stats: HashMap<String, u32> = HashMap::new();
    let mut total_lines = 0;

    for line_num in start_line..=end_line {
        let author = line_authors
            .get(&line_num)
            .map(|s| s.as_str())
            .unwrap_or("unknown");
        *author_stats.entry(author.to_string()).or_insert(0) += 1;
        total_lines += 1;
    }

    // Find the longest author name for column width
    let max_author_len = author_stats
        .keys()
        .map(|name| name.len())
        .max()
        .unwrap_or(0);

    // Sort authors by line count (descending)
    let mut sorted_authors: Vec<_> = author_stats.iter().collect();
    sorted_authors.sort_by(|a, b| b.1.cmp(a.1));

    for (author, count) in sorted_authors {
        let percentage = (*count as f64 / total_lines as f64) * 100.0;
        println!(
            "{:>width$} {:>5} {:>8.1}%",
            author,
            count,
            percentage,
            width = max_author_len
        );
    }
}
