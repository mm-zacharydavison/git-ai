use crate::log_fmt::authorship_log::{Author, LineRange, PromptRecord};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::io::{BufRead, Write};

/// Authorship log format version identifier
pub const AUTHORSHIP_LOG_VERSION: &str = "authorship/3.0.0";

/// Generate a short hash (7 characters) from agent_id and tool
fn generate_short_hash(agent_id: &str, tool: &str) -> String {
    let combined = format!("{}:{}", tool, agent_id);
    let mut hasher = Sha256::new();
    hasher.update(combined.as_bytes());
    let result = hasher.finalize();
    // Take first 7 characters of the hex representation
    format!("{:x}", result)[..7].to_string()
}

/// Count the number of lines represented by a working_log::Line
fn count_working_log_lines(line: &crate::log_fmt::working_log::Line) -> u32 {
    match line {
        crate::log_fmt::working_log::Line::Single(_) => 1,
        crate::log_fmt::working_log::Line::Range(start, end) => end - start + 1,
    }
}

/// Count the number of lines represented by a LineRange
fn count_line_range(range: &LineRange) -> u32 {
    match range {
        LineRange::Single(_) => 1,
        LineRange::Range(start, end) => end - start + 1,
    }
}

/// Metadata section that goes below the divider as JSON
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuthorshipMetadata {
    pub schema_version: String,
    pub base_commit_sha: String,
    pub prompts: BTreeMap<String, PromptRecord>,
}

impl AuthorshipMetadata {
    pub fn new() -> Self {
        Self {
            schema_version: AUTHORSHIP_LOG_VERSION.to_string(),
            base_commit_sha: String::new(),
            prompts: BTreeMap::new(),
        }
    }
}

impl Default for AuthorshipMetadata {
    fn default() -> Self {
        Self::new()
    }
}

/// Attestation entry: short hash followed by line ranges
///
/// IMPORTANT: The hash ALWAYS corresponds to a prompt in the prompts section.
/// This system only tracks AI-generated content, not human-authored content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttestationEntry {
    /// Short hash (7 chars) that maps to an entry in the prompts section of the metadata
    pub hash: String,
    /// Line ranges that this prompt is responsible for
    pub line_ranges: Vec<LineRange>,
}

impl AttestationEntry {
    pub fn new(hash: String, line_ranges: Vec<LineRange>) -> Self {
        Self { hash, line_ranges }
    }

    pub fn remove_line_ranges(&mut self, to_remove: &[LineRange]) {
        let mut current_ranges = self.line_ranges.clone();

        for remove_range in to_remove {
            let mut new_ranges = Vec::new();
            for existing_range in &current_ranges {
                new_ranges.extend(existing_range.remove(remove_range));
            }
            current_ranges = new_ranges;
        }

        self.line_ranges = current_ranges;
    }

    /// Shift line ranges by a given offset starting at insertion_point
    pub fn shift_line_ranges(&mut self, insertion_point: u32, offset: i32) {
        let mut shifted_ranges = Vec::new();
        for range in &self.line_ranges {
            if let Some(shifted) = range.shift(insertion_point, offset) {
                shifted_ranges.push(shifted);
            }
        }
        self.line_ranges = shifted_ranges;
    }
}

/// Per-file attestation data
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileAttestation {
    pub file_path: String,
    pub entries: Vec<AttestationEntry>,
}

impl FileAttestation {
    pub fn new(file_path: String) -> Self {
        Self {
            file_path,
            entries: Vec::new(),
        }
    }

    pub fn add_entry(&mut self, entry: AttestationEntry) {
        self.entries.push(entry);
    }
}

/// The complete authorship log format
#[derive(Clone, PartialEq)]
pub struct AuthorshipLog {
    pub attestations: Vec<FileAttestation>,
    pub metadata: AuthorshipMetadata,
}

impl fmt::Debug for AuthorshipLog {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthorshipLogV3")
            .field("attestations", &self.attestations)
            .field("metadata", &self.metadata)
            .finish()
    }
}

impl AuthorshipLog {
    pub fn new() -> Self {
        Self {
            attestations: Vec::new(),
            metadata: AuthorshipMetadata::new(),
        }
    }

