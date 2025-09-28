use crate::log_fmt::transcript::Message;
use crate::log_fmt::working_log::{AgentId, Checkpoint, Line};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;

// 2.0.0 being used since some users have wip 1.0.0 in git history already
pub const AUTHORSHIP_LOG_VERSION: &str = "authorship/2.0.0";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Author {
    pub username: String,
    pub email: String,
}

/// Represents either a single line or a range of lines
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
pub enum LineRange {
    Single(u32),
    Range(u32, u32), // start, end (inclusive)
}

impl LineRange {
    pub fn contains(&self, line: u32) -> bool {
        match self {
            LineRange::Single(l) => *l == line,
            LineRange::Range(start, end) => line >= *start && line <= *end,
        }
    }

    #[allow(dead_code)]
    pub fn overlaps(&self, other: &LineRange) -> bool {
        match (self, other) {
            (LineRange::Single(l1), LineRange::Single(l2)) => l1 == l2,
            (LineRange::Single(l), LineRange::Range(start, end)) => *l >= *start && *l <= *end,
            (LineRange::Range(start, end), LineRange::Single(l)) => *l >= *start && *l <= *end,
            (LineRange::Range(start1, end1), LineRange::Range(start2, end2)) => {
                start1 <= end2 && start2 <= end1
            }
        }
    }

    /// Remove a line or range from this range, returning the remaining parts
    pub fn remove(&self, to_remove: &LineRange) -> Vec<LineRange> {
        match (self, to_remove) {
            (LineRange::Single(l), LineRange::Single(r)) => {
                if l == r {
                    vec![]
                } else {
                    vec![self.clone()]
                }
            }
            (LineRange::Single(l), LineRange::Range(start, end)) => {
                if *l >= *start && *l <= *end {
                    vec![]
                } else {
                    vec![self.clone()]
                }
            }
            (LineRange::Range(start, end), LineRange::Single(r)) => {
                if *r < *start || *r > *end {
                    vec![self.clone()]
                } else if *r == *start && *r == *end {
                    vec![]
                } else if *r == *start {
                    vec![LineRange::Range(*start + 1, *end)]
                } else if *r == *end {
                    vec![LineRange::Range(*start, *end - 1)]
                } else {
                    vec![
                        LineRange::Range(*start, *r - 1),
                        LineRange::Range(*r + 1, *end),
                    ]
                }
            }
            (LineRange::Range(start1, end1), LineRange::Range(start2, end2)) => {
                if *start2 > *end1 || *end2 < *start1 {
                    // No overlap
                    vec![self.clone()]
                } else {
                    let mut result = Vec::new();
                    // Left part
                    if *start1 < *start2 {
                        result.push(LineRange::Range(*start1, *start2 - 1));
                    }
                    // Right part
                    if *end1 > *end2 {
                        result.push(LineRange::Range(*end2 + 1, *end1));
                    }
                    result
                }
            }
        }
    }

    /// Convert a sorted list of line numbers into compressed ranges
    pub fn compress_lines(lines: &[u32]) -> Vec<LineRange> {
        if lines.is_empty() {
            return vec![];
        }

        let mut ranges = Vec::new();
        let mut current_start = lines[0];
        let mut current_end = lines[0];

        for &line in &lines[1..] {
            if line == current_end + 1 {
                current_end = line;
            } else {
                // End current range and start new one
                if current_start == current_end {
                    ranges.push(LineRange::Single(current_start));
                } else {
                    ranges.push(LineRange::Range(current_start, current_end));
                }
                current_start = line;
                current_end = line;
            }
        }

        // Add the last range
        if current_start == current_end {
            ranges.push(LineRange::Single(current_start));
        } else {
            ranges.push(LineRange::Range(current_start, current_end));
        }

        ranges
    }

    #[allow(dead_code)]
    pub fn expand(&self) -> Vec<u32> {
        match self {
            LineRange::Single(l) => vec![*l],
            LineRange::Range(start, end) => (*start..=*end).collect(),
        }
    }
}

impl fmt::Display for LineRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LineRange::Single(l) => write!(f, "{}", l),
            LineRange::Range(start, end) => write!(f, "[{}, {}]", start, end),
        }
    }
}

/// Per-file attribution entry - used internally for processing, not for serialization
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AttributionEntry {
    pub lines: Vec<LineRange>,
    pub author_key: String,
    pub prompt_session_id: Option<String>,
}

impl AttributionEntry {
    pub fn add_lines(&mut self, lines: &[LineRange]) {
        self.lines.extend(lines.iter().cloned());
        self.lines.sort();
        self.deduplicate_and_merge_ranges();
    }

