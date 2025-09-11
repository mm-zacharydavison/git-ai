use crate::log_fmt::working_log::{Checkpoint, Line, Prompt};
use serde::de::{self, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde::{Deserializer, Serializer, ser::SerializeSeq};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;

/// Semantic version for the authorship log format
pub const AUTHORSHIP_LOG_VERSION: &str = "authorship/0.0.2";

/// Represents the source of code attribution - either human or AI agent
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Attribution {
    Human { username: String },
    Agent { prompt_id: String },
}

impl Attribution {
    /// Derive the "author" field from the attribution
    /// For agents, this requires resolving the prompt_id to get the actual agent details
    pub fn author(&self, prompts: &BTreeMap<String, Prompt>) -> String {
        match self {
            Attribution::Human { username } => username.clone(),
            Attribution::Agent { prompt_id } => {
                if let Some(prompt) = prompts.get(prompt_id) {
                    prompt.agent_id.tool.clone() // Just return "cursor", "windsurf", etc.
                } else {
                    prompt_id.clone() // Fallback to hash if prompt not found
                }
            }
        }
    }
}

/// Represents a line range with its attribution source
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineAttribution {
    pub range: LineRange,
    pub attribution: Attribution,
}

/// Represents either a single line or a range of lines
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
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

impl serde::Serialize for LineRange {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            LineRange::Single(l) => serializer.serialize_u32(*l),
            LineRange::Range(start, end) => {
                let mut seq = serializer.serialize_seq(Some(2))?;
                seq.serialize_element(start)?;
                seq.serialize_element(end)?;
                seq.end()
            }
        }
    }
}

impl<'de> serde::Deserialize<'de> for LineRange {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct LineRangeVisitor;
        impl<'de> Visitor<'de> for LineRangeVisitor {
            type Value = LineRange;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("an integer or a two-element array")
            }
            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(LineRange::Single(value as u32))
            }
            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let start: u32 = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;
                let end: u32 = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                Ok(LineRange::Range(start, end))
            }
        }
        deserializer.deserialize_any(LineRangeVisitor)
    }
}

/// Represents a line range with its author
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthoredRange {
    /// Line range [start, end] (inclusive)
    pub range: (u32, u32),
    /// Author of this line range
    pub author: String,
}

#[allow(dead_code)]
impl AuthoredRange {
    pub fn new(start: u32, end: u32, author: String) -> Self {
        Self {
            range: (start, end),
            author,
        }
    }

    pub fn start(&self) -> u32 {
        self.range.0
    }

    pub fn end(&self) -> u32 {
        self.range.1
    }

    /// Check if this range overlaps with another range
    pub fn overlaps(&self, other: &AuthoredRange) -> bool {
        self.start() <= other.end() && other.start() <= self.end()
    }

    /// Check if this range contains a specific line
    pub fn contains(&self, line: u32) -> bool {
        line >= self.start() && line <= self.end()
    }
}

impl fmt::Display for AuthoredRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.start() == self.end() {
            write!(f, "[{}, \"{}\"]", self.start(), self.author)
        } else {
            write!(
                f,
                "[[{}, {}], \"{}\"]",
                self.start(),
                self.end(),
                self.author
            )
        }
    }
}

/// Represents an author with their line ranges
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorEntry {
    pub author: String, // Derived from attribution for backward compatibility
    pub lines: Vec<LineRange>,
    pub attribution: Attribution, // The actual attribution source
}

impl PartialEq for AuthorEntry {
    fn eq(&self, other: &Self) -> bool {
        self.author == other.author && self.lines == other.lines
    }
}

impl AuthorEntry {
    #[allow(dead_code)]
    pub fn new(author: String) -> Self {
        Self {
            author: author.clone(),
            lines: Vec::new(),
            attribution: Attribution::Human { username: author },
        }
    }

    pub fn new_with_attribution(
        attribution: Attribution,
        prompts: &BTreeMap<String, Prompt>,
    ) -> Self {
        Self {
            author: attribution.author(prompts),
            lines: Vec::new(),
            attribution,
        }
    }

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

    pub fn get_author_for_line(&self, line: u32) -> Option<&str> {
        for range in &self.lines {
            if range.contains(line) {
                return Some(&self.author);
            }
        }
        None
    }