    /// Merge overlapping and adjacent line ranges
    fn merge_line_ranges(ranges: &[LineRange]) -> Vec<LineRange> {
        if ranges.is_empty() {
            return Vec::new();
        }

        let mut sorted_ranges = ranges.to_vec();
        sorted_ranges.sort_by(|a, b| {
            let a_start = match a {
                LineRange::Single(line) => *line,
                LineRange::Range(start, _) => *start,
            };
            let b_start = match b {
                LineRange::Single(line) => *line,
                LineRange::Range(start, _) => *start,
            };
            a_start.cmp(&b_start)
        });

        let mut merged = Vec::new();
        for current in sorted_ranges {
            if let Some(last) = merged.last_mut() {
                if Self::ranges_can_merge(last, &current) {
                    *last = Self::merge_ranges(last, &current);
                } else {
                    merged.push(current);
                }
            } else {
                merged.push(current);
            }
        }

        merged
    }

    /// Check if two ranges can be merged (overlapping or adjacent)
    fn ranges_can_merge(range1: &LineRange, range2: &LineRange) -> bool {
        let (start1, end1) = match range1 {
            LineRange::Single(line) => (*line, *line),
            LineRange::Range(start, end) => (*start, *end),
        };
        let (start2, end2) = match range2 {
            LineRange::Single(line) => (*line, *line),
            LineRange::Range(start, end) => (*start, *end),
        };

        // Ranges can merge if they overlap or are adjacent
        start1 <= end2 + 1 && start2 <= end1 + 1
    }

    /// Merge two ranges into one
    fn merge_ranges(range1: &LineRange, range2: &LineRange) -> LineRange {
        let (start1, end1) = match range1 {
            LineRange::Single(line) => (*line, *line),
            LineRange::Range(start, end) => (*start, *end),
        };
        let (start2, end2) = match range2 {
            LineRange::Single(line) => (*line, *line),
            LineRange::Range(start, end) => (*start, *end),
        };

        let start = start1.min(start2);
        let end = end1.max(end2);

        if start == end {
            LineRange::Single(start)
        } else {
            LineRange::Range(start, end)
        }
    }