    pub fn remove_lines(&mut self, to_remove: &[LineRange]) {
        let mut new_lines = Vec::new();
        for existing_range in &self.lines {
            let mut remaining = Vec::new();
            for remove_range in to_remove {
                remaining.extend(existing_range.remove(remove_range));
            }
            new_lines.extend(remaining);
        }
        self.lines = new_lines;
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    #[allow(dead_code)]
    fn contains_line(&self, line: u32) -> bool {
        self.lines.iter().any(|r| r.contains(line))
    }

    fn deduplicate_and_merge_ranges(&mut self) {
        if self.lines.is_empty() {
            return;
        }

        // First, sort by start position
        self.lines.sort_by(|a, b| {
            let start_a = match a {
                LineRange::Single(l) => *l,
                LineRange::Range(start, _) => *start,
            };
            let start_b = match b {
                LineRange::Single(l) => *l,
                LineRange::Range(start, _) => *start,
            };
            start_a.cmp(&start_b)
        });

        // Remove exact duplicates
        self.lines.dedup();

        // Merge overlapping/adjacent ranges
        let mut merged = Vec::new();
        for current in self.lines.drain(..) {
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

        self.lines = merged;
    }

    fn ranges_can_merge(range1: &LineRange, range2: &LineRange) -> bool {
        match (range1, range2) {
            (LineRange::Single(l1), LineRange::Single(l2)) => l1.abs_diff(*l2) <= 1,
            (LineRange::Single(l), LineRange::Range(start, end)) => {
                *l >= start.saturating_sub(1) && *l <= end + 1
            }
            (LineRange::Range(start, end), LineRange::Single(l)) => {
                *l >= start.saturating_sub(1) && *l <= end + 1
            }
            (LineRange::Range(start1, end1), LineRange::Range(start2, end2)) => {
                start1 <= &(end2 + 1) && start2 <= &(end1 + 1)
            }
        }
    }

    fn merge_ranges(range1: &LineRange, range2: &LineRange) -> LineRange {
        let (start1, end1) = match range1 {
            LineRange::Single(l) => (*l, *l),
            LineRange::Range(start, end) => (*start, *end),
        };
        let (start2, end2) = match range2 {
            LineRange::Single(l) => (*l, *l),
            LineRange::Range(start, end) => (*start, *end),
        };
        let merged_start = start1.min(start2);
        let merged_end = end1.max(end2);
        if merged_start == merged_end {
            LineRange::Single(merged_start)
        } else {
            LineRange::Range(merged_start, merged_end)
        }
    }
}

/// Prompt session details stored in the top-level prompts map keyed by AgentId.id (UUID)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromptRecord {
    pub agent_id: AgentId,
    pub model: Option<String>,
    pub human_author: Option<String>,
    pub messages: Vec<Message>,
}

/// Per-file attributions are arrays of compact entries
pub type FileAttributions = Vec<AttributionEntry>;

/// Legacy authorship log format - use AuthorshipLogV3 for new serialization
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AuthorshipLog {
    pub files: BTreeMap<String, FileAttributions>,
    pub authors: BTreeMap<String, Author>,
    /// Map of prompt session UUID -> AgentId
    pub prompts: BTreeMap<String, PromptRecord>,
    pub schema_version: String,
    pub base_commit_sha: String,
}

impl AuthorshipLog {
    pub fn new() -> Self {
        Self {
            files: BTreeMap::new(),
            authors: BTreeMap::new(),
            prompts: BTreeMap::new(),
            schema_version: AUTHORSHIP_LOG_VERSION.to_string(),
            base_commit_sha: String::new(),
        }
    }

    pub fn get_or_create_file(&mut self, file: &str) -> &mut FileAttributions {
        self.files.entry(file.to_string()).or_insert_with(Vec::new)
    }

    /// Lookup the author and optional prompt for a given file and line
    pub fn get_line_attribution(
        &self,
        file: &str,
        line: u32,
    ) -> Option<(&Author, Option<&PromptRecord>)> {
        let entries = self.files.get(file)?;
        // Prefer later entries (latest wins)
        for e in entries.iter().rev() {
            // Manual contains check to keep AttributionEntry internals private
            let contains = e.lines.iter().any(|r| r.contains(line));
            if contains {
                if let Some(author) = self.authors.get(&e.author_key) {
                    let prompt = e
                        .prompt_session_id
                        .as_ref()
                        .and_then(|session| self.prompts.get(session));
                    return Some((author, prompt));
                }
            }
        }
        None
    }

    // Prompt hashing removed in v2; authorship log references prompts by AgentId.id and turn index

    /// Generate a stable author key based on username and email
    pub fn generate_author_key(author: &Author) -> String {
        let mut hasher = Sha256::new();
        hasher.update(author.username.as_bytes());
        hasher.update(b"|");
        hasher.update(author.email.as_bytes());
        let full = format!("{:x}", hasher.finalize());
        full.chars().take(8).collect()
    }

    /// Convert from working log checkpoints to authorship log
    #[allow(dead_code)]
    pub fn from_working_log(checkpoints: &[Checkpoint]) -> Self {
        Self::from_working_log_with_base_commit(checkpoints, "")
    }

    /// Convert from working log checkpoints to authorship log with base commit SHA
    pub fn from_working_log_with_base_commit(
        checkpoints: &[Checkpoint],
        base_commit_sha: &str,
    ) -> Self {
        Self::from_working_log_with_base_commit_and_human_author(checkpoints, base_commit_sha, None)
    }