    /// Deduplicate and merge overlapping/adjacent ranges
    fn deduplicate_and_merge_ranges(&mut self) {
        if self.lines.is_empty() {
            return;
        }

        // First, deduplicate identical ranges
        self.lines.dedup();

        // Keep merging until no more merges are possible
        loop {
            let mut merged_any = false;
            let mut i = 0;

            while i < self.lines.len() {
                let mut j = i + 1;
                let mut merged_with = None;

                // Look for a range that can be merged with the current one
                while j < self.lines.len() {
                    if self.ranges_can_merge(&self.lines[i], &self.lines[j]) {
                        // Merge the ranges
                        let merged = self.merge_ranges(&self.lines[i], &self.lines[j]);
                        self.lines[i] = merged;
                        merged_with = Some(j);
                        merged_any = true;
                        break;
                    }
                    j += 1;
                }

                if let Some(j) = merged_with {
                    // Remove the merged range
                    self.lines.remove(j);
                } else {
                    i += 1;
                }
            }

            // If no merges happened in this pass, we're done
            if !merged_any {
                break;
            }
        }

        // Ensure the final result is sorted by start line number
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
    }

    /// Check if two ranges can be merged (overlapping or adjacent)
    fn ranges_can_merge(&self, range1: &LineRange, range2: &LineRange) -> bool {
        match (range1, range2) {
            (LineRange::Single(l1), LineRange::Single(l2)) => {
                // Adjacent single lines
                l1.abs_diff(*l2) <= 1
            }
            (LineRange::Single(l), LineRange::Range(start, end)) => {
                // Single line adjacent to or overlapping with range
                *l >= start.saturating_sub(1) && *l <= end + 1
            }
            (LineRange::Range(start, end), LineRange::Single(l)) => {
                // Range adjacent to or overlapping with single line
                *l >= start.saturating_sub(1) && *l <= end + 1
            }
            (LineRange::Range(start1, end1), LineRange::Range(start2, end2)) => {
                // Two ranges overlap or are adjacent
                start1 <= &(end2 + 1) && start2 <= &(end1 + 1)
            }
        }
    }

    /// Merge two ranges that can be merged
    fn merge_ranges(&self, range1: &LineRange, range2: &LineRange) -> LineRange {
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

/// Per-file authorship: author -> set of line numbers (sorted, unique)
#[derive(Debug, Clone, PartialEq)]
pub struct FileAuthorship {
    pub file: String,
    pub authors: Vec<AuthorEntry>,
}

impl serde::Serialize for FileAuthorship {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("FileAuthorship", 2)?;
        state.serialize_field("file", &self.file)?;
        state.serialize_field("authors", &self.authors)?;
        state.end()
    }
}

impl<'de> serde::Deserialize<'de> for FileAuthorship {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        struct FileAuthorshipHelper {
            file: String,
            authors: Vec<AuthorEntry>,
        }
        let helper = FileAuthorshipHelper::deserialize(deserializer)?;
        Ok(FileAuthorship {
            file: helper.file,
            authors: helper.authors,
        })
    }
}

impl FileAuthorship {
    pub fn new(file: String) -> Self {
        Self {
            file,
            authors: Vec::new(),
        }
    }

    /// Add lines for an author, removing them from all other authors
    pub fn add_lines(&mut self, author: &str, lines: &[u32]) {
        // Create attribution from author string (backward compatibility)
        let attribution = Attribution::Human {
            username: author.to_string(),
        };
        self.add_lines_with_attribution(attribution, lines, &BTreeMap::new());
    }

    /// Add lines with specific attribution, removing them from all other authors
    pub fn add_lines_with_attribution(
        &mut self,
        attribution: Attribution,
        lines: &[u32],
        prompts: &BTreeMap<String, Prompt>,
    ) {
        // Create a single range to remove from all other authors
        let lines_to_remove = LineRange::compress_lines(lines);

        // Remove these lines from all other authors
        for other_author in &mut self.authors {
            other_author.remove_lines(&lines_to_remove);
        }

        // Add to this author with compression
        let author_key = attribution.author(prompts);
        if let Some(entry) = self.authors.iter_mut().find(|a| a.author == author_key) {
            entry.add_lines(&lines_to_remove);
        } else {
            // Create new author entry
            let mut new_entry = AuthorEntry::new_with_attribution(attribution, prompts);
            new_entry.add_lines(&lines_to_remove);
            self.authors.push(new_entry);
        }
    }

    /// Check if this file has any authorship information
    pub fn is_empty(&self) -> bool {
        self.authors.iter().all(|a| a.is_empty())
    }

    pub fn get_author(&self, line: u32) -> Option<&str> {
        // Check authors in reverse order (most recent first)
        for author in self.authors.iter().rev() {
            if let Some(author) = author.get_author_for_line(line) {
                return Some(author);
            }
        }
        None
    }
}