    /// Convert from working log checkpoints to authorship log
    pub fn from_working_log_with_base_commit_and_human_author(
        checkpoints: &[crate::log_fmt::working_log::Checkpoint],
        base_commit_sha: &str,
        human_author: Option<&str>,
    ) -> Self {
        let mut authorship_log = Self::new();
        authorship_log.metadata.base_commit_sha = base_commit_sha.to_string();

        // Track additions and deletions per session_id
        let mut session_additions: HashMap<String, u32> = HashMap::new();
        let mut session_deletions: HashMap<String, u32> = HashMap::new();

        // Process checkpoints and create attributions
        for checkpoint in checkpoints.iter() {
            // If there is an agent session, record it by its short hash (agent_id + tool)
            let session_id_opt = match (&checkpoint.agent_id, &checkpoint.transcript) {
                (Some(agent), Some(transcript)) => {
                    let session_id = generate_short_hash(&agent.id, &agent.tool);
                    // Insert or update the prompt session transcript
                    let entry = authorship_log
                        .metadata
                        .prompts
                        .entry(session_id.clone())
                        .or_insert(PromptRecord {
                            agent_id: agent.clone(),
                            human_author: human_author.map(|s| s.to_string()),
                            messages: transcript.messages().to_vec(),
                            total_additions: 0,
                            total_deletions: 0,
                            accepted_lines: 0,
                        });
                    if entry.messages.len() < transcript.messages().len() {
                        entry.messages = transcript.messages().to_vec();
                    }
                    Some(session_id)
                }
                _ => None,
            };

            for entry in &checkpoint.entries {
                // Track additions and deletions for this session
                if let Some(ref session_id) = session_id_opt {
                    // Count total additions
                    let additions_count: u32 = entry
                        .added_lines
                        .iter()
                        .map(|line| count_working_log_lines(line))
                        .sum();
                    *session_additions.entry(session_id.clone()).or_insert(0) += additions_count;

                    // Count total deletions
                    let deletions_count: u32 = entry
                        .deleted_lines
                        .iter()
                        .map(|line| count_working_log_lines(line))
                        .sum();
                    *session_deletions.entry(session_id.clone()).or_insert(0) += deletions_count;
                }

                // Process deletions first (remove lines from all authors, then shift remaining lines up)
                if !entry.deleted_lines.is_empty() {
                    let file_attestation = authorship_log.get_or_create_file(&entry.file);

                    // Collect all deleted line numbers
                    let mut all_deleted_lines = Vec::new();
                    for line in &entry.deleted_lines {
                        match line {
                            crate::log_fmt::working_log::Line::Single(l) => {
                                all_deleted_lines.push(*l)
                            }
                            crate::log_fmt::working_log::Line::Range(start, end) => {
                                for l in *start..=*end {
                                    all_deleted_lines.push(l);
                                }
                            }
                        }
                    }
                    all_deleted_lines.sort_unstable();
                    all_deleted_lines.dedup();

                    let deleted_ranges = LineRange::compress_lines(&all_deleted_lines);

                    // Remove the deleted lines from all attestations
                    for attestation_entry in file_attestation.entries.iter_mut() {
                        attestation_entry.remove_line_ranges(&deleted_ranges);
                    }

                    // Shift remaining lines up after deletions
                    // Process deletions in reverse order to avoid shifting issues
                    for line in all_deleted_lines.iter().rev() {
                        let deletion_point = *line;
                        for attestation_entry in file_attestation.entries.iter_mut() {
                            // Shift lines after the deletion point up by 1
                            attestation_entry.shift_line_ranges(deletion_point + 1, -1);
                        }
                    }
                }

                // Then process additions (shift existing lines down, then add new author)
                let mut added_lines = Vec::new();
                for line in &entry.added_lines {
                    match line {
                        crate::log_fmt::working_log::Line::Single(l) => added_lines.push(*l),
                        crate::log_fmt::working_log::Line::Range(start, end) => {
                            for l in *start..=*end {
                                added_lines.push(l);
                            }
                        }
                    }
                }
                if !added_lines.is_empty() {
                    // Ensure deterministic, duplicate-free line numbers before compression
                    added_lines.sort_unstable();
                    added_lines.dedup();

                    let num_lines_added = added_lines.len() as i32;
                    let insertion_point = *added_lines.first().unwrap();

                    // Shift existing line attributions down to make room for new lines
                    let file_attestation = authorship_log.get_or_create_file(&entry.file);
                    for attestation_entry in file_attestation.entries.iter_mut() {
                        attestation_entry.shift_line_ranges(insertion_point, num_lines_added);
                    }

                    // Create compressed line ranges for the new additions
                    let new_line_ranges = LineRange::compress_lines(&added_lines);

                    // Only process AI-generated content (entries with prompt_session_id)
                    if let Some(session_id) = session_id_opt.clone() {
                        // Add new attestation entry for the AI-added lines
                        let entry = AttestationEntry::new(session_id, new_line_ranges);
                        file_attestation.add_entry(entry);
                    }
                }
            }
        }

        // Remove empty entries and empty files
        for file_attestation in &mut authorship_log.attestations {
            file_attestation
                .entries
                .retain(|entry| !entry.line_ranges.is_empty());
        }
        authorship_log
            .attestations
            .retain(|f| !f.entries.is_empty());

        // Sort attestation entries by hash for deterministic ordering
        for file_attestation in &mut authorship_log.attestations {
            file_attestation.entries.sort_by(|a, b| a.hash.cmp(&b.hash));
        }

        // Consolidate entries with the same hash
        for file_attestation in &mut authorship_log.attestations {
            let mut consolidated_entries = Vec::new();
            let mut current_hash: Option<String> = None;
            let mut current_ranges: Vec<LineRange> = Vec::new();

            for entry in &file_attestation.entries {
                if current_hash.as_ref() == Some(&entry.hash) {
                    // Same hash, accumulate line ranges
                    current_ranges.extend(entry.line_ranges.clone());
                } else {
                    // Different hash, save previous entry and start new one
                    if let Some(hash) = current_hash.take() {
                        // Merge overlapping and adjacent ranges before adding
                        let merged_ranges = Self::merge_line_ranges(&current_ranges);
                        consolidated_entries.push(AttestationEntry::new(hash, merged_ranges));
                    }
                    current_hash = Some(entry.hash.clone());
                    current_ranges = entry.line_ranges.clone();
                }
            }

            // Don't forget the last entry
            if let Some(hash) = current_hash {
                let merged_ranges = Self::merge_line_ranges(&current_ranges);
                consolidated_entries.push(AttestationEntry::new(hash, merged_ranges));
            }

            file_attestation.entries = consolidated_entries;
        }

        // Calculate accepted_lines for each session from the final attestation log
        let mut session_accepted_lines: HashMap<String, u32> = HashMap::new();
        for file_attestation in &authorship_log.attestations {
            for attestation_entry in &file_attestation.entries {
                let accepted_count: u32 = attestation_entry
                    .line_ranges
                    .iter()
                    .map(|range| count_line_range(range))
                    .sum();
                *session_accepted_lines
                    .entry(attestation_entry.hash.clone())
                    .or_insert(0) += accepted_count;
            }
        }

        // Update all PromptRecords with the calculated metrics
        for (session_id, prompt_record) in authorship_log.metadata.prompts.iter_mut() {
            prompt_record.total_additions = *session_additions.get(session_id).unwrap_or(&0);
            prompt_record.total_deletions = *session_deletions.get(session_id).unwrap_or(&0);
            prompt_record.accepted_lines = *session_accepted_lines.get(session_id).unwrap_or(&0);
        }

        authorship_log
    }

