use crate::error::GitAiError;
use crate::git::refs::get_reference_as_authorship_log;
use crate::log_fmt::authorship_log::AuthorshipLog;
use git2::{BlameOptions, Repository};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
#[derive(Debug, Clone)]
struct BlameHunk {
    /// Line range [start, end] (inclusive)
    range: (u32, u32),
    /// Commit SHA that introduced this hunk
    commit_sha: String,
    /// Original author from Git blame
    original_author: String,
    /// Author from our authorship log (if available)
    ai_author: Option<String>,
}

pub fn run(
    repo: &Repository,
    file_path: &str,
    line_range: Option<(u32, u32)>,
) -> Result<(), GitAiError> {
    // Validate that the file exists
    let file_path_obj = Path::new(file_path);
    if !file_path_obj.exists() {
        return Err(GitAiError::Generic(format!(
            "File not found: {}",
            file_path
        )));
    }

    // Read the current file content
    let file_content = fs::read_to_string(file_path)?;
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

    // Step 1: Get Git's native blame
    let blame_hunks = get_git_blame_hunks(repo, file_path, start_line, end_line)?;

    // Step 2: Overlay AI authorship information
    let enhanced_hunks = overlay_ai_authorship(repo, &blame_hunks, file_path)?;

    // Calculate the maximum author name length for dynamic column width
    let mut max_author_len = 0;
    for line_num in start_line..=end_line {
        let hunk = enhanced_hunks
            .iter()
            .find(|h| line_num >= h.range.0 && line_num <= h.range.1);

        let author = if let Some(h) = hunk {
            h.ai_author.as_ref().unwrap_or(&h.original_author)
        } else {
            "unknown"
        };

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

        // Find the hunk that contains this line
        let hunk = enhanced_hunks
            .iter()
            .find(|h| line_num >= h.range.0 && line_num <= h.range.1);

        let author = if let Some(h) = hunk {
            // Prefer AI author if available, otherwise fall back to original author
            h.ai_author.as_ref().unwrap_or(&h.original_author)
        } else {
            "unknown"
        };

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
    print_blame_summary(&enhanced_hunks, start_line, end_line);

    Ok(())
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
            ai_author: None, // Will be filled in later
        });
    }

    Ok(hunks)
}

fn overlay_ai_authorship(
    repo: &Repository,
    blame_hunks: &[BlameHunk],
    file_path: &str,
) -> Result<Vec<BlameHunk>, GitAiError> {
    let mut enhanced_hunks = blame_hunks.to_vec();

    // Group hunks by commit SHA to avoid repeated lookups
    let mut commit_authorship_cache: HashMap<String, Option<AuthorshipLog>> = HashMap::new();

    for hunk in &mut enhanced_hunks {
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
                // Check if any lines in this hunk have AI authorship
                let mut ai_authors = Vec::new();
                for line_num in hunk.range.0..=hunk.range.1 {
                    if let Some(author) = file_authorship.get_author(line_num) {
                        ai_authors.push(author.to_string());
                    }
                }

                // If we found AI authorship for any lines in this hunk, use the most common one
                if !ai_authors.is_empty() {
                    // Count occurrences of each author
                    let mut author_counts: HashMap<String, usize> = HashMap::new();
                    for author in ai_authors {
                        *author_counts.entry(author).or_insert(0) += 1;
                    }

                    // Find the most common author
                    if let Some((most_common_author, _)) =
                        author_counts.iter().max_by_key(|(_, count)| **count)
                    {
                        hunk.ai_author = Some(most_common_author.clone());
                    }
                }
            }
        }
    }

    Ok(enhanced_hunks)
}

#[allow(unused_variables)]
fn print_blame_summary(hunks: &[BlameHunk], start_line: u32, end_line: u32) {
    println!("{}", "=".repeat(80));

    let mut author_stats: HashMap<String, u32> = HashMap::new();
    let mut total_lines = 0;

    for hunk in hunks {
        let lines_in_hunk = hunk.range.1 - hunk.range.0 + 1;
        let author = hunk.ai_author.as_ref().unwrap_or(&hunk.original_author);
        *author_stats.entry(author.clone()).or_insert(0) += lines_in_hunk;
        total_lines += lines_in_hunk;
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
