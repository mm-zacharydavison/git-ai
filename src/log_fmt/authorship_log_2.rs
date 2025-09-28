use crate::log_fmt::authorship_log::{Author, AuthorshipLog, LineRange, PromptRecord};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::io::{BufRead, Write};

/// New format version identifier
pub const AUTHORSHIP_LOG_V3_VERSION: &str = "authorship/3.0.0";

/// Metadata section that goes below the divider as JSON
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuthorshipMetadata {
    pub schema_version: String,
    pub base_commit_sha: String,
    pub authors: BTreeMap<String, Author>,
    pub prompts: BTreeMap<String, PromptRecord>,
}

impl AuthorshipMetadata {
    pub fn new() -> Self {
        Self {
            schema_version: AUTHORSHIP_LOG_V3_VERSION.to_string(),
            base_commit_sha: String::new(),
            authors: BTreeMap::new(),
            prompts: BTreeMap::new(),
        }
    }
}

impl Default for AuthorshipMetadata {
    fn default() -> Self {
        Self::new()
    }
}

/// Attestation entry: hash followed by line ranges
///
/// IMPORTANT: The hash ALWAYS corresponds to a prompt in the prompts section.
/// This system only tracks AI-generated content, not human-authored content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttestationEntry {
    /// Short Hash that maps to an entry in the prompts section of the metadata
    pub hash: String,
    /// Line ranges that this prompt is responsible for
    pub line_ranges: Vec<LineRange>,
}

impl AttestationEntry {
    pub fn new(hash: String, line_ranges: Vec<LineRange>) -> Self {
        Self { hash, line_ranges }
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

/// The complete authorship log in the new format
#[derive(Debug, Clone, PartialEq)]
pub struct AuthorshipLogV3 {
    pub attestations: Vec<FileAttestation>,
    pub metadata: AuthorshipMetadata,
}

impl AuthorshipLogV3 {
    pub fn new() -> Self {
        Self {
            attestations: Vec::new(),
            metadata: AuthorshipMetadata::new(),
        }
    }

    /// Convert from the old AuthorshipLog format to the new format
    pub fn from_authorship_log(log: &AuthorshipLog) -> Self {
        let mut v3_log = Self::new();

        // Copy metadata
        v3_log.metadata.base_commit_sha = log.base_commit_sha.clone();
        v3_log.metadata.authors = log.authors.clone();
        v3_log.metadata.prompts = log.prompts.clone();

        // Convert file attributions to attestations
        // Only process AI-generated content (entries with prompt_session_id)
        for (file_path, attributions) in &log.files {
            let mut file_attestation = FileAttestation::new(file_path.clone());

            for attribution in attributions {
                // Only process AI-generated content - skip human content
                if let Some(session_id) = &attribution.prompt_session_id {
                    // Hash always corresponds to a prompt in the prompts section
                    // Use session ID as the hash - this maps to an entry in prompts
                    let hash = session_id.clone();
                    let entry = AttestationEntry::new(hash, attribution.lines.clone());
                    file_attestation.add_entry(entry);
                }
                // Skip entries without prompt_session_id (human content)
            }

            if !file_attestation.entries.is_empty() {
                v3_log.attestations.push(file_attestation);
            }
        }

        v3_log
    }

    /// Serialize to the new text format
    pub fn serialize_to_string(&self) -> Result<String, fmt::Error> {
        let mut output = String::new();

        // Write attestation section
        for file_attestation in &self.attestations {
            output.push_str(&file_attestation.file_path);
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
    pub fn serialize_to_writer<W: Write>(&self, mut writer: W) -> std::io::Result<()> {
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
    pub fn deserialize_from_reader<R: BufRead>(
        reader: R,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let content: Result<String, _> = reader.lines().collect();
        let content = content?;
        Self::deserialize_from_string(&content)
    }
}

impl Default for AuthorshipLogV3 {
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
            current_file = Some(FileAttestation::new(line.to_string()));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log_fmt::authorship_log::Author;
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
        let mut log = AuthorshipLogV3::new();
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

        // Add some metadata
        log.metadata.authors.insert(
            "xyzAbc".to_string(),
            Author {
                username: "alice".to_string(),
                email: "alice@example.com".to_string(),
            },
        );

        // Serialize and snapshot the format
        let serialized = log.serialize_to_string().unwrap();
        println!("serialized: {}", serialized);
        assert_debug_snapshot!(serialized);

        // Test roundtrip: deserialize and verify structure matches
        let deserialized = AuthorshipLogV3::deserialize_from_string(&serialized).unwrap();
        assert_debug_snapshot!(deserialized);
    }

    #[test]
    fn test_expected_format() {
        let mut log = AuthorshipLogV3::new();

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
    fn test_conversion_from_old_format() {
        // Test converting from the old AuthorshipLog format
        let mut old_log = crate::log_fmt::authorship_log::AuthorshipLog::new();
        old_log.base_commit_sha = "test-commit-sha".to_string();

        // Add some test data to the old format
        old_log.authors.insert(
            "author1".to_string(),
            Author {
                username: "test_user".to_string(),
                email: "test@example.com".to_string(),
            },
        );

        // Convert to new format
        let new_log = AuthorshipLogV3::from_authorship_log(&old_log);
        assert_debug_snapshot!(new_log);
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
    fn test_hash_always_maps_to_prompt() {
        // Demonstrate that every hash in attestation section maps to prompts section
        let mut log = AuthorshipLogV3::new();

        // Add a prompt to the metadata
        let prompt_hash = "prompt_abc123";
        log.metadata.prompts.insert(
            prompt_hash.to_string(),
            crate::log_fmt::authorship_log::PromptRecord {
                agent_id: crate::log_fmt::working_log::AgentId {
                    tool: "cursor".to_string(),
                    id: "session_123".to_string(),
                },
                messages: vec![],
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
}