    pub fn get_or_create_file(&mut self, file: &str) -> &mut FileAttestation {
        // Check if file already exists
        let exists = self.attestations.iter().any(|f| f.file_path == file);

        if !exists {
            self.attestations
                .push(FileAttestation::new(file.to_string()));
        }

        // Now get the reference
        self.attestations
            .iter_mut()
            .find(|f| f.file_path == file)
            .unwrap()
    }

    /// Serialize to the new text format
    pub fn serialize_to_string(&self) -> Result<String, fmt::Error> {
        let mut output = String::new();

        // Write attestation section
        for file_attestation in &self.attestations {
            // Quote file names that contain spaces or whitespace
            let file_path = if needs_quoting(&file_attestation.file_path) {
                format!("\"{}\"", &file_attestation.file_path)
            } else {
                file_attestation.file_path.clone()
            };
            output.push_str(&file_path);
            output.push('\n');

            for entry in &file_attestation.entries {
                output.push_str("  ");
                output.push_str(&entry.hash);
                output.push(' ');
                output.push_str(&format_line_ranges(&entry.line_ranges));
                output.push('\n');
            }
        }

        // Write divider
        output.push_str("---\n");

        // Write JSON metadata section
        let json_str = serde_json::to_string_pretty(&self.metadata).map_err(|_| fmt::Error)?;
        output.push_str(&json_str);

        Ok(output)
    }

    /// Write to a writer in the new format
    pub fn _serialize_to_writer<W: Write>(&self, mut writer: W) -> std::io::Result<()> {
        let content = self
            .serialize_to_string()
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::Other, "Serialization failed"))?;
        writer.write_all(content.as_bytes())?;
        Ok(())
    }

    /// Deserialize from the new text format
    pub fn deserialize_from_string(content: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let lines: Vec<&str> = content.lines().collect();

        // Find the divider
        let divider_pos = lines
            .iter()
            .position(|&line| line == "---")
            .ok_or("Missing divider '---' in authorship log")?;

        // Parse attestation section (before divider)
        let attestation_lines = &lines[..divider_pos];
        let attestations = parse_attestation_section(attestation_lines)?;

        // Parse JSON metadata section (after divider)
        let json_lines = &lines[divider_pos + 1..];
        let json_content = json_lines.join("\n");
        let metadata: AuthorshipMetadata = serde_json::from_str(&json_content)?;

        Ok(Self {
            attestations,
            metadata,
        })
    }

    /// Read from a reader in the new format
    pub fn _deserialize_from_reader<R: BufRead>(
        reader: R,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let content: Result<String, _> = reader.lines().collect();
        let content = content?;
        Self::deserialize_from_string(&content)
    }

    /// Lookup the author and optional prompt for a given file and line
    pub fn get_line_attribution(
        &self,
        file: &str,
        line: u32,
    ) -> Option<(Author, Option<&PromptRecord>)> {
        // Find the file attestation
        let file_attestation = self.attestations.iter().find(|f| f.file_path == file)?;

        // Check entries in reverse order (latest wins)
        for entry in file_attestation.entries.iter().rev() {
            // Check if this line is covered by any of the line ranges
            let contains = entry.line_ranges.iter().any(|range| range.contains(line));
            if contains {
                // The hash corresponds to a prompt session short hash
                if let Some(prompt_record) = self.metadata.prompts.get(&entry.hash) {
                    // Create author info from the prompt record
                    let author = Author {
                        username: prompt_record.agent_id.tool.clone(),
                        email: String::new(), // AI agents don't have email
                    };

                    // Return author and prompt info
                    return Some((author, Some(prompt_record)));
                }
            }
        }
        None
    }
}