    /// Convert from working log checkpoints to authorship log with base commit SHA and human author
    pub fn from_working_log_with_base_commit_and_human_author(
        checkpoints: &[Checkpoint],
        base_commit_sha: &str,
        human_author: Option<&str>,
    ) -> Self {
        let mut authorship_log = AuthorshipLog::new();
        authorship_log.base_commit_sha = base_commit_sha.to_string();

        // Process checkpoints and create attributions
        for checkpoint in checkpoints.iter() {
            // If there is an agent session, record it by its UUID (AgentId.id)
            let session_id_opt =
                match (&checkpoint.agent_id, &checkpoint.transcript) {
                    (Some(agent), Some(transcript)) => {
                        let session_id = agent.id.clone();
                        // Insert or update the prompt session transcript
                        let entry = authorship_log.prompts.entry(session_id.clone()).or_insert(
                            PromptRecord {
                                agent_id: agent.clone(),
                                model: Some(agent.model.clone()),
                                human_author: human_author.map(|s| s.to_string()),
                                messages: transcript.messages().to_vec(),
                            },
                        );
                        if entry.messages.len() < transcript.messages().len() {
                            entry.messages = transcript.messages().to_vec();
                        }
                        Some(session_id)
                    }
                    _ => None,
                };
            for entry in &checkpoint.entries {
                // Process deletions first (remove lines from all authors)
                {
                    let file_entries = authorship_log.get_or_create_file(&entry.file);
                    for line in &entry.deleted_lines {
                        let to_remove = match line {
                            Line::Single(l) => LineRange::Single(*l),
                            Line::Range(start, end) => LineRange::Range(*start, *end),
                        };
                        for record in file_entries.iter_mut() {
                            record.remove_lines(&[to_remove.clone()]);
                        }
                    }
                }

                // Then process additions (new author takes ownership)
                let mut added_lines = Vec::new();
                for line in &entry.added_lines {
                    match line {
                        Line::Single(l) => added_lines.push(*l),
                        Line::Range(start, end) => {
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
                    // Determine author key and optional prompt reference
                    let author_struct = Author {
                        username: checkpoint.author.clone(),
                        email: "".to_string(),
                    };
                    let key = Self::generate_author_key(&author_struct);
                    if !authorship_log.authors.contains_key(&key) {
                        authorship_log.authors.insert(key.clone(), author_struct);
                    }
                    let author_key = key;
                    let prompt_session_id = session_id_opt.clone();

                    // Create a single range to remove from all other entries
                    let lines_to_remove = LineRange::compress_lines(&added_lines);

                    // Remove these lines from all other entries and add new attribution
                    let file_entries = authorship_log.get_or_create_file(&entry.file);
                    for rec in file_entries.iter_mut() {
                        rec.remove_lines(&lines_to_remove);
                    }
                    if let Some(rec) = file_entries.iter_mut().find(|r| {
                        r.author_key == author_key && r.prompt_session_id == prompt_session_id
                    }) {
                        rec.add_lines(&lines_to_remove);
                    } else {
                        let mut new_rec = AttributionEntry {
                            lines: Vec::new(),
                            author_key: author_key.clone(),
                            prompt_session_id: prompt_session_id.clone(),
                        };
                        new_rec.add_lines(&lines_to_remove);
                        file_entries.push(new_rec);
                    }
                }
            }
        }
        // Remove empty files/entries
        authorship_log
            .files
            .retain(|_, entries| entries.iter().any(|e| !e.is_empty()));
        authorship_log
    }
}

impl Default for AuthorshipLog {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log_fmt::working_log::{Checkpoint, Line, WorkingLogEntry};

    fn resolve_username(log: &AuthorshipLog, file: &str, line: u32) -> Option<String> {
        log.get_line_attribution(file, line)
            .map(|(author, _)| author.username.clone())
    }

    #[test]
    fn test_file_authorship_add_lines() {
        // Simulate two checkpoints: claude adds 1-10, then aidan adds 5,6,50
        let entry1 =
            WorkingLogEntry::new("src/test.rs".to_string(), vec![Line::Range(1, 10)], vec![]);
        let checkpoint1 = Checkpoint::new(
            "abc123".to_string(),
            "".to_string(),
            "claude".to_string(),
            vec![entry1],
        );
        let entry2 = WorkingLogEntry::new(
            "src/test.rs".to_string(),
            vec![Line::Single(5), Line::Single(6), Line::Single(50)],
            vec![],
        );
        let checkpoint2 = Checkpoint::new(
            "def456".to_string(),
            "".to_string(),
            "aidan".to_string(),
            vec![entry2],
        );
        let log = AuthorshipLog::from_working_log(&[checkpoint1, checkpoint2]);
        assert_eq!(
            resolve_username(&log, "src/test.rs", 5),
            Some("aidan".to_string())
        );
        assert_eq!(
            resolve_username(&log, "src/test.rs", 6),
            Some("aidan".to_string())
        );
        assert_eq!(
            resolve_username(&log, "src/test.rs", 50),
            Some("aidan".to_string())
        );
        assert_eq!(
            resolve_username(&log, "src/test.rs", 4),
            Some("claude".to_string())
        );
        assert_eq!(
            resolve_username(&log, "src/test.rs", 7),
            Some("claude".to_string())
        );
        assert_eq!(resolve_username(&log, "src/test.rs", 100), None);
    }

    #[test]
    fn test_authorship_log_from_working_log() {
        let entry1 =
            WorkingLogEntry::new("src/test.rs".to_string(), vec![Line::Range(1, 10)], vec![]);
        let checkpoint1 = Checkpoint::new(
            "abc123".to_string(),
            "".to_string(),
            "claude".to_string(),
            vec![entry1],
        );
        let entry2 = WorkingLogEntry::new(
            "src/test.rs".to_string(),
            vec![Line::Single(5), Line::Single(6), Line::Single(50)],
            vec![],
        );
        let checkpoint2 = Checkpoint::new(
            "def456".to_string(),
            "".to_string(),
            "aidan".to_string(),
            vec![entry2],
        );
        let checkpoints = vec![checkpoint1, checkpoint2];
        let authorship_log = AuthorshipLog::from_working_log(&checkpoints);
        assert_eq!(
            resolve_username(&authorship_log, "src/test.rs", 5),
            Some("aidan".to_string())
        );
        assert_eq!(
            resolve_username(&authorship_log, "src/test.rs", 6),
            Some("aidan".to_string())
        );
        assert_eq!(
            resolve_username(&authorship_log, "src/test.rs", 50),
            Some("aidan".to_string())
        );
        assert_eq!(
            resolve_username(&authorship_log, "src/test.rs", 4),
            Some("claude".to_string())
        );
        assert_eq!(
            resolve_username(&authorship_log, "src/test.rs", 7),
            Some("claude".to_string())
        );
        assert_eq!(resolve_username(&authorship_log, "src/test.rs", 100), None);
    }

    #[test]
    fn test_deletion_removes_lines() {
        let entry1 =
            WorkingLogEntry::new("src/test.rs".to_string(), vec![Line::Range(1, 10)], vec![]);
        let checkpoint1 = Checkpoint::new(
            "abc123".to_string(),
            "".to_string(),
            "claude".to_string(),
            vec![entry1],
        );
        let entry2 = WorkingLogEntry::new(
            "src/test.rs".to_string(),
            vec![],
            vec![Line::Single(5), Line::Range(8, 10)],
        );
        let checkpoint2 = Checkpoint::new(
            "def456".to_string(),
            "".to_string(),
            "aidan".to_string(),
            vec![entry2],
        );
        let checkpoints = vec![checkpoint1, checkpoint2];
        let authorship_log = AuthorshipLog::from_working_log(&checkpoints);
        assert_eq!(resolve_username(&authorship_log, "src/test.rs", 5), None);
        assert_eq!(resolve_username(&authorship_log, "src/test.rs", 8), None);
        assert_eq!(resolve_username(&authorship_log, "src/test.rs", 9), None);
        assert_eq!(resolve_username(&authorship_log, "src/test.rs", 10), None);
        assert_eq!(
            resolve_username(&authorship_log, "src/test.rs", 4),
            Some("claude".to_string())
        );
        assert_eq!(
            resolve_username(&authorship_log, "src/test.rs", 6),
            Some("claude".to_string())
        );
    }

    #[test]
    fn test_middle_range_addition() {
        let entry1 =
            WorkingLogEntry::new("src/test.rs".to_string(), vec![Line::Range(1, 20)], vec![]);
        let checkpoint1 = Checkpoint::new(
            "abc123".to_string(),
            "".to_string(),
            "claude".to_string(),
            vec![entry1],
        );
        let entry2 =
            WorkingLogEntry::new("src/test.rs".to_string(), vec![Line::Range(8, 12)], vec![]);
        let checkpoint2 = Checkpoint::new(
            "def456".to_string(),
            "".to_string(),
            "aidan".to_string(),
            vec![entry2],
        );
        let checkpoints = vec![checkpoint1, checkpoint2];
        let authorship_log = AuthorshipLog::from_working_log(&checkpoints);
        // Lines 1-7: claude
        assert_eq!(
            resolve_username(&authorship_log, "src/test.rs", 1),
            Some("claude".to_string())
        );
        assert_eq!(
            resolve_username(&authorship_log, "src/test.rs", 7),
            Some("claude".to_string())
        );
        // Lines 8-12: aidan
        assert_eq!(
            resolve_username(&authorship_log, "src/test.rs", 8),
            Some("aidan".to_string())
        );
        assert_eq!(
            resolve_username(&authorship_log, "src/test.rs", 12),
            Some("aidan".to_string())
        );
        // Lines 13-20: claude
        assert_eq!(
            resolve_username(&authorship_log, "src/test.rs", 13),
            Some("claude".to_string())
        );
        assert_eq!(
            resolve_username(&authorship_log, "src/test.rs", 20),
            Some("claude".to_string())
        );
    }

    #[test]
    fn test_middle_range_deletion() {
        let entry1 =
            WorkingLogEntry::new("src/test.rs".to_string(), vec![Line::Range(1, 20)], vec![]);
        let checkpoint1 = Checkpoint::new(
            "abc123".to_string(),
            "".to_string(),
            "claude".to_string(),
            vec![entry1],
        );
        let entry2 =
            WorkingLogEntry::new("src/test.rs".to_string(), vec![], vec![Line::Range(8, 12)]);
        let checkpoint2 = Checkpoint::new(
            "def456".to_string(),
            "".to_string(),
            "aidan".to_string(),
            vec![entry2],
        );
        let checkpoints = vec![checkpoint1, checkpoint2];
        let authorship_log = AuthorshipLog::from_working_log(&checkpoints);
        // Lines 1-7: claude
        assert_eq!(
            resolve_username(&authorship_log, "src/test.rs", 1),
            Some("claude".to_string())
        );
        assert_eq!(
            resolve_username(&authorship_log, "src/test.rs", 7),
            Some("claude".to_string())
        );
        // Lines 8-12: deleted (None)
        assert_eq!(resolve_username(&authorship_log, "src/test.rs", 8), None);
        assert_eq!(resolve_username(&authorship_log, "src/test.rs", 12), None);
        // Lines 13-20: claude
        assert_eq!(
            resolve_username(&authorship_log, "src/test.rs", 13),
            Some("claude".to_string())
        );
        assert_eq!(
            resolve_username(&authorship_log, "src/test.rs", 20),
            Some("claude".to_string())
        );
    }

    #[test]
    fn test_multiple_overlapping_ranges() {
        let entry1 =
            WorkingLogEntry::new("src/test.rs".to_string(), vec![Line::Range(1, 30)], vec![]);
        let checkpoint1 = Checkpoint::new(
            "abc123".to_string(),
            "".to_string(),
            "claude".to_string(),
            vec![entry1],
        );
        let entry2 =
            WorkingLogEntry::new("src/test.rs".to_string(), vec![Line::Range(5, 15)], vec![]);
        let checkpoint2 = Checkpoint::new(
            "def456".to_string(),
            "".to_string(),
            "aidan".to_string(),
            vec![entry2],
        );
        let entry3 =
            WorkingLogEntry::new("src/test.rs".to_string(), vec![Line::Range(10, 20)], vec![]);
        let checkpoint3 = Checkpoint::new(
            "ghi789".to_string(),
            "".to_string(),
            "user".to_string(),
            vec![entry3],
        );
        let checkpoints = vec![checkpoint1, checkpoint2, checkpoint3];
        let authorship_log = AuthorshipLog::from_working_log(&checkpoints);
        // Lines 1-4: claude
        assert_eq!(
            resolve_username(&authorship_log, "src/test.rs", 1),
            Some("claude".to_string())
        );
        assert_eq!(
            resolve_username(&authorship_log, "src/test.rs", 4),
            Some("claude".to_string())
        );
        // Lines 5-9: aidan
        assert_eq!(
            resolve_username(&authorship_log, "src/test.rs", 5),
            Some("aidan".to_string())
        );
        assert_eq!(
            resolve_username(&authorship_log, "src/test.rs", 9),
            Some("aidan".to_string())
        );
        // Lines 10-20: user (overwrites both claude and aidan)
        assert_eq!(
            resolve_username(&authorship_log, "src/test.rs", 10),
            Some("user".to_string())
        );
        assert_eq!(
            resolve_username(&authorship_log, "src/test.rs", 20),
            Some("user".to_string())
        );
        // Lines 21-30: claude
        assert_eq!(
            resolve_username(&authorship_log, "src/test.rs", 21),
            Some("claude".to_string())
        );
        assert_eq!(
            resolve_username(&authorship_log, "src/test.rs", 30),
            Some("claude".to_string())
        );
    }

    #[test]
    fn test_single_line_edits_in_middle() {
        let entry1 =
            WorkingLogEntry::new("src/test.rs".to_string(), vec![Line::Range(1, 20)], vec![]);
        let checkpoint1 = Checkpoint::new(
            "abc123".to_string(),
            "".to_string(),
            "claude".to_string(),
            vec![entry1],
        );
        let entry2 = WorkingLogEntry::new(
            "src/test.rs".to_string(),
            vec![Line::Single(5), Line::Single(10), Line::Single(15)],
            vec![],
        );
        let checkpoint2 = Checkpoint::new(
            "def456".to_string(),
            "".to_string(),
            "aidan".to_string(),
            vec![entry2],
        );
        let checkpoints = vec![checkpoint1, checkpoint2];
        let authorship_log = AuthorshipLog::from_working_log(&checkpoints);
        // Specific lines taken by aidan
        assert_eq!(
            resolve_username(&authorship_log, "src/test.rs", 5),
            Some("aidan".to_string())
        );
        assert_eq!(
            resolve_username(&authorship_log, "src/test.rs", 10),
            Some("aidan".to_string())
        );
        assert_eq!(
            resolve_username(&authorship_log, "src/test.rs", 15),
            Some("aidan".to_string())
        );
        // Other lines still claude
        assert_eq!(
            resolve_username(&authorship_log, "src/test.rs", 4),
            Some("claude".to_string())
        );
        assert_eq!(
            resolve_username(&authorship_log, "src/test.rs", 6),
            Some("claude".to_string())
        );
        assert_eq!(
            resolve_username(&authorship_log, "src/test.rs", 9),
            Some("claude".to_string())
        );
        assert_eq!(
            resolve_username(&authorship_log, "src/test.rs", 11),
            Some("claude".to_string())
        );
        assert_eq!(
            resolve_username(&authorship_log, "src/test.rs", 14),
            Some("claude".to_string())
        );
        assert_eq!(
            resolve_username(&authorship_log, "src/test.rs", 16),
            Some("claude".to_string())
        );
    }

    #[test]
    fn test_edge_case_boundary_lines() {
        let entry1 =
            WorkingLogEntry::new("src/test.rs".to_string(), vec![Line::Range(1, 10)], vec![]);
        let checkpoint1 = Checkpoint::new(
            "abc123".to_string(),
            "".to_string(),
            "claude".to_string(),
            vec![entry1],
        );
        let entry2 = WorkingLogEntry::new(
            "src/test.rs".to_string(),
            vec![Line::Range(1, 1), Line::Range(10, 10)],
            vec![],
        );
        let checkpoint2 = Checkpoint::new(
            "def456".to_string(),
            "".to_string(),
            "aidan".to_string(),
            vec![entry2],
        );
        let checkpoints = vec![checkpoint1, checkpoint2];
        let authorship_log = AuthorshipLog::from_working_log(&checkpoints);
        // Boundary lines taken by aidan
        assert_eq!(
            resolve_username(&authorship_log, "src/test.rs", 1),
            Some("aidan".to_string())
        );
        assert_eq!(
            resolve_username(&authorship_log, "src/test.rs", 10),
            Some("aidan".to_string())
        );
        // Middle lines still claude
        assert_eq!(
            resolve_username(&authorship_log, "src/test.rs", 5),
            Some("claude".to_string())
        );
        assert_eq!(
            resolve_username(&authorship_log, "src/test.rs", 9),
            Some("claude".to_string())
        );
    }

    #[test]
    fn test_replacement_operation() {
        // Test the specific case from the user's example
        let entry = WorkingLogEntry::new(
            "src/commands/post_commit.rs".to_string(),
            vec![Line::Range(86, 89), Line::Range(98, 102)], // added lines
            vec![Line::Range(86, 108), Line::Single(117)],   // deleted lines
        );
        let checkpoint = Checkpoint::new(
            "test123".to_string(),
            "".to_string(),
            "aidan".to_string(),
            vec![entry],
        );
        let checkpoints = vec![checkpoint];
        let authorship_log = AuthorshipLog::from_working_log(&checkpoints);
        assert!(
            authorship_log
                .files
                .contains_key("src/commands/post_commit.rs")
        );
        // Lines 86-89 and 98-102 should be authored by "aidan"
        for n in [86, 87, 88, 89, 98, 99, 100, 101, 102] {
            assert_eq!(
                resolve_username(&authorship_log, "src/commands/post_commit.rs", n),
                Some("aidan".to_string())
            );
        }
        // Lines outside the added ranges should not have authorship
        for n in [85, 90, 97, 103, 117] {
            assert_eq!(
                resolve_username(&authorship_log, "src/commands/post_commit.rs", n),
                None
            );
        }
    }

    #[test]
    fn test_deduplicate_identical_ranges() {
        let mut entry = AttributionEntry {
            lines: Vec::new(),
            author_key: "a".to_string(),
            prompt_session_id: None,
        };
        entry.add_lines(&[LineRange::Range(1, 5)]);
        entry.add_lines(&[LineRange::Range(1, 5)]);
        entry.add_lines(&[LineRange::Range(1, 5)]);
        assert_eq!(entry.lines.len(), 1);
        assert_eq!(entry.lines[0], LineRange::Range(1, 5));
    }

    #[test]
    fn test_merge_adjacent_ranges() {
        let mut entry = AttributionEntry {
            lines: Vec::new(),
            author_key: "a".to_string(),
            prompt_session_id: None,
        };
        entry.add_lines(&[LineRange::Range(1, 5)]);
        entry.add_lines(&[LineRange::Range(6, 10)]);
        assert_eq!(entry.lines.len(), 1);
        assert_eq!(entry.lines[0], LineRange::Range(1, 10));
    }

    #[test]
    fn test_merge_overlapping_ranges() {
        let mut entry = AttributionEntry {
            lines: Vec::new(),
            author_key: "a".to_string(),
            prompt_session_id: None,
        };
        entry.add_lines(&[LineRange::Range(1, 8)]);
        entry.add_lines(&[LineRange::Range(5, 12)]);
        assert_eq!(entry.lines.len(), 1);
        assert_eq!(entry.lines[0], LineRange::Range(1, 12));
    }

    #[test]
    fn test_merge_single_lines_with_ranges() {
        let mut entry = AttributionEntry {
            lines: Vec::new(),
            author_key: "a".to_string(),
            prompt_session_id: None,
        };
        entry.add_lines(&[LineRange::Range(1, 5)]);
        entry.add_lines(&[LineRange::Single(6)]);
        entry.add_lines(&[LineRange::Single(7)]);
        assert_eq!(entry.lines.len(), 1);
        assert_eq!(entry.lines[0], LineRange::Range(1, 7));
    }

    #[test]
    fn test_merge_adjacent_single_lines() {
        let mut entry = AttributionEntry {
            lines: Vec::new(),
            author_key: "a".to_string(),
            prompt_session_id: None,
        };
        entry.add_lines(&[LineRange::Single(5)]);
        entry.add_lines(&[LineRange::Single(6)]);
        entry.add_lines(&[LineRange::Single(7)]);
        assert_eq!(entry.lines.len(), 1);
        assert_eq!(entry.lines[0], LineRange::Range(5, 7));
    }

    #[test]
    fn test_complex_merging_scenario() {
        let mut entry = AttributionEntry {
            lines: Vec::new(),
            author_key: "a".to_string(),
            prompt_session_id: None,
        };
        entry.add_lines(&[LineRange::Range(1, 3)]);
        entry.add_lines(&[LineRange::Single(4)]);
        entry.add_lines(&[LineRange::Range(5, 7)]);
        entry.add_lines(&[LineRange::Single(8)]);
        entry.add_lines(&[LineRange::Range(10, 12)]);
        entry.add_lines(&[LineRange::Single(13)]);
        entry.add_lines(&[LineRange::Single(14)]);
        assert_eq!(entry.lines.len(), 2);
        assert_eq!(entry.lines[0], LineRange::Range(1, 8));
        assert_eq!(entry.lines[1], LineRange::Range(10, 14));
    }

    #[test]
    fn test_duplicate_ranges_issue_fix() {
        let mut entry = AttributionEntry {
            lines: Vec::new(),
            author_key: "a".to_string(),
            prompt_session_id: None,
        };
        for _ in 0..100 {
            entry.add_lines(&[LineRange::Range(412, 414)]);
        }
        for _ in 0..100 {
            entry.add_lines(&[LineRange::Single(420)]);
        }
        for _ in 0..100 {
            entry.add_lines(&[LineRange::Range(423, 424)]);
        }
        assert_eq!(entry.lines.len(), 3);
        assert_eq!(entry.lines[0], LineRange::Range(412, 414));
        assert_eq!(entry.lines[1], LineRange::Single(420));
        assert_eq!(entry.lines[2], LineRange::Range(423, 424));
        let start_lines: Vec<u32> = entry
            .lines
            .iter()
            .map(|range| match range {
                LineRange::Single(l) => *l,
                LineRange::Range(start, _) => *start,
            })
            .collect();
        assert_eq!(start_lines, vec![412, 420, 423]);
    }

    #[test]
    fn test_massive_duplicate_ranges_from_long_working_log() {
        // Simulate the issue from the user's JSON example with massive duplicates
        let mut entry = AttributionEntry {
            lines: Vec::new(),
            author_key: "e892bede".to_string(),
            prompt_session_id: Some("bacce5c7-2718-45ef-8364-b916522bcde3".to_string()),
        };

        // Add the same ranges many times (like in the user's example)
        for _ in 0..50 {
            entry.add_lines(&[LineRange::Range(1, 5)]);
        }
        for _ in 0..50 {
            entry.add_lines(&[LineRange::Single(7)]);
        }
        for _ in 0..50 {
            entry.add_lines(&[LineRange::Range(9, 48)]);
        }
        for _ in 0..50 {
            entry.add_lines(&[LineRange::Range(70, 71)]);
        }
        for _ in 0..50 {
            entry.add_lines(&[LineRange::Range(125, 141)]);
        }
        for _ in 0..50 {
            entry.add_lines(&[LineRange::Range(143, 149)]);
        }

        // Should be compressed to just 6 unique ranges
        assert_eq!(entry.lines.len(), 6);
        assert_eq!(entry.lines[0], LineRange::Range(1, 5));
        assert_eq!(entry.lines[1], LineRange::Single(7));
        assert_eq!(entry.lines[2], LineRange::Range(9, 48));
        assert_eq!(entry.lines[3], LineRange::Range(70, 71));
        assert_eq!(entry.lines[4], LineRange::Range(125, 141));
        assert_eq!(entry.lines[5], LineRange::Range(143, 149));
    }

    #[test]
    fn test_prompt_attribution_in_authorship_log() {
        use crate::log_fmt::transcript::{AiTranscript, Message};
        use crate::log_fmt::working_log::AgentId;

        let entry =
            WorkingLogEntry::new("src/test.rs".to_string(), vec![Line::Range(1, 10)], vec![]);

        let user_message = Message::user("Please add error handling to this function".to_string());
        let assistant_message =
            Message::assistant("I'll add error handling to the function.".to_string());

        let mut transcript = AiTranscript::new();
        transcript.add_message(user_message);
        transcript.add_message(assistant_message);

        let agent_id = AgentId {
            tool: "cursor".to_string(),
            id: "session-abc123".to_string(),
            model: "claude-3-sonnet".to_string(),
        };

        let mut checkpoint = Checkpoint::new(
            "abc123".to_string(),
            "".to_string(),
            "claude".to_string(),
            vec![entry],
        );
        checkpoint.transcript = Some(transcript);
        checkpoint.agent_id = Some(agent_id);

        let checkpoints = vec![checkpoint];
        let authorship_log = AuthorshipLog::from_working_log(&checkpoints);
        let entries = &authorship_log.files["src/test.rs"];
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(e.prompt_session_id.as_deref(), Some("session-abc123"));
        // Check prompt registry stores the agent session
        let prompt = authorship_log.prompts.get("session-abc123").unwrap();
        assert_eq!(prompt.agent_id.tool, "cursor");
        assert_eq!(prompt.agent_id.id, "session-abc123");

        // Verify that lines are still attributed correctly (username)
        let who = resolve_username(&authorship_log, "src/test.rs", 5).unwrap();
        assert_eq!(who, "claude");
    }

    #[test]
    fn test_authorship_log_json_example() {
        use crate::log_fmt::transcript::{AiTranscript, Message};
        use crate::log_fmt::working_log::AgentId;

        // Create a checkpoint with a transcript
        let user_message = Message::user("Please add error handling to this function".to_string());
        let assistant_message = Message::assistant(
            "I'll add comprehensive error handling with try-catch blocks".to_string(),
        );

        let mut transcript = AiTranscript::new();
        transcript.add_message(user_message);
        transcript.add_message(assistant_message);

        let agent_id = AgentId {
            tool: "cursor".to_string(),
            id: "session-abc123".to_string(),
            model: "claude-3-sonnet".to_string(),
        };

        let checkpoint = Checkpoint {
            snapshot: "abc123".to_string(),
            diff: "diff content".to_string(),
            author: "john.doe".to_string(),
            timestamp: 1234567890,
            entries: vec![crate::log_fmt::working_log::WorkingLogEntry {
                file: "src/main.rs".to_string(),
                added_lines: vec![crate::log_fmt::working_log::Line::Range(5, 10)],
                deleted_lines: vec![],
            }],
            transcript: Some(transcript),
            agent_id: Some(agent_id),
        };

        // Convert to authorship log
        let authorship_log = AuthorshipLog::from_working_log(&[checkpoint]);

        // Verify the structure
        assert_eq!(authorship_log.files.len(), 1);
        assert_eq!(authorship_log.prompts.len(), 1);
        assert!(authorship_log.files.contains_key("src/main.rs"));

        // Verify prompt session registry stores agent metadata
        let prompt_id = authorship_log.prompts.keys().next().unwrap();
        let stored_prompt = authorship_log.prompts.get(prompt_id).unwrap();
        assert_eq!(stored_prompt.agent_id.tool, "cursor");
        assert_eq!(stored_prompt.agent_id.id, "session-abc123");
    }

    #[test]
    fn test_multiple_prompts_same_tool_separate_entries() {
        use crate::log_fmt::transcript::{AiTranscript, Message};
        use crate::log_fmt::working_log::AgentId;

        // Create two different transcripts from the same tool (cursor)
        let mut transcript1 = AiTranscript::new();
        transcript1.add_message(Message::user("Add error handling".to_string()));

        let agent_id1 = AgentId {
            tool: "cursor".to_string(),
            id: "session-abc123".to_string(),
            model: "claude-3-sonnet".to_string(),
        };

        let mut transcript2 = AiTranscript::new();
        transcript2.add_message(Message::user("Add logging".to_string()));

        let agent_id2 = AgentId {
            tool: "cursor".to_string(),
            id: "session-xyz789".to_string(),
            model: "gpt-4".to_string(),
        };

        // Create two checkpoints with different transcripts
        let checkpoint1 = Checkpoint {
            snapshot: "abc123".to_string(),
            diff: "diff1".to_string(),
            author: "john.doe".to_string(),
            timestamp: 1234567890,
            entries: vec![crate::log_fmt::working_log::WorkingLogEntry {
                file: "src/main.rs".to_string(),
                added_lines: vec![crate::log_fmt::working_log::Line::Range(5, 10)],
                deleted_lines: vec![],
            }],
            transcript: Some(transcript1),
            agent_id: Some(agent_id1),
        };

        let checkpoint2 = Checkpoint {
            snapshot: "xyz789".to_string(),
            diff: "diff2".to_string(),
            author: "john.doe".to_string(),
            timestamp: 1234567891,
            entries: vec![crate::log_fmt::working_log::WorkingLogEntry {
                file: "src/main.rs".to_string(),
                added_lines: vec![crate::log_fmt::working_log::Line::Range(15, 20)],
                deleted_lines: vec![],
            }],
            transcript: Some(transcript2),
            agent_id: Some(agent_id2),
        };

        // Convert to authorship log
        let authorship_log = AuthorshipLog::from_working_log(&[checkpoint1, checkpoint2]);

        // Should have two separate attribution entries for the same human with different prompts
        let file_entries = &authorship_log.files["src/main.rs"];
        assert_eq!(file_entries.len(), 2);

        // Both should reference different prompt session ids
        let mut prompts: Vec<String> = file_entries
            .iter()
            .map(|a| a.prompt_session_id.clone().expect("prompt expected"))
            .collect();
        prompts.sort();
        prompts.dedup();
        assert_eq!(prompts.len(), 2);

        // Verify prompts are stored separately
        assert_eq!(authorship_log.prompts.len(), 2);
    }

    #[test]
    fn test_mixed_human_and_agent_attribution() {
        use crate::log_fmt::transcript::{AiTranscript, Message};
        use crate::log_fmt::working_log::AgentId;

        // Human-written entry
        let human_entry =
            WorkingLogEntry::new("src/test.rs".to_string(), vec![Line::Range(1, 5)], vec![]);
        let human_checkpoint = Checkpoint::new(
            "abc123".to_string(),
            "".to_string(),
            "john.doe".to_string(),
            vec![human_entry],
        );

        // AI-generated entry
        let ai_entry =
            WorkingLogEntry::new("src/test.rs".to_string(), vec![Line::Range(6, 10)], vec![]);
        let user_message = Message::user("Add error handling".to_string());

        let mut transcript = AiTranscript::new();
        transcript.add_message(user_message);

        let agent_id = AgentId {
            tool: "cursor".to_string(),
            id: "session-xyz789".to_string(),
            model: "claude-3-sonnet".to_string(),
        };

        let mut ai_checkpoint = Checkpoint::new(
            "def456".to_string(),
            "".to_string(),
            "claude".to_string(),
            vec![ai_entry],
        );
        ai_checkpoint.transcript = Some(transcript);
        ai_checkpoint.agent_id = Some(agent_id);

        let checkpoints = vec![human_checkpoint, ai_checkpoint];
        let authorship_log = AuthorshipLog::from_working_log(&checkpoints);
        let entries = &authorship_log.files["src/test.rs"];
        assert_eq!(entries.len(), 2);
        assert_eq!(
            resolve_username(&authorship_log, "src/test.rs", 3),
            Some("john.doe".to_string())
        );
        // AI-assisted lines still map to the same human username internally
        assert_eq!(
            resolve_username(&authorship_log, "src/test.rs", 8),
            Some("claude".to_string())
        );
    }

    #[test]
    fn test_base_commit_sha_field() {
        let entry =
            WorkingLogEntry::new("src/test.rs".to_string(), vec![Line::Range(1, 5)], vec![]);
        let checkpoint = Checkpoint::new(
            "abc123".to_string(),
            "".to_string(),
            "test_user".to_string(),
            vec![entry],
        );

        // Test with empty base commit SHA
        let authorship_log = AuthorshipLog::from_working_log(&[checkpoint.clone()]);
        assert_eq!(authorship_log.base_commit_sha, "");

        // Test with specific base commit SHA
        let base_sha = "dcdd5667741816262deb45aaa7958cba68a6a72a";
        let authorship_log_with_base =
            AuthorshipLog::from_working_log_with_base_commit(&[checkpoint], base_sha);
        assert_eq!(authorship_log_with_base.base_commit_sha, base_sha);
    }
}