impl fmt::Display for FileAuthorship {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.file)?;
        for author in &self.authors {
            write!(f, "\n  Author: {}", author.author)?;
            for range in &author.lines {
                write!(f, " {}", range)?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuthorshipLog {
    pub files: BTreeMap<String, FileAuthorship>,
    pub prompts: BTreeMap<String, Prompt>, // Global prompts by hash
    pub schema_version: String,
}

impl AuthorshipLog {
    pub fn new() -> Self {
        Self {
            files: BTreeMap::new(),
            prompts: BTreeMap::new(),
            schema_version: AUTHORSHIP_LOG_VERSION.to_string(),
        }
    }

    pub fn get_or_create_file(&mut self, file: &str) -> &mut FileAuthorship {
        self.files
            .entry(file.to_string())
            .or_insert_with(|| FileAuthorship::new(file.to_string()))
    }

    /// Generate a unique prompt ID based on the prompt content
    fn generate_prompt_id(prompt: &Prompt) -> String {
        // Create a hash of the prompt content for uniqueness
        let mut hasher = Sha256::new();
        hasher.update(prompt.agent_id.tool.as_bytes());
        hasher.update(prompt.agent_id.id.as_bytes());
        for message in &prompt.messages {
            hasher.update(message.text.as_bytes());
            hasher.update(format!("{:?}", message.role).as_bytes());
            if let Some(ref username) = message.username {
                hasher.update(username.as_bytes());
            }
            hasher.update(&message.timestamp.to_le_bytes());
        }
        format!("{:x}", hasher.finalize())
    }

    /// Add a prompt to the global prompts dictionary and return its ID
    fn add_prompt(&mut self, prompt: Prompt) -> String {
        let prompt_id = Self::generate_prompt_id(&prompt);
        self.prompts.entry(prompt_id.clone()).or_insert(prompt);
        prompt_id
    }

    /// Convert from working log checkpoints to authorship log
    pub fn from_working_log(checkpoints: &[Checkpoint]) -> Self {
        let mut authorship_log = AuthorshipLog::new();

        // First pass: collect all unique prompts and create prompt IDs
        let mut prompt_id_map = std::collections::HashMap::new();
        for checkpoint in checkpoints.iter() {
            if let Some(prompt) = &checkpoint.prompt {
                let prompt_id = authorship_log.add_prompt(prompt.clone());
                prompt_id_map.insert(checkpoint.snapshot.clone(), prompt_id);
            }
        }

        // Create a map of prompt_id -> tool_name for quick lookup
        let mut prompt_tool_map = std::collections::HashMap::new();
        for (prompt_id, prompt) in &authorship_log.prompts {
            prompt_tool_map.insert(prompt_id.clone(), prompt.agent_id.tool.clone());
        }

        // Second pass: process checkpoints and create attributions
        for checkpoint in checkpoints.iter() {
            for entry in &checkpoint.entries {
                let file_auth = authorship_log.get_or_create_file(&entry.file);

                // Process deletions first (remove lines from all authors)
                for line in &entry.deleted_lines {
                    let to_remove = match line {
                        Line::Single(l) => LineRange::Single(*l),
                        Line::Range(start, end) => LineRange::Range(*start, *end),
                    };

                    for author in &mut file_auth.authors {
                        author.remove_lines(&[to_remove.clone()]);
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
                    // Create attribution based on whether there's a prompt
                    let attribution =
                        if let Some(prompt_id) = prompt_id_map.get(&checkpoint.snapshot) {
                            Attribution::Agent {
                                prompt_id: prompt_id.clone(),
                            }
                        } else {
                            Attribution::Human {
                                username: checkpoint.author.clone(),
                            }
                        };

                    // Get the author name for this attribution
                    let author_name = match &attribution {
                        Attribution::Human { username } => username.clone(),
                        Attribution::Agent { prompt_id } => {
                            prompt_tool_map
                                .get(prompt_id)
                                .cloned()
                                .unwrap_or_else(|| prompt_id.clone()) // Fallback to hash if not found
                        }
                    };

                    // Create a single range to remove from all other authors
                    let lines_to_remove = LineRange::compress_lines(&added_lines);

                    // Remove these lines from all other authors
                    for other_author in &mut file_auth.authors {
                        other_author.remove_lines(&lines_to_remove);
                    }

                    // Add to this author with compression
                    // Group by attribution (not just author name) so each prompt gets its own entry
                    if let Some(entry) = file_auth
                        .authors
                        .iter_mut()
                        .find(|a| a.attribution == attribution)
                    {
                        entry.add_lines(&lines_to_remove);
                    } else {
                        // Create new author entry
                        let mut new_entry = AuthorEntry {
                            author: author_name,
                            lines: Vec::new(),
                            attribution,
                        };
                        new_entry.add_lines(&lines_to_remove);
                        file_auth.authors.push(new_entry);
                    }
                }
            }
        }
        // Remove empty files
        authorship_log.files.retain(|_, f| !f.is_empty());
        authorship_log
    }
}

impl Default for AuthorshipLog {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for AuthorshipLog {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (_file_path, file_auth) in &self.files {
            writeln!(f, "{}", file_auth)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log_fmt::working_log::{Checkpoint, Line, WorkingLogEntry};

    #[test]
    fn test_authored_range_creation() {
        let range = AuthoredRange::new(1, 10, "claude".to_string());
        assert_eq!(range.start(), 1);
        assert_eq!(range.end(), 10);
        assert_eq!(range.author, "claude");
    }

    #[test]
    fn test_authored_range_overlaps() {
        let range1 = AuthoredRange::new(1, 10, "claude".to_string());
        let range2 = AuthoredRange::new(5, 15, "user".to_string());
        let range3 = AuthoredRange::new(20, 30, "claude".to_string());

        assert!(range1.overlaps(&range2));
        assert!(range2.overlaps(&range1));
        assert!(!range1.overlaps(&range3));
        assert!(!range3.overlaps(&range1));
    }

    #[test]
    fn test_file_authorship_add_lines() {
        let mut file_auth = FileAuthorship::new("src/test.rs".to_string());
        file_auth.add_lines("claude", &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
        assert_eq!(file_auth.get_author(5), Some("claude"));
        file_auth.add_lines("aidan", &[5, 6, 50]);
        assert_eq!(file_auth.get_author(5), Some("aidan"));
        assert_eq!(file_auth.get_author(6), Some("aidan"));
        assert_eq!(file_auth.get_author(50), Some("aidan"));
        assert_eq!(file_auth.get_author(4), Some("claude"));
        assert_eq!(file_auth.get_author(7), Some("claude"));
        assert_eq!(file_auth.get_author(100), None);
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
        let file_auth = &authorship_log.files["src/test.rs"];
        assert_eq!(file_auth.get_author(5), Some("aidan"));
        assert_eq!(file_auth.get_author(6), Some("aidan"));
        assert_eq!(file_auth.get_author(50), Some("aidan"));
        assert_eq!(file_auth.get_author(4), Some("claude"));
        assert_eq!(file_auth.get_author(7), Some("claude"));
        assert_eq!(file_auth.get_author(100), None);
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
        let file_auth = &authorship_log.files["src/test.rs"];
        assert_eq!(file_auth.get_author(5), None);
        assert_eq!(file_auth.get_author(8), None);
        assert_eq!(file_auth.get_author(9), None);
        assert_eq!(file_auth.get_author(10), None);
        assert_eq!(file_auth.get_author(4), Some("claude"));
        assert_eq!(file_auth.get_author(6), Some("claude"));
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
        let file_auth = &authorship_log.files["src/test.rs"];

        // Lines 1-7: claude
        assert_eq!(file_auth.get_author(1), Some("claude"));
        assert_eq!(file_auth.get_author(7), Some("claude"));
        // Lines 8-12: aidan
        assert_eq!(file_auth.get_author(8), Some("aidan"));
        assert_eq!(file_auth.get_author(12), Some("aidan"));
        // Lines 13-20: claude
        assert_eq!(file_auth.get_author(13), Some("claude"));
        assert_eq!(file_auth.get_author(20), Some("claude"));
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
        let file_auth = &authorship_log.files["src/test.rs"];

        // Lines 1-7: claude
        assert_eq!(file_auth.get_author(1), Some("claude"));
        assert_eq!(file_auth.get_author(7), Some("claude"));
        // Lines 8-12: deleted (None)
        assert_eq!(file_auth.get_author(8), None);
        assert_eq!(file_auth.get_author(12), None);
        // Lines 13-20: claude
        assert_eq!(file_auth.get_author(13), Some("claude"));
        assert_eq!(file_auth.get_author(20), Some("claude"));
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
        let file_auth = &authorship_log.files["src/test.rs"];

        // Debug: print the actual authorship data
        println!("Debug: File authorship data:");
        for author in &file_auth.authors {
            println!("  Author: {}", author.author);
            for range in &author.lines {
                println!("    Range: {}", range);
            }
        }

        // Lines 1-4: claude
        assert_eq!(file_auth.get_author(1), Some("claude"));
        assert_eq!(file_auth.get_author(4), Some("claude"));
        // Lines 5-9: aidan
        assert_eq!(file_auth.get_author(5), Some("aidan"));
        assert_eq!(file_auth.get_author(9), Some("aidan"));
        // Lines 10-20: user (overwrites both claude and aidan)
        assert_eq!(file_auth.get_author(10), Some("user"));
        assert_eq!(file_auth.get_author(20), Some("user"));
        // Lines 21-30: claude
        assert_eq!(file_auth.get_author(21), Some("claude"));
        assert_eq!(file_auth.get_author(30), Some("claude"));
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
        let file_auth = &authorship_log.files["src/test.rs"];

        // Specific lines taken by aidan
        assert_eq!(file_auth.get_author(5), Some("aidan"));
        assert_eq!(file_auth.get_author(10), Some("aidan"));
        assert_eq!(file_auth.get_author(15), Some("aidan"));
        // Other lines still claude
        assert_eq!(file_auth.get_author(4), Some("claude"));
        assert_eq!(file_auth.get_author(6), Some("claude"));
        assert_eq!(file_auth.get_author(9), Some("claude"));
        assert_eq!(file_auth.get_author(11), Some("claude"));
        assert_eq!(file_auth.get_author(14), Some("claude"));
        assert_eq!(file_auth.get_author(16), Some("claude"));
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
        let file_auth = &authorship_log.files["src/test.rs"];

        // Boundary lines taken by aidan
        assert_eq!(file_auth.get_author(1), Some("aidan"));
        assert_eq!(file_auth.get_author(10), Some("aidan"));
        // Middle lines still claude
        assert_eq!(file_auth.get_author(5), Some("claude"));
        assert_eq!(file_auth.get_author(9), Some("claude"));
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

        // The file should have authorship information
        assert!(
            authorship_log
                .files
                .contains_key("src/commands/post_commit.rs")
        );
        let file_auth = &authorship_log.files["src/commands/post_commit.rs"];

        // Lines 86-89 and 98-102 should be authored by "aidan"
        assert_eq!(file_auth.get_author(86), Some("aidan"));
        assert_eq!(file_auth.get_author(87), Some("aidan"));
        assert_eq!(file_auth.get_author(88), Some("aidan"));
        assert_eq!(file_auth.get_author(89), Some("aidan"));
        assert_eq!(file_auth.get_author(98), Some("aidan"));
        assert_eq!(file_auth.get_author(99), Some("aidan"));
        assert_eq!(file_auth.get_author(100), Some("aidan"));
        assert_eq!(file_auth.get_author(101), Some("aidan"));
        assert_eq!(file_auth.get_author(102), Some("aidan"));

        // Lines outside the added ranges should not have authorship
        assert_eq!(file_auth.get_author(85), None);
        assert_eq!(file_auth.get_author(90), None);
        assert_eq!(file_auth.get_author(97), None);
        assert_eq!(file_auth.get_author(103), None);
        assert_eq!(file_auth.get_author(117), None);
    }

    #[test]
    fn test_line_range_compact_serialization() {
        use serde_json;
        let ranges = vec![
            LineRange::Single(1),
            LineRange::Range(2, 4),
            LineRange::Single(19),
        ];
        let json = serde_json::to_string(&ranges).unwrap();
        assert_eq!(json, "[1,[2,4],19]");
        let deserialized: Vec<LineRange> = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, ranges);
    }

    #[test]
    fn test_new_authorship_format_serialization() {
        use serde_json;
        let mut file_auth = FileAuthorship::new("src/test.rs".to_string());
        file_auth.add_lines("claude", &[1, 2, 3, 4, 5]);
        file_auth.add_lines("aidan", &[6, 7, 8]);

        let json = serde_json::to_string(&file_auth).unwrap();
        println!("Serialized format: {}", json);

        // Verify the structure matches the new format
        let deserialized: FileAuthorship = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.file, "src/test.rs");
        assert_eq!(deserialized.authors.len(), 2);

        // Check that authors are in the expected format
        let claude_entry = deserialized
            .authors
            .iter()
            .find(|a| a.author == "claude")
            .unwrap();
        let aidan_entry = deserialized
            .authors
            .iter()
            .find(|a| a.author == "aidan")
            .unwrap();

        assert_eq!(claude_entry.lines.len(), 1); // Should be compressed to one range
        assert_eq!(aidan_entry.lines.len(), 1); // Should be compressed to one range
    }

    #[test]
    fn test_deduplicate_identical_ranges() {
        let mut author_entry = AuthorEntry::new("claude".to_string());

        // Add the same range multiple times
        author_entry.add_lines(&[LineRange::Range(1, 5)]);
        author_entry.add_lines(&[LineRange::Range(1, 5)]);
        author_entry.add_lines(&[LineRange::Range(1, 5)]);

        // Should only have one range after deduplication
        assert_eq!(author_entry.lines.len(), 1);
        assert_eq!(author_entry.lines[0], LineRange::Range(1, 5));
    }

    #[test]
    fn test_merge_adjacent_ranges() {
        let mut author_entry = AuthorEntry::new("claude".to_string());

        // Add adjacent ranges
        author_entry.add_lines(&[LineRange::Range(1, 5)]);
        author_entry.add_lines(&[LineRange::Range(6, 10)]);

        // Should be merged into one range
        assert_eq!(author_entry.lines.len(), 1);
        assert_eq!(author_entry.lines[0], LineRange::Range(1, 10));
    }

    #[test]
    fn test_merge_overlapping_ranges() {
        let mut author_entry = AuthorEntry::new("claude".to_string());

        // Add overlapping ranges
        author_entry.add_lines(&[LineRange::Range(1, 8)]);
        author_entry.add_lines(&[LineRange::Range(5, 12)]);

        // Should be merged into one range
        assert_eq!(author_entry.lines.len(), 1);
        assert_eq!(author_entry.lines[0], LineRange::Range(1, 12));
    }

    #[test]
    fn test_merge_single_lines_with_ranges() {
        let mut author_entry = AuthorEntry::new("claude".to_string());

        // Add a range and adjacent single lines
        author_entry.add_lines(&[LineRange::Range(1, 5)]);
        author_entry.add_lines(&[LineRange::Single(6)]);
        author_entry.add_lines(&[LineRange::Single(7)]);

        // Should be merged into one range
        assert_eq!(author_entry.lines.len(), 1);
        assert_eq!(author_entry.lines[0], LineRange::Range(1, 7));
    }

    #[test]
    fn test_merge_adjacent_single_lines() {
        let mut author_entry = AuthorEntry::new("claude".to_string());

        // Add adjacent single lines
        author_entry.add_lines(&[LineRange::Single(5)]);
        author_entry.add_lines(&[LineRange::Single(6)]);
        author_entry.add_lines(&[LineRange::Single(7)]);

        // Should be merged into one range
        assert_eq!(author_entry.lines.len(), 1);
        assert_eq!(author_entry.lines[0], LineRange::Range(5, 7));
    }

    #[test]
    fn test_complex_merging_scenario() {
        let mut author_entry = AuthorEntry::new("claude".to_string());

        // Add a complex mix of ranges and single lines
        author_entry.add_lines(&[LineRange::Range(1, 3)]);
        author_entry.add_lines(&[LineRange::Single(4)]);
        author_entry.add_lines(&[LineRange::Range(5, 7)]);
        author_entry.add_lines(&[LineRange::Single(8)]);
        author_entry.add_lines(&[LineRange::Range(10, 12)]);
        author_entry.add_lines(&[LineRange::Single(13)]);
        author_entry.add_lines(&[LineRange::Single(14)]);

        // Should result in two ranges: [1,8] and [10,14]
        assert_eq!(author_entry.lines.len(), 2);
        assert_eq!(author_entry.lines[0], LineRange::Range(1, 8));
        assert_eq!(author_entry.lines[1], LineRange::Range(10, 14));
    }

    #[test]
    fn test_duplicate_ranges_issue_fix() {
        // This test simulates the exact issue reported: massive amounts of duplicate line ranges
        let mut author_entry = AuthorEntry::new("claude".to_string());

        // Simulate the same range being added hundreds of times (like in the reported issue)
        for _ in 0..100 {
            author_entry.add_lines(&[LineRange::Range(412, 414)]);
        }
        for _ in 0..100 {
            author_entry.add_lines(&[LineRange::Single(420)]);
        }
        for _ in 0..100 {
            author_entry.add_lines(&[LineRange::Range(423, 424)]);
        }

        // After deduplication and merging, we should have only 3 unique ranges
        assert_eq!(author_entry.lines.len(), 3);

        // The ranges should be sorted by their start position
        // Range(412, 414) comes first, then Single(420), then Range(423, 424)
        assert_eq!(author_entry.lines[0], LineRange::Range(412, 414));
        assert_eq!(author_entry.lines[1], LineRange::Single(420));
        assert_eq!(author_entry.lines[2], LineRange::Range(423, 424));

        // Verify that the ranges are properly sorted by start line number
        let start_lines: Vec<u32> = author_entry
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
    fn test_prompt_attribution_in_authorship_log() {
        use crate::log_fmt::working_log::{AgentId, Prompt, PromptMessage, PromptRole};

        let entry =
            WorkingLogEntry::new("src/test.rs".to_string(), vec![Line::Range(1, 10)], vec![]);

        let prompt_message = PromptMessage {
            text: "Please add error handling to this function".to_string(),
            role: PromptRole::User,
            username: Some("john.doe".to_string()),
            timestamp: 1234567890,
        };

        let agent_id = AgentId {
            tool: "cursor".to_string(),
            id: "session-abc123".to_string(),
        };

        let prompt = Prompt {
            messages: vec![prompt_message],
            agent_id,
        };

        let mut checkpoint = Checkpoint::new(
            "abc123".to_string(),
            "".to_string(),
            "claude".to_string(),
            vec![entry],
        );
        checkpoint.prompt = Some(prompt);

        let checkpoints = vec![checkpoint];
        let authorship_log = AuthorshipLog::from_working_log(&checkpoints);
        let file_auth = &authorship_log.files["src/test.rs"];

        // Should have one author entry
        assert_eq!(file_auth.authors.len(), 1);
        let author_entry = &file_auth.authors[0];

        // Check that the author is derived from the prompt (now shows tool name)
        // The author should be the tool name (e.g., "cursor"), not the hash
        assert_eq!(author_entry.author, "cursor");

        // Check that the attribution is Agent with a prompt_id
        match &author_entry.attribution {
            Attribution::Agent { prompt_id } => {
                // Verify the prompt exists in the global prompts dictionary
                let prompt = authorship_log.prompts.get(prompt_id).unwrap();
                assert_eq!(prompt.agent_id.tool, "cursor");
                assert_eq!(prompt.agent_id.id, "session-abc123");
                assert_eq!(prompt.messages.len(), 1);
                assert_eq!(
                    prompt.messages[0].text,
                    "Please add error handling to this function"
                );
            }
            Attribution::Human { .. } => panic!("Expected Agent attribution, got Human"),
        }

        // Verify that lines are still attributed correctly
        assert_eq!(file_auth.get_author(5), Some(author_entry.author.as_str()));
    }

    #[test]
    fn test_authorship_log_json_example() {
        use crate::log_fmt::working_log::{AgentId, Prompt, PromptMessage, PromptRole};

        // Create a checkpoint with a prompt
        let prompt = Prompt {
            agent_id: AgentId {
                tool: "cursor".to_string(),
                id: "session-abc123".to_string(),
            },
            messages: vec![
                PromptMessage {
                    text: "Please add error handling to this function".to_string(),
                    role: PromptRole::User,
                    username: Some("john.doe".to_string()),
                    timestamp: 1234567890,
                },
                PromptMessage {
                    text: "I'll add comprehensive error handling with try-catch blocks".to_string(),
                    role: PromptRole::Agent,
                    username: None,
                    timestamp: 1234567891,
                },
            ],
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
            prompt: Some(prompt),
        };

        // Convert to authorship log
        let authorship_log = AuthorshipLog::from_working_log(&[checkpoint]);

        // Serialize to JSON to show the format
        let json = serde_json::to_string_pretty(&authorship_log).unwrap();
        println!("Authorship Log JSON Example:");
        println!("{}", json);

        // Verify the structure
        assert_eq!(authorship_log.files.len(), 1);
        assert_eq!(authorship_log.prompts.len(), 1);
        assert!(authorship_log.files.contains_key("src/main.rs"));

        // Verify prompt is stored in global dictionary
        let prompt_id = authorship_log.prompts.keys().next().unwrap();
        let stored_prompt = authorship_log.prompts.get(prompt_id).unwrap();
        assert_eq!(stored_prompt.agent_id.tool, "cursor");
        assert_eq!(stored_prompt.agent_id.id, "session-abc123");
        assert_eq!(stored_prompt.messages.len(), 2);
    }

    #[test]
    fn test_multiple_prompts_same_tool_separate_entries() {
        use crate::log_fmt::working_log::{AgentId, Prompt, PromptMessage, PromptRole};

        // Create two different prompts from the same tool (cursor)
        let prompt1 = Prompt {
            agent_id: AgentId {
                tool: "cursor".to_string(),
                id: "session-abc123".to_string(),
            },
            messages: vec![PromptMessage {
                text: "Add error handling".to_string(),
                role: PromptRole::User,
                username: Some("john.doe".to_string()),
                timestamp: 1234567890,
            }],
        };

        let prompt2 = Prompt {
            agent_id: AgentId {
                tool: "cursor".to_string(),
                id: "session-xyz789".to_string(),
            },
            messages: vec![PromptMessage {
                text: "Add logging".to_string(),
                role: PromptRole::User,
                username: Some("john.doe".to_string()),
                timestamp: 1234567891,
            }],
        };

        // Create two checkpoints with different prompts
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
            prompt: Some(prompt1),
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
            prompt: Some(prompt2),
        };

        // Convert to authorship log
        let authorship_log = AuthorshipLog::from_working_log(&[checkpoint1, checkpoint2]);

        // Should have two separate author entries for the same tool
        let file_auth = &authorship_log.files["src/main.rs"];
        assert_eq!(file_auth.authors.len(), 2);

        // Both should have author "cursor" but different prompt_ids
        let authors: Vec<&str> = file_auth
            .authors
            .iter()
            .map(|a| a.author.as_str())
            .collect();
        assert_eq!(authors, vec!["cursor", "cursor"]);

        // But they should have different attributions (different prompt_ids)
        let attributions: Vec<&Attribution> =
            file_auth.authors.iter().map(|a| &a.attribution).collect();
        match &attributions[0] {
            Attribution::Agent { prompt_id: id1 } => match &attributions[1] {
                Attribution::Agent { prompt_id: id2 } => {
                    assert_ne!(id1, id2, "Different prompts should have different IDs");
                }
                _ => panic!("Expected Agent attribution"),
            },
            _ => panic!("Expected Agent attribution"),
        }

        // Verify prompts are stored separately
        assert_eq!(authorship_log.prompts.len(), 2);

        // Show the JSON output to demonstrate multiple cursor entries
        let json = serde_json::to_string_pretty(&authorship_log).unwrap();
        println!("Multiple Prompts Same Tool JSON Example:");
        println!("{}", json);
    }

    #[test]
    fn test_mixed_human_and_agent_attribution() {
        use crate::log_fmt::working_log::{AgentId, Prompt, PromptMessage, PromptRole};

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
        let prompt_message = PromptMessage {
            text: "Add error handling".to_string(),
            role: PromptRole::User,
            username: Some("jane.doe".to_string()),
            timestamp: 1234567890,
        };
        let agent_id = AgentId {
            tool: "cursor".to_string(),
            id: "session-xyz789".to_string(),
        };
        let prompt = Prompt {
            messages: vec![prompt_message],
            agent_id,
        };
        let mut ai_checkpoint = Checkpoint::new(
            "def456".to_string(),
            "".to_string(),
            "claude".to_string(),
            vec![ai_entry],
        );
        ai_checkpoint.prompt = Some(prompt);

        let checkpoints = vec![human_checkpoint, ai_checkpoint];
        let authorship_log = AuthorshipLog::from_working_log(&checkpoints);
        let file_auth = &authorship_log.files["src/test.rs"];

        // Should have two author entries
        assert_eq!(file_auth.authors.len(), 2);

        // Find human and agent entries
        let human_entry = file_auth
            .authors
            .iter()
            .find(|a| a.author == "john.doe")
            .unwrap();
        let agent_entry = file_auth
            .authors
            .iter()
            .find(|a| a.author != "john.doe") // Agent entry will have a hash as author
            .unwrap();

        // Check human attribution
        match &human_entry.attribution {
            Attribution::Human { username } => assert_eq!(username, "john.doe"),
            Attribution::Agent { .. } => panic!("Expected Human attribution, got Agent"),
        }

        // Check agent attribution
        match &agent_entry.attribution {
            Attribution::Agent { prompt_id } => {
                // Verify the prompt exists in the global prompts dictionary
                let prompt = authorship_log.prompts.get(prompt_id).unwrap();
                assert_eq!(prompt.agent_id.tool, "cursor");
                assert_eq!(prompt.agent_id.id, "session-xyz789");
            }
            Attribution::Human { .. } => panic!("Expected Agent attribution, got Human"),
        }

        // Verify line attributions
        assert_eq!(file_auth.get_author(3), Some("john.doe")); // Human lines
        assert_eq!(file_auth.get_author(8), Some(agent_entry.author.as_str())); // Agent lines
    }
}