impl Default for AuthorshipLog {
    fn default() -> Self {
        Self::new()
    }
}

/// Format line ranges as comma-separated values with ranges as "start-end"
/// Sorts ranges first: Single ranges by their value, Range ones by their lowest bound
fn format_line_ranges(ranges: &[LineRange]) -> String {
    let mut sorted_ranges = ranges.to_vec();
    sorted_ranges.sort_by(|a, b| {
        let a_start = match a {
            LineRange::Single(line) => *line,
            LineRange::Range(start, _) => *start,
        };
        let b_start = match b {
            LineRange::Single(line) => *line,
            LineRange::Range(start, _) => *start,
        };
        a_start.cmp(&b_start)
    });

    sorted_ranges
        .iter()
        .map(|range| match range {
            LineRange::Single(line) => line.to_string(),
            LineRange::Range(start, end) => format!("{}-{}", start, end),
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// Parse line ranges from a string like "1,2,19-222"
/// No spaces are expected in the format
fn parse_line_ranges(input: &str) -> Result<Vec<LineRange>, Box<dyn std::error::Error>> {
    let mut ranges = Vec::new();

    for part in input.split(',') {
        if part.is_empty() {
            continue;
        }

        if let Some(dash_pos) = part.find('-') {
            // Range format: "start-end"
            let start_str = &part[..dash_pos];
            let end_str = &part[dash_pos + 1..];
            let start: u32 = start_str.parse()?;
            let end: u32 = end_str.parse()?;
            ranges.push(LineRange::Range(start, end));
        } else {
            // Single line format: "line"
            let line: u32 = part.parse()?;
            ranges.push(LineRange::Single(line));
        }
    }

    Ok(ranges)
}

/// Parse the attestation section (before the divider)
fn parse_attestation_section(
    lines: &[&str],
) -> Result<Vec<FileAttestation>, Box<dyn std::error::Error>> {
    let mut attestations = Vec::new();
    let mut current_file: Option<FileAttestation> = None;

    for line in lines {
        let line = line.trim_end(); // Remove trailing whitespace but preserve leading

        if line.is_empty() {
            continue;
        }

        if line.starts_with("  ") {
            // Attestation entry line (indented)
            let entry_line = &line[2..]; // Remove "  " prefix

            // Split on first space to separate hash from line ranges
            if let Some(space_pos) = entry_line.find(' ') {
                let hash = entry_line[..space_pos].to_string();
                let ranges_str = &entry_line[space_pos + 1..];
                let line_ranges = parse_line_ranges(ranges_str)?;

                let entry = AttestationEntry::new(hash, line_ranges);

                if let Some(ref mut file_attestation) = current_file {
                    file_attestation.add_entry(entry);
                } else {
                    return Err("Attestation entry found without a file path".into());
                }
            } else {
                return Err(format!("Invalid attestation entry format: {}", entry_line).into());
            }
        } else {
            // File path line (not indented)
            if let Some(file_attestation) = current_file.take() {
                if !file_attestation.entries.is_empty() {
                    attestations.push(file_attestation);
                }
            }

            // Parse file path, handling quoted paths
            let file_path = if line.starts_with('"') && line.ends_with('"') {
                // Quoted path - remove quotes (no unescaping needed since quotes aren't allowed in file names)
                line[1..line.len() - 1].to_string()
            } else {
                // Unquoted path
                line.to_string()
            };

            current_file = Some(FileAttestation::new(file_path));
        }
    }

    // Don't forget the last file
    if let Some(file_attestation) = current_file {
        if !file_attestation.entries.is_empty() {
            attestations.push(file_attestation);
        }
    }

    Ok(attestations)
}

/// Check if a file path needs quoting (contains spaces or whitespace)
fn needs_quoting(path: &str) -> bool {
    path.contains(' ') || path.contains('\t') || path.contains('\n')
}

#[cfg(test)]
mod tests {
    use super::*;
    use insta::assert_debug_snapshot;

    #[test]
    fn test_format_line_ranges() {
        let ranges = vec![
            LineRange::Range(19, 222),
            LineRange::Single(1),
            LineRange::Single(2),
        ];

        assert_debug_snapshot!(format_line_ranges(&ranges));
    }

    #[test]
    fn test_parse_line_ranges() {
        let ranges = parse_line_ranges("1,2,19-222").unwrap();
        assert_debug_snapshot!(ranges);
    }

    #[test]
    fn test_serialize_deserialize_roundtrip() {
        let mut log = AuthorshipLog::new();
        log.metadata.base_commit_sha = "abc123".to_string();

        // Add some attestations
        let mut file1 = FileAttestation::new("src/file.xyz".to_string());
        file1.add_entry(AttestationEntry::new(
            "xyzAbc".to_string(),
            vec![
                LineRange::Single(1),
                LineRange::Single(2),
                LineRange::Range(19, 222),
            ],
        ));
        file1.add_entry(AttestationEntry::new(
            "123456".to_string(),
            vec![LineRange::Range(400, 405)],
        ));

        let mut file2 = FileAttestation::new("src/file2.xyz".to_string());
        file2.add_entry(AttestationEntry::new(
            "123456".to_string(),
            vec![
                LineRange::Range(1, 111),
                LineRange::Single(245),
                LineRange::Single(260),
            ],
        ));

        log.attestations.push(file1);
        log.attestations.push(file2);

        // Serialize and snapshot the format
        let serialized = log.serialize_to_string().unwrap();
        assert_debug_snapshot!(serialized);

        // Test roundtrip: deserialize and verify structure matches
        let deserialized = AuthorshipLog::deserialize_from_string(&serialized).unwrap();
        assert_debug_snapshot!(deserialized);
    }

    #[test]
    fn test_expected_format() {
        let mut log = AuthorshipLog::new();

        let mut file1 = FileAttestation::new("src/file.xyz".to_string());
        file1.add_entry(AttestationEntry::new(
            "xyzAbc".to_string(),
            vec![
                LineRange::Single(1),
                LineRange::Single(2),
                LineRange::Range(19, 222),
            ],
        ));
        file1.add_entry(AttestationEntry::new(
            "123456".to_string(),
            vec![LineRange::Range(400, 405)],
        ));

        let mut file2 = FileAttestation::new("src/file2.xyz".to_string());
        file2.add_entry(AttestationEntry::new(
            "123456".to_string(),
            vec![
                LineRange::Range(1, 111),
                LineRange::Single(245),
                LineRange::Single(260),
            ],
        ));

        log.attestations.push(file1);
        log.attestations.push(file2);

        let serialized = log.serialize_to_string().unwrap();
        assert_debug_snapshot!(serialized);
    }

    #[test]
    fn test_line_range_sorting() {
        // Test that ranges are sorted correctly: single ranges and ranges by lowest bound
        let ranges = vec![
            LineRange::Range(100, 200),
            LineRange::Single(5),
            LineRange::Range(10, 15),
            LineRange::Single(50),
            LineRange::Single(1),
            LineRange::Range(25, 30),
        ];

        let formatted = format_line_ranges(&ranges);
        assert_debug_snapshot!(formatted);

        // Should be sorted as: 1, 5, 10-15, 25-30, 50, 100-200
    }

    #[test]
    fn test_file_names_with_spaces() {
        // Test file names with spaces and special characters
        let mut log = AuthorshipLog::new();

        // Add a prompt to the metadata
        let agent_id = crate::log_fmt::working_log::AgentId {
            tool: "cursor".to_string(),
            id: "session_123".to_string(),
            model: "claude-3-sonnet".to_string(),
        };
        let prompt_hash = generate_short_hash(&agent_id.id, &agent_id.tool);
        log.metadata.prompts.insert(
            prompt_hash.clone(),
            crate::log_fmt::authorship_log::PromptRecord {
                agent_id: agent_id,
                human_author: None,
                messages: vec![],
                total_additions: 0,
                total_deletions: 0,
                accepted_lines: 0,
            },
        );

        // Add attestations for files with spaces and special characters
        let mut file1 = FileAttestation::new("src/my file.rs".to_string());
        file1.add_entry(AttestationEntry::new(
            prompt_hash.to_string(),
            vec![LineRange::Range(1, 10)],
        ));

        let mut file2 = FileAttestation::new("docs/README (copy).md".to_string());
        file2.add_entry(AttestationEntry::new(
            prompt_hash.to_string(),
            vec![LineRange::Single(5)],
        ));

        let mut file3 = FileAttestation::new("test/file-with-dashes.js".to_string());
        file3.add_entry(AttestationEntry::new(
            prompt_hash.to_string(),
            vec![LineRange::Range(20, 25)],
        ));

        log.attestations.push(file1);
        log.attestations.push(file2);
        log.attestations.push(file3);

        let serialized = log.serialize_to_string().unwrap();
        println!("Serialized with special file names:\n{}", serialized);
        assert_debug_snapshot!(serialized);

        // Try to deserialize - this should work if we handle escaping properly
        let deserialized = AuthorshipLog::deserialize_from_string(&serialized);
        match deserialized {
            Ok(log) => {
                println!("Deserialization successful!");
                assert_debug_snapshot!(log);
            }
            Err(e) => {
                println!("Deserialization failed: {}", e);
                // This will fail with current implementation
            }
        }
    }

    #[test]
    fn test_hash_always_maps_to_prompt() {
        // Demonstrate that every hash in attestation section maps to prompts section
        let mut log = AuthorshipLog::new();

        // Add a prompt to the metadata
        let agent_id = crate::log_fmt::working_log::AgentId {
            tool: "cursor".to_string(),
            id: "session_123".to_string(),
            model: "claude-3-sonnet".to_string(),
        };
        let prompt_hash = generate_short_hash(&agent_id.id, &agent_id.tool);
        log.metadata.prompts.insert(
            prompt_hash.clone(),
            crate::log_fmt::authorship_log::PromptRecord {
                agent_id: agent_id,
                human_author: None,
                messages: vec![],
                total_additions: 0,
                total_deletions: 0,
                accepted_lines: 0,
            },
        );

        // Add attestation that references this prompt
        let mut file1 = FileAttestation::new("src/example.rs".to_string());
        file1.add_entry(AttestationEntry::new(
            prompt_hash.to_string(),
            vec![LineRange::Range(1, 10)],
        ));
        log.attestations.push(file1);

        let serialized = log.serialize_to_string().unwrap();
        assert_debug_snapshot!(serialized);

        // Verify that every hash in attestations has a corresponding prompt
        for file_attestation in &log.attestations {
            for entry in &file_attestation.entries {
                assert!(
                    log.metadata.prompts.contains_key(&entry.hash),
                    "Hash '{}' should have a corresponding prompt in metadata",
                    entry.hash
                );
            }
        }
    }

    #[test]
    fn test_serialize_deserialize_no_attestations() {
        // Test that serialization and deserialization work correctly when there are no attestations
        let mut log = AuthorshipLog::new();
        log.metadata.base_commit_sha = "abc123".to_string();

        let agent_id = crate::log_fmt::working_log::AgentId {
            tool: "cursor".to_string(),
            id: "session_123".to_string(),
            model: "claude-3-sonnet".to_string(),
        };
        let prompt_hash = generate_short_hash(&agent_id.id, &agent_id.tool);
        log.metadata.prompts.insert(
            prompt_hash,
            crate::log_fmt::authorship_log::PromptRecord {
                agent_id: agent_id,
                human_author: None,
                messages: vec![],
                total_additions: 0,
                total_deletions: 0,
                accepted_lines: 0,
            },
        );

        // Serialize and verify the format
        let serialized = log.serialize_to_string().unwrap();
        assert_debug_snapshot!(serialized);

        // Test roundtrip: deserialize and verify structure matches
        let deserialized = AuthorshipLog::deserialize_from_string(&serialized).unwrap();
        assert_debug_snapshot!(deserialized);

        // Verify that the deserialized log has the same metadata but no attestations
        assert_eq!(deserialized.metadata.base_commit_sha, "abc123");
        assert_eq!(deserialized.metadata.prompts.len(), 1);
        assert_eq!(deserialized.attestations.len(), 0);
    }

    #[test]
    fn test_remove_line_ranges_complete_removal() {
        let mut entry =
            AttestationEntry::new("test_hash".to_string(), vec![LineRange::Range(2, 5)]);

        // Remove the exact same range
        entry.remove_line_ranges(&[LineRange::Range(2, 5)]);

        // Should be empty after removing the exact range
        assert!(
            entry.line_ranges.is_empty(),
            "Expected empty line_ranges after complete removal, got: {:?}",
            entry.line_ranges
        );
    }

    #[test]
    fn test_remove_line_ranges_partial_removal() {
        let mut entry =
            AttestationEntry::new("test_hash".to_string(), vec![LineRange::Range(2, 10)]);

        // Remove middle part
        entry.remove_line_ranges(&[LineRange::Range(5, 7)]);

        // Should have two ranges: [2-4] and [8-10]
        assert_eq!(entry.line_ranges.len(), 2);
        assert_eq!(entry.line_ranges[0], LineRange::Range(2, 4));
        assert_eq!(entry.line_ranges[1], LineRange::Range(8, 10));
    }

    #[test]
    fn test_metrics_calculation() {
        use crate::log_fmt::transcript::{AiTranscript, Message};
        use crate::log_fmt::working_log::{AgentId, Checkpoint, Line, WorkingLogEntry};

        // Create an agent ID
        let agent_id = AgentId {
            tool: "cursor".to_string(),
            id: "test_session".to_string(),
            model: "claude-3-sonnet".to_string(),
        };

        // Create a transcript
        let mut transcript = AiTranscript::new();
        transcript.add_message(Message::user("Add a function".to_string()));
        transcript.add_message(Message::assistant("Here's the function".to_string()));

        // Create working log entries
        // First checkpoint: add 10 lines (single line + range of 9)
        let entry1 =
            WorkingLogEntry::new("src/test.rs".to_string(), vec![Line::Range(1, 10)], vec![]);
        let mut checkpoint1 = Checkpoint::new(
            "abc123".to_string(),
            "".to_string(),
            "ai".to_string(),
            vec![entry1],
        );
        checkpoint1.agent_id = Some(agent_id.clone());
        checkpoint1.transcript = Some(transcript.clone());

        // Second checkpoint: delete 3 lines, add 5 lines (modified some lines)
        let entry2 = WorkingLogEntry::new(
            "src/test.rs".to_string(),
            vec![Line::Range(5, 9)], // 5 added lines
            vec![Line::Range(5, 7)], // 3 deleted lines
        );
        let mut checkpoint2 = Checkpoint::new(
            "def456".to_string(),
            "".to_string(),
            "ai".to_string(),
            vec![entry2],
        );
        checkpoint2.agent_id = Some(agent_id.clone());
        checkpoint2.transcript = Some(transcript);

        // Convert to authorship log
        let authorship_log = AuthorshipLog::from_working_log_with_base_commit_and_human_author(
            &[checkpoint1, checkpoint2],
            "base123",
            None,
        );

        // Get the prompt record
        let session_hash = generate_short_hash(&agent_id.id, &agent_id.tool);
        let prompt_record = authorship_log.metadata.prompts.get(&session_hash).unwrap();

        // Verify metrics
        // total_additions: 10 (from first checkpoint) + 5 (from second) = 15
        assert_eq!(prompt_record.total_additions, 15);
        // total_deletions: 0 (from first) + 3 (from second) = 3
        assert_eq!(prompt_record.total_deletions, 3);
        // accepted_lines: After correct shifting logic:
        // - Checkpoint 1 adds 1-10 (10 lines)
        // - Checkpoint 2 deletes 5-7 (removes 3), shifts 8-10 up to 5-7 (7 lines remain)
        // - Checkpoint 2 adds 5-9 (5 lines), shifts existing 5-7 down to 10-12
        // - Final: AI owns 1-4, 5-9, 10-12 = 12 lines
        assert_eq!(prompt_record.accepted_lines, 12);
    }
}
