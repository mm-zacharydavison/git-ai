//! Attribution tracking through file changes
//!
//! This library maintains attribution ranges as files are edited, preserving
//! authorship information even through moves, edits, and whitespace changes.

use diff_match_patch_rs::dmp::Diff;
use crate::error::GitAiError;
use diff_match_patch_rs::{Compat, DiffMatchPatch, Ops};
use std::collections::HashMap;
use crate::authorship::working_log::CheckpointKind;

/// Represents a single attribution range in the file.
/// Ranges can overlap (multiple authors can be attributed to the same text).
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Attribution {
    /// Character position where this attribution starts (inclusive)
    pub start: usize,
    /// Character position where this attribution ends (exclusive)
    pub end: usize,
    /// Identifier for the author of this range
    pub author_id: String,
    /// Timestamp of the attribution (in milliseconds since epoch)
    pub ts: u128,
}

/// Represents attribution for a range of lines.
/// Both start_line and end_line are inclusive (1-indexed).
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct LineAttribution {
    /// Line number where this attribution starts (inclusive, 1-indexed)
    pub start_line: u32,
    /// Line number where this attribution ends (inclusive, 1-indexed)
    pub end_line: u32,
    /// Identifier for the author of this range
    pub author_id: String,
}

impl LineAttribution {
    pub fn new(start_line: u32, end_line: u32, author_id: String) -> Self {
        LineAttribution {
            start_line,
            end_line,
            author_id,
        }
    }

    /// Returns the number of lines this attribution covers
    pub fn line_count(&self) -> u32 {
        if self.start_line > self.end_line {
            0
        } else {
            self.end_line - self.start_line + 1
        }
    }

    /// Checks if this line attribution is empty
    pub fn is_empty(&self) -> bool {
        self.start_line > self.end_line
    }

    /// Checks if this attribution overlaps with a given line range (inclusive)
    pub fn overlaps(&self, start_line: u32, end_line: u32) -> bool {
        self.start_line <= end_line && self.end_line >= start_line
    }

    /// Returns the overlapping portion of this attribution with a given line range
    pub fn intersection(&self, start_line: u32, end_line: u32) -> Option<(u32, u32)> {
        let overlap_start = self.start_line.max(start_line);
        let overlap_end = self.end_line.min(end_line);

        if overlap_start <= overlap_end {
            Some((overlap_start, overlap_end))
        } else {
            None
        }
    }
}

impl Attribution {
    pub fn new(start: usize, end: usize, author_id: String, ts: u128) -> Self {
        Attribution {
            start,
            end,
            author_id,
            ts,
        }
    }

    /// Returns the length of this attribution range
    pub fn len(&self) -> usize {
        self.end - self.start
    }

    /// Checks if this attribution is empty
    pub fn is_empty(&self) -> bool {
        self.start >= self.end
    }

    /// Checks if this attribution overlaps with a given range
    pub fn overlaps(&self, start: usize, end: usize) -> bool {
        self.start < end && self.end > start
    }

    /// Returns the overlapping portion of this attribution with a given range
    pub fn intersection(&self, start: usize, end: usize) -> Option<(usize, usize)> {
        let overlap_start = self.start.max(start);
        let overlap_end = self.end.min(end);

        if overlap_start < overlap_end {
            Some((overlap_start, overlap_end))
        } else {
            None
        }
    }
}

/// Represents a deletion operation from the diff
#[derive(Debug, Clone)]
struct Deletion {
    /// Start position in old content
    start: usize,
    /// End position in old content
    end: usize,
    /// The deleted text
    text: String,
}

/// Represents an insertion operation from the diff
#[derive(Debug, Clone)]
struct Insertion {
    /// Start position in new content
    start: usize,
    /// End position in new content
    end: usize,
    /// The inserted text
    text: String,
}

/// Information about a detected move operation
#[derive(Debug, Clone)]
struct MoveMapping {
    /// The deletion that was moved
    deletion_idx: usize,
    /// The insertion where it was moved to
    insertion_idx: usize,
    /// Similarity score (0.0 to 1.0)
    similarity: f64,
    /// Where the deleted text appears in the insertion (offset, length)
    alignment: (usize, usize),
}

/// Configuration for the attribution tracker
pub struct AttributionConfig {
    /// Minimum similarity threshold for detecting moves (0.0 to 1.0)
    pub move_threshold: f64,
    /// Minimum text length to consider for move detection
    pub min_move_length: usize,
}

impl Default for AttributionConfig {
    fn default() -> Self {
        AttributionConfig {
            move_threshold: 0.8,
            min_move_length: 10,
        }
    }
}

/// Main attribution tracker
pub struct AttributionTracker {
    config: AttributionConfig,
    dmp: DiffMatchPatch,
}

impl AttributionTracker {
    /// Create a new attribution tracker with default configuration
    pub fn new() -> Self {
        AttributionTracker {
            config: AttributionConfig::default(),
            dmp: DiffMatchPatch::new(),
        }
    }

    /// Create a new attribution tracker with custom configuration
    pub fn with_config(config: AttributionConfig) -> Self {
        AttributionTracker {
            config,
            dmp: DiffMatchPatch::new(),
        }
    }

    /// Attribute all unattributed ranges to the given author
    pub fn attribute_unattributed_ranges(&self, content: &str, prev_attributions: &[Attribution], author: &str, ts: u128) -> Vec<Attribution> {
        let mut attributions = prev_attributions.to_vec();
        let mut unattributed_char_idxs = Vec::new();
        
        // Find all unattributed character positions
        for i in 0..content.len() {
            if !attributions.iter().any(|a| a.overlaps(i, i + 1)) {
                unattributed_char_idxs.push(i);
            }
        }
        
        // Sort the unattributed character indices by position
        unattributed_char_idxs.sort();
        
        // Group contiguous unattributed ranges
        let mut contiguous_ranges = Vec::new();
        if !unattributed_char_idxs.is_empty() {
            let mut start = unattributed_char_idxs[0];
            let mut end = start + 1;
            
            for i in 1..unattributed_char_idxs.len() {
                let current = unattributed_char_idxs[i];
                if current == end {
                    // Contiguous with previous range
                    end = current + 1;
                } else {
                    // Gap found, save current range and start new one
                    contiguous_ranges.push((start, end));
                    start = current;
                    end = current + 1;
                }
            }
            // Don't forget the last range
            contiguous_ranges.push((start, end));
        }
        
        // Create attributions for each contiguous unattributed range
        for (start, end) in contiguous_ranges {
            attributions.push(Attribution::new(start, end, author.to_string(), ts));
        }
        
        attributions
    }

    /// Update attributions from old content to new content
    ///
    /// # Arguments
    /// * `old_content` - The previous version of the file
    /// * `new_content` - The new version of the file
    /// * `old_attributions` - Attributions from the previous version
    /// * `current_author` - Author ID to use for new changes
    ///
    /// # Returns
    /// A vector of updated attributions for the new content
    pub fn update_attributions(
        &self,
        old_content: &str,
        new_content: &str,
        old_attributions: &[Attribution],
        current_author: &str,
        ts: u128,
    ) -> Result<Vec<Attribution>, GitAiError> {
        // Phase 1: Compute diff
        let diffs = self
            .dmp
            .diff_main::<Compat>(old_content, new_content)
            .map_err(|e| GitAiError::Generic(format!("Diff computation failed: {:?}", e)))?;

        // Phase 2: Build deletion and insertion catalogs
        let (deletions, insertions) = self.build_diff_catalog(&diffs);

        // Phase 3: Detect move operations
        let move_mappings = self.detect_moves(&deletions, &insertions);

        // Phase 4: Transform attributions through the diff
        let new_attributions = self.transform_attributions(
            &diffs,
            old_content,
            old_attributions,
            current_author,
            &deletions,
            &insertions,
            &move_mappings,
            ts,
        );

        // Phase 5: Merge and clean up
        Ok(self.merge_attributions(new_attributions))
    }

    /// Build catalogs of deletions and insertions from the diff
    fn build_diff_catalog(&self, diffs: &[Diff<char>]) -> (Vec<Deletion>, Vec<Insertion>) {
        let mut deletions = Vec::new();
        let mut insertions = Vec::new();

        let mut old_pos = 0;
        let mut new_pos = 0;

        for diff in diffs {
            let op = diff.op();
            let text_chars = diff.data();
            let text: String = text_chars.iter().collect();
            let len = text_chars.len();

            match op {
                Ops::Equal => {
                    old_pos += len;
                    new_pos += len;
                }
                Ops::Delete => {
                    deletions.push(Deletion {
                        start: old_pos,
                        end: old_pos + len,
                        text,
                    });
                    old_pos += len;
                }
                Ops::Insert => {
                    insertions.push(Insertion {
                        start: new_pos,
                        end: new_pos + len,
                        text,
                    });
                    new_pos += len;
                }
            }
        }

        (deletions, insertions)
    }

    /// Detect move operations between deletions and insertions
    fn detect_moves(&self, deletions: &[Deletion], insertions: &[Insertion]) -> Vec<MoveMapping> {
        let mut move_mappings = Vec::new();
        let mut used_insertions = std::collections::HashSet::new();

        // Process deletions from largest to smallest for better matching
        let mut deletion_indices: Vec<usize> = (0..deletions.len()).collect();
        deletion_indices.sort_by_key(|&i| std::cmp::Reverse(deletions[i].text.len()));

        for &del_idx in &deletion_indices {
            let deletion = &deletions[del_idx];

            // Skip small deletions
            if deletion.text.len() < self.config.min_move_length {
                continue;
            }

            let mut best_match: Option<(usize, f64, (usize, usize))> = None;

            for (ins_idx, insertion) in insertions.iter().enumerate() {
                if used_insertions.contains(&ins_idx) {
                    continue;
                }

                // Compute similarity
                let similarity = self.compute_similarity(&deletion.text, &insertion.text);

                if similarity >= self.config.move_threshold {
                    // Find alignment
                    let alignment = self.find_alignment(&deletion.text, &insertion.text);

                    match best_match {
                        Some((_, best_sim, _)) => {
                            if similarity > best_sim {
                                best_match = Some((ins_idx, similarity, alignment));
                            }
                        }
                        None => {
                            best_match = Some((ins_idx, similarity, alignment));
                        }
                    }
                }
            }

            if let Some((ins_idx, similarity, alignment)) = best_match {
                move_mappings.push(MoveMapping {
                    deletion_idx: del_idx,
                    insertion_idx: ins_idx,
                    similarity,
                    alignment,
                });
                used_insertions.insert(ins_idx);
            }
        }

        move_mappings
    }

    /// Compute similarity between two text strings
    fn compute_similarity(&self, text1: &str, text2: &str) -> f64 {
        if text1 == text2 {
            return 1.0;
        }

        if text1.is_empty() || text2.is_empty() {
            return 0.0;
        }

        // Use Jaro-Winkler similarity from strsim
        strsim::jaro_winkler(text1, text2)
    }

    /// Find where text1 best aligns within text2
    /// Returns (offset in text2, length of match)
    fn find_alignment(&self, text1: &str, text2: &str) -> (usize, usize) {
        // If exact match, check if text1 is a substring of text2
        if let Some(pos) = text2.find(text1) {
            return (pos, text1.len());
        }

        // If text2 contains text1, return that
        if text2.contains(text1) {
            return (text2.find(text1).unwrap(), text1.len());
        }

        // For approximate matches, assume the entire insertion corresponds
        // This is a simplification - could be improved with fuzzy matching
        (0, text2.len())
    }

    /// Transform attributions through the diff
    fn transform_attributions(
        &self,
        diffs: &[Diff<char>],
        old_content: &str,
        old_attributions: &[Attribution],
        current_author: &str,
        deletions: &[Deletion],
        insertions: &[Insertion],
        move_mappings: &[MoveMapping],
        ts: u128,
    ) -> Vec<Attribution> {
        let mut new_attributions = Vec::new();

        // Build lookup maps for moves
        let mut deletion_to_move: HashMap<usize, &MoveMapping> = HashMap::new();
        let mut insertion_from_move: std::collections::HashSet<usize> =
            std::collections::HashSet::new();

        for mapping in move_mappings {
            deletion_to_move.insert(mapping.deletion_idx, mapping);
            insertion_from_move.insert(mapping.insertion_idx);
        }


        let mut old_pos = 0;
        let mut new_pos = 0;
        let mut deletion_idx = 0;
        let mut insertion_idx = 0;

        for diff in diffs {
            let op = diff.op();
            let text_chars = diff.data();
            let len = text_chars.len();

            match op {
                Ops::Equal => {
                    // Unchanged text: transform attributions directly
                    let old_range = (old_pos, old_pos + len);
                    let new_range = (new_pos, new_pos + len);

                    for attr in old_attributions {
                        if let Some((overlap_start, overlap_end)) =
                            attr.intersection(old_range.0, old_range.1)
                        {
                            // Transform to new position
                            let offset_in_range = overlap_start - old_range.0;
                            let overlap_len = overlap_end - overlap_start;

                            new_attributions.push(Attribution::new(
                                new_range.0 + offset_in_range,
                                new_range.0 + offset_in_range + overlap_len,
                                attr.author_id.clone(),
                                attr.ts.clone(),
                            ));
                        }
                    }

                    old_pos += len;
                    new_pos += len;
                }
                Ops::Delete => {
                    let deletion_range = (old_pos, old_pos + len);

                    // Check if this deletion is part of a move
                    if let Some(mapping) = deletion_to_move.get(&deletion_idx) {
                        // This text was moved - transform attributions to new location
                        let insertion = &insertions[mapping.insertion_idx];
                        let (align_offset, align_len) = mapping.alignment;

                        for attr in old_attributions {
                            if let Some((overlap_start, overlap_end)) =
                                attr.intersection(deletion_range.0, deletion_range.1)
                            {
                                // Map to new location
                                let offset_in_deletion = overlap_start - deletion_range.0;
                                let overlap_len = overlap_end - overlap_start;

                                // Simple mapping: assumes linear correspondence
                                let scale = if deletions[deletion_idx].text.len() > 0 {
                                    align_len as f64 / deletions[deletion_idx].text.len() as f64
                                } else {
                                    1.0
                                };

                                let new_start = insertion.start + align_offset + (offset_in_deletion as f64 * scale) as usize;
                                let new_len = (overlap_len as f64 * scale) as usize;

                                new_attributions.push(Attribution::new(
                                    new_start,
                                    new_start + new_len,
                                    attr.author_id.clone(),
                                    attr.ts.clone(),
                                ));
                            }
                        }
                    }
                    // else: True deletion - attributions are lost

                    old_pos += len;
                    deletion_idx += 1;
                }
                Ops::Insert => {
                    // Check if this insertion is from a detected move
                    if insertion_from_move.contains(&insertion_idx) {
                        // Already handled in Delete phase
                        new_pos += len;
                        insertion_idx += 1;
                        continue;
                    }

                    // Check if this "insertion" actually exists elsewhere in the old content
                    // This handles cases where content is in the same file but the diff algorithm
                    // didn't treat it as EQUAL (e.g., content between a cut and paste operation)
                    let insertion_text: String = text_chars.iter().collect();
                    let mut attributed = false;

                    // Try to find the longest prefix of this insertion that exists in the old content
                    // Start with the full text and work backwards
                    let mut match_info: Option<(usize, usize)> = None;  // (old_pos, match_len)

                    // TODO Figure out (a) if we need this here and (b) if we do, then what the threshold should be
                    for search_len in (100..=insertion_text.len()).rev() {
                        let search_text = &insertion_text[..search_len];
                        if let Some(old_match_pos) = old_content[old_pos..].find(search_text) {
                            match_info = Some((old_pos + old_match_pos, search_len));
                            break;
                        }
                    }

                    if let Some((absolute_old_pos, match_len)) = match_info {
                        // This text existed before, preserve its attributions
                        for attr in old_attributions {
                            if let Some((overlap_start, overlap_end)) =
                                attr.intersection(absolute_old_pos, absolute_old_pos + match_len)
                            {
                                let offset_in_range = overlap_start - absolute_old_pos;
                                let overlap_len = overlap_end - overlap_start;

                                new_attributions.push(Attribution::new(
                                    new_pos + offset_in_range,
                                    new_pos + offset_in_range + overlap_len,
                                    attr.author_id.clone(),
                                    ts, // TODO: Double check if we should update the timestamp on move attributions?
                                ));
                                attributed = true;
                            }
                        }

                        // Any remaining part gets attributed to current author
                        if match_len < len {
                            new_attributions.push(Attribution::new(
                                new_pos + match_len,
                                new_pos + len,
                                current_author.to_string(),
                                ts,
                            ));
                            attributed = true;
                        }
                    }

                    // If we couldn't find a match or no attributions, attribute to current author
                    if !attributed {
                        new_attributions.push(Attribution::new(
                            new_pos,
                            new_pos + len,
                            current_author.to_string(),
                            ts,
                        ));
                    }

                    new_pos += len;
                    insertion_idx += 1;
                }
            }
        }

        new_attributions
    }

    /// Merge and clean up attributions
    fn merge_attributions(&self, mut attributions: Vec<Attribution>) -> Vec<Attribution> {
        if attributions.is_empty() {
            return attributions;
        }

        // Sort by start position
        attributions.sort_by_key(|a| (a.start, a.end, a.author_id.clone()));

        // Remove exact duplicates
        attributions.dedup();

        attributions
    }
}

impl Default for AttributionTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper struct to track line boundaries in content
struct LineBoundaries {
    /// Maps line number (1-indexed) to (start_char, end_char) exclusive end
    line_ranges: Vec<(usize, usize)>,
}

impl LineBoundaries {
    fn new(content: &str) -> Self {
        let mut line_ranges = Vec::new();
        let mut start = 0;

        for (idx, _) in content.match_indices('\n') {
            // Line from start to idx (inclusive of newline)
            line_ranges.push((start, idx + 1));
            start = idx + 1;
        }

        // Handle last line if it doesn't end with newline
        if start < content.len() {
            line_ranges.push((start, content.len()));
        } else if start == content.len() && content.is_empty() {
            // Empty file - no lines
        } else if start == content.len() && !content.is_empty() {
            // File ends with newline, last line is already added
        }

        LineBoundaries { line_ranges }
    }

    fn line_count(&self) -> u32 {
        self.line_ranges.len() as u32
    }

    fn get_line_range(&self, line_num: u32) -> Option<(usize, usize)> {
        if line_num < 1 || line_num as usize > self.line_ranges.len() {
            None
        } else {
            Some(self.line_ranges[line_num as usize - 1])
        }
    }
}

/// Convert line-based attributions to character-based attributions.
///
/// # Arguments
/// * `line_attributions` - Line-based attributions to convert
/// * `content` - The file content to map line numbers to character positions
///
/// # Returns
/// A vector of character-based attributions covering the same ranges
pub fn line_attributions_to_attributions(
    line_attributions: &Vec<LineAttribution>,
    content: &str,
    ts: u128,
) -> Vec<Attribution> {
    if line_attributions.is_empty() || content.is_empty() {
        return Vec::new();
    }

    let boundaries = LineBoundaries::new(content);
    let mut result = Vec::new();

    for line_attr in line_attributions {
        // Get character ranges for start and end lines
        let start_range = boundaries.get_line_range(line_attr.start_line);
        let end_range = boundaries.get_line_range(line_attr.end_line);

        if let (Some((start_char, _)), Some((_, end_char))) = (start_range, end_range) {
            result.push(Attribution::new(
                start_char,
                end_char,
                line_attr.author_id.clone(),
                ts,
            ));
        }
    }

    result
}

/// Convert character-based attributions to line-based attributions.
/// For each line, selects the "dominant" author based on who contributed
/// the most non-whitespace characters to that line.
///
/// # Arguments
/// * `attributions` - Character-based attributions
/// * `content` - The file content being attributed
///
/// # Returns
/// A vector of line attributions with consecutive lines by the same author merged
pub fn attributions_to_line_attributions(
    attributions: &Vec<Attribution>,
    content: &str,
) -> Vec<LineAttribution> {
    if content.is_empty() || attributions.is_empty() {
        return Vec::new();
    }

    let boundaries = LineBoundaries::new(content);
    let line_count = boundaries.line_count();

    if line_count == 0 {
        return Vec::new();
    }

    // For each line, determine the dominant author
    let mut line_authors: Vec<Option<String>> = Vec::with_capacity(line_count as usize);

    for line_num in 1..=line_count {
        let author = find_dominant_author_for_line(
            line_num,
            &boundaries,
            attributions,
            content,
        );
        line_authors.push(Some(author));
    }

    // Merge consecutive lines with the same author
    let mut merged_line_authors = merge_consecutive_line_attributions(line_authors);

    // Strip away all human lines (only AI lines need to be retained)
    merged_line_authors.retain(|line_attr| line_attr.author_id != CheckpointKind::Human.to_str());
    merged_line_authors
}

/// Find the dominant author for a specific line based on non-whitespace character count
fn find_dominant_author_for_line(
    line_num: u32,
    boundaries: &LineBoundaries,
    attributions: &Vec<Attribution>,
    full_content: &str,
) -> String {
    let (line_start, line_end) = boundaries.get_line_range(line_num).unwrap();

    let mut candidate_attrs = Vec::new();
    for attribution in attributions {
        if !attribution.overlaps(line_start, line_end) {
            continue;
        }

        // Get the substring of the content on this line that is covered by the attribution
        let content_slice = &full_content[std::cmp::max(line_start, attribution.start)..std::cmp::min(line_end, attribution.end)];
        let non_whitespace_count = content_slice.chars().filter(|c| !c.is_whitespace()).count();
        if non_whitespace_count > 0 {
            candidate_attrs.push(attribution.clone());
        } else {
            // If the attribution is only whitespace, discard it
            continue;
        }
    }

    if candidate_attrs.is_empty() {
        return CheckpointKind::Human.to_str();
    }
    
    // Choose the author with the latest timestamp
    let latest_timestamp = candidate_attrs.iter().max_by_key(|a| a.ts).unwrap().ts;
    let latest_author = candidate_attrs.iter().filter(|a| a.ts == latest_timestamp).map(|a| a.author_id.clone()).collect::<Vec<String>>();
    return latest_author[0].clone();
}

/// Merge consecutive lines with the same author into LineAttribution ranges
fn merge_consecutive_line_attributions(line_authors: Vec<Option<String>>) -> Vec<LineAttribution> {
    let mut result = Vec::new();
    let line_count = line_authors.len();

    let mut current_author: Option<String> = None;
    let mut current_start: u32 = 0;

    for (idx, author) in line_authors.into_iter().enumerate() {
        let line_num = (idx + 1) as u32;

        match (&current_author, author) {
            (None, None) => {
                // No attribution for this line, continue
            }
            (None, Some(new_author)) => {
                // Start a new attribution
                current_author = Some(new_author);
                current_start = line_num;
            }
            (Some(_), None) => {
                // End current attribution
                if let Some(author) = current_author.take() {
                    result.push(LineAttribution::new(
                        current_start,
                        line_num - 1,
                        author,
                    ));
                }
            }
            (Some(curr), Some(new_author)) => {
                if curr == &new_author {
                    // Continue current attribution
                } else {
                    // End current, start new
                    result.push(LineAttribution::new(
                        current_start,
                        line_num - 1,
                        curr.clone(),
                    ));
                    current_author = Some(new_author);
                    current_start = line_num;
                }
            }
        }
    }

    // Close final attribution if any
    if let Some(author) = current_author {
        result.push(LineAttribution::new(
            current_start,
            line_count as u32,
            author,
        ));
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    
    // Test timestamp constant for consistent testing
    const TEST_TS: u128 = 1234567890000;

    #[test]
    fn test_simple_insertion() {
        let tracker = AttributionTracker::new();

        let old_content = "Hello world";
        let new_content = "Hello beautiful world";

        let old_attributions = vec![Attribution::new(0, 11, "Alice".to_string(), TEST_TS)];

        let new_attributions = tracker
            .update_attributions(old_content, new_content, &old_attributions, "Bob", TEST_TS)
            .unwrap();

        // Should have:
        // - "Hello " attributed to Alice
        // - "beautiful " attributed to Bob
        // - "world" attributed to Alice

        assert!(new_attributions.len() >= 3);

        // Check that "beautiful " is attributed to Bob
        let bob_attrs: Vec<_> = new_attributions
            .iter()
            .filter(|a| a.author_id == "Bob")
            .collect();
        assert!(!bob_attrs.is_empty());
    }

    #[test]
    fn test_simple_deletion() {
        let tracker = AttributionTracker::new();

        let old_content = "Hello beautiful world";
        let new_content = "Hello world";

        let old_attributions = vec![
            Attribution::new(0, 6, "Alice".to_string(), TEST_TS),
            Attribution::new(6, 16, "Bob".to_string(), TEST_TS),
            Attribution::new(16, 21, "Alice".to_string(), TEST_TS),
        ];

        let new_attributions = tracker
            .update_attributions(old_content, new_content, &old_attributions, "Charlie", TEST_TS)
            .unwrap();

        // Bob's attribution should be gone
        let bob_attrs: Vec<_> = new_attributions
            .iter()
            .filter(|a| a.author_id == "Bob")
            .collect();
        assert!(bob_attrs.is_empty());

        // Alice's attributions should remain
        let alice_attrs: Vec<_> = new_attributions
            .iter()
            .filter(|a| a.author_id == "Alice")
            .collect();
        assert!(!alice_attrs.is_empty());
    }

    #[test]
    fn test_cut_and_paste() {
        let tracker = AttributionTracker::new();

        // Original: function at the top
        let old_content = "fn helper() {\n  println!(\"helper\");\n}\n\nfn main() {\n  println!(\"main\");\n}";

        // New: function moved to bottom
        let new_content = "fn main() {\n  println!(\"main\");\n}\n\nfn helper() {\n  println!(\"helper\");\n}";

        // Attribute the helper function to Alice
        let old_attributions = vec![
            Attribution::new(0, 34, "Alice".to_string(), TEST_TS), // fn helper() { ... }
            Attribution::new(36, 70, "Bob".to_string(), TEST_TS),   // fn main() { ... }
        ];

        let new_attributions = tracker
            .update_attributions(old_content, new_content, &old_attributions, "Charlie", TEST_TS)
            .unwrap();

        // Alice's attribution should move with the helper function
        // Bob's attribution should stay with the main function
        let alice_attrs: Vec<_> = new_attributions
            .iter()
            .filter(|a| a.author_id == "Alice")
            .collect();

        let bob_attrs: Vec<_> = new_attributions
            .iter()
            .filter(|a| a.author_id == "Bob")
            .collect();

        // Both authors should still have attributions
        assert!(!alice_attrs.is_empty(), "Alice's attribution should be preserved through move");
        assert!(!bob_attrs.is_empty(), "Bob's attribution should be preserved");
    }

    #[test]
    fn test_indentation_change() {
        let tracker = AttributionTracker::new();

        let old_content = "fn test() {\n  code();\n}";
        let new_content = "fn test() {\n    code();\n}"; // Changed from 2 to 4 space indent

        let old_attributions = vec![Attribution::new(0, 23, "Alice".to_string(), TEST_TS)];

        let new_attributions = tracker
            .update_attributions(old_content, new_content, &old_attributions, "Bob", TEST_TS)
            .unwrap();

        // Alice should still be attributed to most of the code
        let alice_attrs: Vec<_> = new_attributions
            .iter()
            .filter(|a| a.author_id == "Alice")
            .collect();
        assert!(!alice_attrs.is_empty());

        // Bob should only be attributed to the extra spaces
        let bob_attrs: Vec<_> = new_attributions
            .iter()
            .filter(|a| a.author_id == "Bob")
            .collect();

        // Bob gets attribution for the new whitespace
        assert!(!bob_attrs.is_empty());
    }

    #[test]
    fn test_overlapping_attributions() {
        let tracker = AttributionTracker::new();

        let old_content = "Hello world";
        let new_content = "Hello beautiful world";

        // Overlapping attributions: Alice owns 0-11, Bob owns 0-5
        let old_attributions = vec![
            Attribution::new(0, 11, "Alice".to_string(), TEST_TS),
            Attribution::new(0, 5, "Bob".to_string(), TEST_TS),
        ];

        let new_attributions = tracker
            .update_attributions(old_content, new_content, &old_attributions, "Charlie", TEST_TS)
            .unwrap();

        // Both Alice and Bob should have overlapping attributions preserved
        let alice_attrs: Vec<_> = new_attributions
            .iter()
            .filter(|a| a.author_id == "Alice")
            .collect();

        let bob_attrs: Vec<_> = new_attributions
            .iter()
            .filter(|a| a.author_id == "Bob")
            .collect();

        assert!(!alice_attrs.is_empty());
        assert!(!bob_attrs.is_empty());
    }

    #[test]
    fn test_replacement() {
        let tracker = AttributionTracker::new();

        let old_content = "The quick brown fox";
        let new_content = "The slow brown fox";

        let old_attributions = vec![Attribution::new(0, 19, "Alice".to_string(), TEST_TS)];

        let new_attributions = tracker
            .update_attributions(old_content, new_content, &old_attributions, "Bob", TEST_TS)
            .unwrap();

        // "The " should be Alice
        // "slow" should be Bob
        // " brown fox" should be Alice

        let bob_attrs: Vec<_> = new_attributions
            .iter()
            .filter(|a| a.author_id == "Bob")
            .collect();
        assert!(!bob_attrs.is_empty(), "Bob should be attributed for the replacement");

        let alice_attrs: Vec<_> = new_attributions
            .iter()
            .filter(|a| a.author_id == "Alice")
            .collect();
        assert!(!alice_attrs.is_empty(), "Alice should retain attribution for unchanged parts");
    }

    #[test]
    fn test_empty_file() {
        let tracker = AttributionTracker::new();

        let old_content = "";
        let new_content = "Hello world";

        let old_attributions = vec![];

        let new_attributions = tracker
            .update_attributions(old_content, new_content, &old_attributions, "Alice", TEST_TS)
            .unwrap();

        assert_eq!(new_attributions.len(), 1);
        assert_eq!(new_attributions[0].author_id, "Alice");
        assert_eq!(new_attributions[0].start, 0);
        assert_eq!(new_attributions[0].end, 11);
    }

    #[test]
    fn test_no_changes() {
        let tracker = AttributionTracker::new();

        let old_content = "Hello world";
        let new_content = "Hello world";

        let old_attributions = vec![Attribution::new(0, 11, "Alice".to_string(), TEST_TS)];

        let new_attributions = tracker
            .update_attributions(old_content, new_content, &old_attributions, "Bob", TEST_TS)
            .unwrap();

        assert_eq!(new_attributions.len(), 1);
        assert_eq!(new_attributions[0].author_id, "Alice");
        assert_eq!(new_attributions[0].start, 0);
        assert_eq!(new_attributions[0].end, 11);
    }

    #[test]
    fn test_attribution_overlap() {
        let attr = Attribution::new(10, 20, "Alice".to_string(), TEST_TS);

        assert!(attr.overlaps(15, 25));
        assert!(attr.overlaps(5, 15));
        assert!(attr.overlaps(5, 25));
        assert!(attr.overlaps(12, 18));
        assert!(!attr.overlaps(0, 10));
        assert!(!attr.overlaps(20, 30));
    }

    #[test]
    fn test_attribution_intersection() {
        let attr = Attribution::new(10, 20, "Alice".to_string(), TEST_TS);

        assert_eq!(attr.intersection(15, 25), Some((15, 20)));
        assert_eq!(attr.intersection(5, 15), Some((10, 15)));
        assert_eq!(attr.intersection(5, 25), Some((10, 20)));
        assert_eq!(attr.intersection(12, 18), Some((12, 18)));
        assert_eq!(attr.intersection(0, 10), None);
        assert_eq!(attr.intersection(20, 30), None);
    }

    #[test]
    fn test_multiline_text_with_move() {
        let tracker = AttributionTracker::new();

        let old_content = r#"// Header
fn foo() {
    bar();
}

fn main() {
    foo();
}"#;

        let new_content = r#"// Header
fn main() {
    foo();
}

fn foo() {
    bar();
}"#;

        // Attribute different functions to different authors
        let old_attributions = vec![
            Attribution::new(0, 10, "Alice".to_string(), TEST_TS),  // // Header
            Attribution::new(10, 34, "Bob".to_string(), TEST_TS),   // fn foo() { bar(); }
            Attribution::new(35, 63, "Charlie".to_string(), TEST_TS), // fn main() { foo(); }
        ];

        let new_attributions = tracker
            .update_attributions(old_content, new_content, &old_attributions, "Dave", TEST_TS)
            .unwrap();

        // All three authors should still have attributions
        let alice_count = new_attributions.iter().filter(|a| a.author_id == "Alice").count();
        let bob_count = new_attributions.iter().filter(|a| a.author_id == "Bob").count();
        let charlie_count = new_attributions.iter().filter(|a| a.author_id == "Charlie").count();

        assert!(alice_count > 0, "Alice's header attribution should be preserved");
        assert!(bob_count > 0, "Bob's foo function attribution should move");
        assert!(charlie_count > 0, "Charlie's main function attribution should move");
    }

    #[test]
    fn test_newline_insertion() {
        let tracker = AttributionTracker::new();

        // A creates two lines
        let v1_content = "console.log(\"A- HELLO\")\n";
        let v1_attributions = vec![Attribution::new(0, 24, "A".to_string(), TEST_TS)];

        // B adds a line
        let v2_content = "console.log(\"A- HELLO\")\nconsole.log(\"B- HELLO\")\n";
        let v2_attributions = tracker
            .update_attributions(v1_content, v2_content, &v1_attributions, "B", TEST_TS)
            .unwrap();

        // A adds three empty lines between B's line and the next line
        let v3_content = "console.log(\"A- HELLO\")\nconsole.log(\"B- HELLO\")\n\n\n\n";
        let v3_attributions = tracker
            .update_attributions(v2_content, v3_content, &v2_attributions, "A", TEST_TS)
            .unwrap();

        // C adds a line
        let v4_content = "console.log(\"A- HELLO\")\nconsole.log(\"B- HELLO\")\n\n\n\nconsole.log(\"C- HELLO\")";
        let v4_attributions = tracker
            .update_attributions(v3_content, v4_content, &v3_attributions, "C", TEST_TS)
            .unwrap();

        // Verify attributions
        // Line 1 (0-23) + newline (23) = A
        // Line 2 (24-47) + newline (47) = B
        // Empty line newline (48) = A
        // Empty line newline (49) = A
        // Empty line newline (50) = A
        // Line 6 (51-73) = C

        let a_attrs: Vec<_> = v4_attributions.iter().filter(|a| a.author_id == "A").collect();
        let b_attrs: Vec<_> = v4_attributions.iter().filter(|a| a.author_id == "B").collect();
        let c_attrs: Vec<_> = v4_attributions.iter().filter(|a| a.author_id == "C").collect();

        // A should have the first line + its newline, and the three empty line newlines
        // That's char 0-24 and chars 48-51 (3 newlines)
        let a_total: usize = a_attrs.iter().map(|a| a.len()).sum();
        assert_eq!(a_total, 24 + 3, "A should have 24 chars from first line + 3 newlines");

        // B should have the second line + its newline = 24 chars
        let b_total: usize = b_attrs.iter().map(|a| a.len()).sum();
        assert_eq!(b_total, 24, "B should have 24 chars");

        // C should have the last line (no trailing newline) = 23 chars
        let c_total: usize = c_attrs.iter().map(|a| a.len()).sum();
        assert_eq!(c_total, 23, "C should have 23 chars");

        // Check that the three newlines (chars 48-51) are all attributed to A
        for pos in 48..51 {
            let attributed_to_a = a_attrs.iter().any(|a| a.start <= pos && a.end > pos);
            assert!(attributed_to_a, "Character at position {} should be attributed to A", pos);
        }

        // Ensure C doesn't have any attribution in the 47-51 range (the newlines)
        for attr in &c_attrs {
            assert!(attr.start >= 51 || attr.end <= 47,
                "C should not have attribution in the newline range 47-51, but has {:?}", attr);
        }
    }

    // ========== Line to Character Attribution Conversion Tests ==========

    #[test]
    fn test_line_to_char_attribution_single_range() {
        let content = "line 1\nline 2\nline 3\n";
        let line_attrs = vec![LineAttribution::new(1, 3, "Alice".to_string())];

        let char_attrs = line_attributions_to_attributions(&line_attrs, content, TEST_TS);

        assert_eq!(char_attrs.len(), 1);
        assert_eq!(char_attrs[0].start, 0);
        assert_eq!(char_attrs[0].end, 21); // entire content
        assert_eq!(char_attrs[0].author_id, "Alice");
    }

    #[test]
    fn test_line_to_char_attribution_multiple_ranges() {
        let content = "line 1\nline 2\nline 3\nline 4\n";
        let line_attrs = vec![
            LineAttribution::new(1, 2, "Alice".to_string()),
            LineAttribution::new(3, 4, "Bob".to_string()),
        ];

        let char_attrs = line_attributions_to_attributions(&line_attrs, content, TEST_TS);

        assert_eq!(char_attrs.len(), 2);
        assert_eq!(char_attrs[0].start, 0);
        assert_eq!(char_attrs[0].end, 14); // lines 1-2
        assert_eq!(char_attrs[0].author_id, "Alice");
        assert_eq!(char_attrs[1].start, 14);
        assert_eq!(char_attrs[1].end, 28); // lines 3-4
        assert_eq!(char_attrs[1].author_id, "Bob");
    }

    #[test]
    fn test_line_to_char_attribution_single_line() {
        let content = "line 1\nline 2\nline 3\n";
        let line_attrs = vec![LineAttribution::new(2, 2, "Bob".to_string())];

        let char_attrs = line_attributions_to_attributions(&line_attrs, content, TEST_TS);

        assert_eq!(char_attrs.len(), 1);
        assert_eq!(char_attrs[0].start, 7);
        assert_eq!(char_attrs[0].end, 14); // just line 2
        assert_eq!(char_attrs[0].author_id, "Bob");
    }

    #[test]
    fn test_line_to_char_attribution_no_trailing_newline() {
        let content = "line 1\nline 2";
        let line_attrs = vec![LineAttribution::new(1, 2, "Alice".to_string())];

        let char_attrs = line_attributions_to_attributions(&line_attrs, content, TEST_TS);

        assert_eq!(char_attrs.len(), 1);
        assert_eq!(char_attrs[0].start, 0);
        assert_eq!(char_attrs[0].end, 13); // entire content
        assert_eq!(char_attrs[0].author_id, "Alice");
    }

    #[test]
    fn test_line_to_char_attribution_empty_input() {
        let content = "line 1\nline 2\n";
        let line_attrs = vec![];

        let char_attrs = line_attributions_to_attributions(&line_attrs, content, TEST_TS);

        assert_eq!(char_attrs.len(), 0);
    }

    #[test]
    fn test_line_to_char_attribution_empty_content() {
        let content = "";
        let line_attrs = vec![LineAttribution::new(1, 1, "Alice".to_string())];

        let char_attrs = line_attributions_to_attributions(&line_attrs, content, TEST_TS);

        assert_eq!(char_attrs.len(), 0);
    }

    #[test]
    fn test_line_to_char_attribution_invalid_line_numbers() {
        let content = "line 1\nline 2\n";
        // Line 5 doesn't exist (only 2 lines)
        let line_attrs = vec![LineAttribution::new(5, 10, "Alice".to_string())];

        let char_attrs = line_attributions_to_attributions(&line_attrs, content, TEST_TS);

        // Should skip invalid line ranges
        assert_eq!(char_attrs.len(), 0);
    }

    #[test]
    fn test_line_to_char_attribution_realistic_code() {
        let content = r#"fn main() {
    println!("hello");
}

fn test() {
    assert!(true);
}
"#;
        let line_attrs = vec![
            LineAttribution::new(1, 3, "Alice".to_string()),
            LineAttribution::new(5, 7, "Bob".to_string()),
        ];

        let char_attrs = line_attributions_to_attributions(&line_attrs, content, TEST_TS);

        assert_eq!(char_attrs.len(), 2);
        assert_eq!(char_attrs[0].author_id, "Alice");
        assert_eq!(char_attrs[1].author_id, "Bob");

        // Verify the character ranges map to correct content
        assert!(content[char_attrs[0].start..char_attrs[0].end].contains("fn main()"));
        assert!(content[char_attrs[1].start..char_attrs[1].end].contains("fn test()"));
    }

    #[test]
    fn test_line_to_char_attribution_preserves_author_order() {
        let content = "a\nb\nc\nd\ne\n";
        let line_attrs = vec![
            LineAttribution::new(1, 1, "Alice".to_string()),
            LineAttribution::new(2, 2, "Bob".to_string()),
            LineAttribution::new(3, 3, "Charlie".to_string()),
            LineAttribution::new(4, 4, "Dave".to_string()),
            LineAttribution::new(5, 5, "Eve".to_string()),
        ];

        let char_attrs = line_attributions_to_attributions(&line_attrs, content, TEST_TS);

        assert_eq!(char_attrs.len(), 5);
        assert_eq!(char_attrs[0].author_id, "Alice");
        assert_eq!(char_attrs[1].author_id, "Bob");
        assert_eq!(char_attrs[2].author_id, "Charlie");
        assert_eq!(char_attrs[3].author_id, "Dave");
        assert_eq!(char_attrs[4].author_id, "Eve");
    }

    #[test]
    fn test_line_to_char_round_trip() {
        // Test that converting to line attributions and back preserves information
        let content = "line 1\nline 2\nline 3\n";
        let original_char_attrs = vec![
            Attribution::new(0, 7, "Alice".to_string(), TEST_TS),
            Attribution::new(7, 14, "Bob".to_string(), TEST_TS),
            Attribution::new(14, 21, "Charlie".to_string(), TEST_TS),
        ];

        // Convert to line attributions
        let line_attrs = attributions_to_line_attributions(&original_char_attrs, content);

        // Convert back to character attributions
        let round_trip_char_attrs = line_attributions_to_attributions(&line_attrs, content, TEST_TS);

        // Should have same number of attributions
        assert_eq!(round_trip_char_attrs.len(), 3);

        // Should have same authors in same order
        assert_eq!(round_trip_char_attrs[0].author_id, "Alice");
        assert_eq!(round_trip_char_attrs[1].author_id, "Bob");
        assert_eq!(round_trip_char_attrs[2].author_id, "Charlie");

        // Character ranges should match original (line boundaries)
        assert_eq!(round_trip_char_attrs[0].start, 0);
        assert_eq!(round_trip_char_attrs[0].end, 7);
        assert_eq!(round_trip_char_attrs[1].start, 7);
        assert_eq!(round_trip_char_attrs[1].end, 14);
        assert_eq!(round_trip_char_attrs[2].start, 14);
        assert_eq!(round_trip_char_attrs[2].end, 21);
    }

    // ========== LineAttribution Tests ==========

    #[test]
    fn test_line_attribution_simple_single_author() {
        let content = "line 1\nline 2\nline 3\n";
        let attributions = vec![Attribution::new(0, content.len(), "Alice".to_string(), TEST_TS)];

        let line_attrs = attributions_to_line_attributions(&attributions, content);

        assert_eq!(line_attrs.len(), 1);
        assert_eq!(line_attrs[0].start_line, 1);
        assert_eq!(line_attrs[0].end_line, 3);
        assert_eq!(line_attrs[0].author_id, "Alice");
    }

    #[test]
    fn test_line_attribution_multiple_authors_distinct_lines() {
        let content = "line 1\nline 2\nline 3\n";
        // Alice: line 1, Bob: line 2, Charlie: line 3
        let attributions = vec![
            Attribution::new(0, 7, "Alice".to_string(), TEST_TS),     // "line 1\n"
            Attribution::new(7, 14, "Bob".to_string(), TEST_TS),      // "line 2\n"
            Attribution::new(14, 21, "Charlie".to_string(), TEST_TS), // "line 3\n"
        ];

        let line_attrs = attributions_to_line_attributions(&attributions, content);

        assert_eq!(line_attrs.len(), 3);
        assert_eq!(line_attrs[0].start_line, 1);
        assert_eq!(line_attrs[0].end_line, 1);
        assert_eq!(line_attrs[0].author_id, "Alice");
        assert_eq!(line_attrs[1].start_line, 2);
        assert_eq!(line_attrs[1].end_line, 2);
        assert_eq!(line_attrs[1].author_id, "Bob");
        assert_eq!(line_attrs[2].start_line, 3);
        assert_eq!(line_attrs[2].end_line, 3);
        assert_eq!(line_attrs[2].author_id, "Charlie");
    }

    #[test]
    fn test_line_attribution_whitespace_doesnt_count() {
        // Test that whitespace is ignored when determining dominant author
        let content = "    code\n";
        let attributions = vec![
            Attribution::new(0, 4, "Alice".to_string(), TEST_TS),  // "    " (4 spaces)
            Attribution::new(4, 9, "Bob".to_string(), TEST_TS),    // "code\n"
        ];

        let line_attrs = attributions_to_line_attributions(&attributions, content);

        // Alice has 0 non-ws chars, Bob has 4 non-ws chars
        assert_eq!(line_attrs.len(), 1);
        assert_eq!(line_attrs[0].author_id, "Bob");
    }

    #[test]
    fn test_line_attribution_merging_consecutive_lines() {
        let content = "line 1\nline 2\nline 3\nline 4\n";
        let attributions = vec![
            Attribution::new(0, 7, "Alice".to_string(), TEST_TS),     // line 1
            Attribution::new(7, 14, "Alice".to_string(), TEST_TS),    // line 2
            Attribution::new(14, 21, "Bob".to_string(), TEST_TS),     // line 3
            Attribution::new(21, 28, "Bob".to_string(), TEST_TS),     // line 4
        ];

        let line_attrs = attributions_to_line_attributions(&attributions, content);

        // Should merge consecutive lines with same author
        assert_eq!(line_attrs.len(), 2);
        assert_eq!(line_attrs[0].start_line, 1);
        assert_eq!(line_attrs[0].end_line, 2);
        assert_eq!(line_attrs[0].author_id, "Alice");
        assert_eq!(line_attrs[1].start_line, 3);
        assert_eq!(line_attrs[1].end_line, 4);
        assert_eq!(line_attrs[1].author_id, "Bob");
    }

    #[test]
    fn test_line_attribution_overlapping_attributions() {
        let content = "hello world\n";
        let attributions = vec![
            Attribution::new(0, 12, "Alice".to_string(), TEST_TS), // entire line
            Attribution::new(6, 11, "Bob".to_string(), TEST_TS),   // "world"
        ];

        let line_attrs = attributions_to_line_attributions(&attributions, content);

        // Alice has "hello world\n" = 10 non-ws chars (hello=5, world=5)
        // Bob has "world" = 5 non-ws chars
        // Alice should win with 10 > 5
        assert_eq!(line_attrs.len(), 1);
        assert_eq!(line_attrs[0].author_id, "Alice");
    }

    #[test]
    fn test_line_attribution_empty_content() {
        let content = "";
        let attributions = vec![];

        let line_attrs = attributions_to_line_attributions(&attributions, content);

        assert_eq!(line_attrs.len(), 0);
    }

    #[test]
    fn test_line_attribution_no_trailing_newline() {
        let content = "line 1\nline 2";
        let attributions = vec![
            Attribution::new(0, 7, "Alice".to_string(), TEST_TS),
            Attribution::new(7, 13, "Bob".to_string(), TEST_TS),
        ];

        let line_attrs = attributions_to_line_attributions(&attributions, content);

        assert_eq!(line_attrs.len(), 2);
        assert_eq!(line_attrs[0].author_id, "Alice");
        assert_eq!(line_attrs[1].author_id, "Bob");
    }

    #[test]
    fn test_line_attribution_realistic_code() {
        let content = r#"fn calculate_sum(a: i32, b: i32) -> i32 {
    let result = a + b;
    println!("Sum: {}", result);
    result
}

fn main() {
    let x = 5;
    let y = 10;
    let sum = calculate_sum(x, y);
    println!("Total: {}", sum);
}
"#;

        // Alice wrote calculate_sum function (lines 1-5)
        // Bob wrote main function (lines 7-12)
        let attributions = vec![
            Attribution::new(0, 89, "Alice".to_string(), TEST_TS),
            Attribution::new(91, content.len(), "Bob".to_string(), TEST_TS),
        ];

        let line_attrs = attributions_to_line_attributions(&attributions, content);

        // Should have Alice's block, then Bob's block
        assert!(line_attrs.len() >= 2);

        // First attribution should be Alice
        assert_eq!(line_attrs[0].author_id, "Alice");
        assert_eq!(line_attrs[0].start_line, 1);

        // Should have a Bob attribution
        let bob_attrs: Vec<_> = line_attrs.iter().filter(|a| a.author_id == "Bob").collect();
        assert!(bob_attrs.len() > 0);
    }

    #[test]
    fn test_line_attribution_mixed_authorship_per_line() {
        let content = "let x = foo() + bar();\n";
        let attributions = vec![
            Attribution::new(0, 8, "Alice".to_string(), TEST_TS),   // "let x = "
            Attribution::new(8, 13, "Bob".to_string(), TEST_TS),    // "foo()"
            Attribution::new(13, 16, "Alice".to_string(), TEST_TS), // " + "
            Attribution::new(16, 21, "Charlie".to_string(), TEST_TS), // "bar()"
            Attribution::new(21, 23, "Alice".to_string(), TEST_TS), // ";\n"
        ];

        let line_attrs = attributions_to_line_attributions(&attributions, content);

        // Alice: "let x = " (6) + " + " (1) + ";\n" (1) = 8 non-ws chars
        // Bob: "foo()" (5) = 5 non-ws chars
        // Charlie: "bar()" (5) = 5 non-ws chars
        // Alice should win
        assert_eq!(line_attrs.len(), 1);
        assert_eq!(line_attrs[0].author_id, "Alice");
    }

    #[test]
    fn test_line_attribution_all_whitespace_line() {
        let content = "code\n    \nmore code\n";
        let attributions = vec![
            Attribution::new(0, 5, "Alice".to_string(), TEST_TS),     // "code\n"
            Attribution::new(5, 10, "Bob".to_string(), TEST_TS),      // "    \n" (whitespace line)
            Attribution::new(10, 20, "Charlie".to_string(), TEST_TS), // "more code\n"
        ];

        let line_attrs = attributions_to_line_attributions(&attributions, content);

        // Line 2 has only whitespace from Bob, which after trimming becomes empty and is ignored
        // So line 2 should have no attribution, and we only get attributions for lines 1 and 3
        assert_eq!(line_attrs.len(), 2);
        assert_eq!(line_attrs[0].start_line, 1);
        assert_eq!(line_attrs[0].end_line, 1);
        assert_eq!(line_attrs[0].author_id, "Alice");
        assert_eq!(line_attrs[1].start_line, 3);
        assert_eq!(line_attrs[1].end_line, 3);
        assert_eq!(line_attrs[1].author_id, "Charlie");
    }

    #[test]
    fn test_line_attribution_helper_methods() {
        let line_attr = LineAttribution::new(5, 10, "Alice".to_string());

        assert_eq!(line_attr.line_count(), 6);
        assert!(!line_attr.is_empty());
        assert!(line_attr.overlaps(8, 12));
        assert!(!line_attr.overlaps(1, 4));
        assert_eq!(line_attr.intersection(8, 12), Some((8, 10)));
        assert_eq!(line_attr.intersection(1, 4), None);
    }

    #[test]
    fn test_move_with_unchanged_content_between() {
        let tracker = AttributionTracker::new();

        // Exact example from the bug report
        let old_content = r#"module.exports =
  ({ enabled = true, logLevel, openAnalyzer, analyzerMode } = {}) =>
  (nextConfig = {}) => {
    if (!enabled) {
      return nextConfig
    }
    if (process.env.TURBOPACK) {
      console.warn(
        'The Next Bundle Analyzer is not compatible with Turbopack builds yet, no report will be generated.\n\n' +
          'To run this analysis pass the `--webpack` flag to `next build`'
      )
      return nextConfig
    }

    const extension = analyzerMode === 'json' ? '.json' : '.html'

    return Object.assign({}, nextConfig, {
      webpack(config, options) {
        const { BundleAnalyzerPlugin } = require('webpack-bundle-analyzer')
        config.plugins.push(
          new BundleAnalyzerPlugin({
            analyzerMode: analyzerMode || 'static',
            logLevel,
            openAnalyzer,
            reportFilename: !options.nextRuntime
              ? `./analyze/client${extension}`
              : `../${options.nextRuntime === 'nodejs' ? '../' : ''}analyze/${
                  options.nextRuntime
                }${extension}`,
          })
        )

        if (typeof nextConfig.webpack === 'function') {
          return nextConfig.webpack(config, options)
        }
        return config
      },
    })
  }"#;

        let old_attributions = vec![Attribution::new(0, old_content.len(), "A".to_string(), TEST_TS)];

        // Move the if block to the end
        let new_content = r#"module.exports =
  ({ enabled = true, logLevel, openAnalyzer, analyzerMode } = {}) =>
  (nextConfig = {}) => {
    if (!enabled) {
      return nextConfig
    }
    if (process.env.TURBOPACK) {
      console.warn(
        'The Next Bundle Analyzer is not compatible with Turbopack builds yet, no report will be generated.\n\n' +
          'To run this analysis pass the `--webpack` flag to `next build`'
      )
      return nextConfig
    }

    const extension = analyzerMode === 'json' ? '.json' : '.html'

    return Object.assign({}, nextConfig, {
      webpack(config, options) {
        const { BundleAnalyzerPlugin } = require('webpack-bundle-analyzer')
        config.plugins.push(
          new BundleAnalyzerPlugin({
            analyzerMode: analyzerMode || 'static',
            logLevel,
            openAnalyzer,
            reportFilename: !options.nextRuntime
              ? `./analyze/client${extension}`
              : `../${options.nextRuntime === 'nodejs' ? '../' : ''}analyze/${
                  options.nextRuntime
                }${extension}`,
          })
        )


        return config
      },
    })
  }
  if (typeof nextConfig.webpack === 'function') {
    return nextConfig.webpack(config, options)
  }"#;

        let new_attributions = tracker
            .update_attributions(old_content, new_content, &old_attributions, "B", TEST_TS)
            .unwrap();

        // The section "return config\n      },\n    })\n  }" should NOT be attributed to B
        // It should remain attributed to A
        let return_config_pos = new_content.find("return config").unwrap();
        let closing_brace_after_return = new_content[return_config_pos..].find("  }").unwrap() + return_config_pos + 3;

        for pos in return_config_pos..closing_brace_after_return {
            let attributed_to_b = new_attributions
                .iter()
                .filter(|a| a.author_id == "B")
                .any(|a| a.start <= pos && a.end > pos);

            assert!(
                !attributed_to_b,
                "Character at position {} should NOT be attributed to B (the section between cut and paste). \
                Character: {:?}",
                pos,
                new_content.chars().nth(pos)
            );
        }
    }

    #[test]
    fn test_line_attribution_strips_leading_trailing_whitespace() {
        // Test that leading and trailing whitespace is stripped from attribution ranges
        let content = "    code    \n";
        let attributions = vec![
            Attribution::new(0, 4, "Alice".to_string(), TEST_TS),   // "    " (only whitespace)
            Attribution::new(4, 8, "Bob".to_string(), TEST_TS),     // "code"
            Attribution::new(8, 12, "Charlie".to_string(), TEST_TS), // "    " (only whitespace)
        ];

        let line_attrs = attributions_to_line_attributions(&attributions, content);

        // Only Bob should be attributed (Alice and Charlie have only whitespace)
        assert_eq!(line_attrs.len(), 1);
        assert_eq!(line_attrs[0].author_id, "Bob");
    }

    #[test]
    fn test_line_attribution_ignores_whitespace_only_ranges() {
        // Test that ranges containing only whitespace are completely ignored
        let content = "a b c\n";
        let attributions = vec![
            Attribution::new(0, 1, "Alice".to_string(), TEST_TS),   // "a"
            Attribution::new(1, 2, "Bob".to_string(), TEST_TS),     // " " (only whitespace)
            Attribution::new(2, 3, "Charlie".to_string(), TEST_TS), // "b"
            Attribution::new(3, 4, "Dave".to_string(), TEST_TS),    // " " (only whitespace)
            Attribution::new(4, 5, "Eve".to_string(), TEST_TS),     // "c"
        ];

        let line_attrs = attributions_to_line_attributions(&attributions, content);

        // Bob and Dave should be ignored (only whitespace)
        // Alice: 1 char, Charlie: 1 char, Eve: 1 char (tie, Alice wins alphabetically)
        assert_eq!(line_attrs.len(), 1);
        assert_eq!(line_attrs[0].author_id, "Alice");
    }

    #[test]
    fn test_line_attribution_trims_edges_before_counting() {
        // Test that we trim whitespace from edges before counting
        let content = "  code  \n";
        let attributions = vec![
            Attribution::new(0, 8, "Alice".to_string(), TEST_TS),  // "  code  " -> trimmed to "code"
            Attribution::new(2, 6, "Bob".to_string(), TEST_TS),    // "code"
        ];

        let line_attrs = attributions_to_line_attributions(&attributions, content);

        // Both Alice and Bob have "code" (4 chars each) after trimming
        // Alphabetically, Alice comes first
        assert_eq!(line_attrs.len(), 1);
        assert_eq!(line_attrs[0].author_id, "Alice");
    }

    #[test]
    fn test_line_attribution_mixed_with_surrounding_whitespace() {
        // Test with attribution that has whitespace in the middle but not on edges after trim
        let content = "  a b c  \n";
        let attributions = vec![
            Attribution::new(0, 3, "Alice".to_string(), TEST_TS),   // "  a" -> trimmed to "a"
            Attribution::new(3, 5, "Bob".to_string(), TEST_TS),     // " b" -> trimmed to "b"
            Attribution::new(5, 9, "Charlie".to_string(), TEST_TS), // " c  " -> trimmed to "c"
        ];

        let line_attrs = attributions_to_line_attributions(&attributions, content);

        // All three have 1 char after trimming, Alice wins alphabetically
        assert_eq!(line_attrs.len(), 1);
        assert_eq!(line_attrs[0].author_id, "Alice");
    }

    #[test]
    fn test_line_attribution_whitespace_between_words() {
        // Test that internal whitespace is properly handled but not counted
        let content = "foo   bar\n";
        let attributions = vec![
            Attribution::new(0, 3, "Alice".to_string(), TEST_TS),  // "foo"
            Attribution::new(3, 6, "Bob".to_string(), TEST_TS),    // "   " (only whitespace)
            Attribution::new(6, 9, "Charlie".to_string(), TEST_TS), // "bar"
        ];

        let line_attrs = attributions_to_line_attributions(&attributions, content);

        // Alice: 3 non-ws chars, Bob: 0 (ignored), Charlie: 3 non-ws chars
        // Tie between Alice and Charlie, Alice wins alphabetically
        assert_eq!(line_attrs.len(), 1);
        assert_eq!(line_attrs[0].author_id, "Alice");
    }

    #[test]
    fn test_line_attribution_indentation_only() {
        // Test a line that starts with attribution of only indentation
        let content = "    if (true) {\n";
        let attributions = vec![
            Attribution::new(0, 4, "Alice".to_string(), TEST_TS),   // "    " (only whitespace)
            Attribution::new(4, 15, "Bob".to_string(), TEST_TS),    // "if (true) {"
        ];

        let line_attrs = attributions_to_line_attributions(&attributions, content);

        // Alice's whitespace-only range should be ignored, Bob should win
        assert_eq!(line_attrs.len(), 1);
        assert_eq!(line_attrs[0].author_id, "Bob");
    }

    #[test]
    fn test_line_attribution_trim_tabs_and_spaces() {
        // Test that both tabs and spaces are trimmed
        let content = "\t  code  \t\n";
        let attributions = vec![
            Attribution::new(0, 10, "Alice".to_string(), TEST_TS), // "\t  code  \t" -> trimmed to "code"
        ];

        let line_attrs = attributions_to_line_attributions(&attributions, content);

        assert_eq!(line_attrs.len(), 1);
        assert_eq!(line_attrs[0].author_id, "Alice");
    }

    #[test]
    fn test_line_attribution_empty_after_trim() {
        // Test range that becomes empty after trimming (edge case)
        let content = "a   b\n";
        let attributions = vec![
            Attribution::new(0, 1, "Alice".to_string(), TEST_TS),   // "a"
            Attribution::new(1, 4, "Bob".to_string(), TEST_TS),     // "   " (only whitespace)
            Attribution::new(4, 5, "Charlie".to_string(), TEST_TS), // "b"
        ];

        let line_attrs = attributions_to_line_attributions(&attributions, content);

        // Bob's range should be ignored after trimming
        // Alice: 1 char, Charlie: 1 char, Alice wins alphabetically
        assert_eq!(line_attrs.len(), 1);
        assert_eq!(line_attrs[0].author_id, "Alice");
    }

    #[test]
    fn test_ai_human_interleaved_line_attribution_with_discard() {
        let tracker = AttributionTracker::new();

        // Start with base content and no attributions
        let old_content = "Base line";

        // Simulate adding interleaved AI and Human lines

        // Step 1: AI adds a newline and "AI Line 1"
        // "Base line" has no attributions, "\nAI Line 1" is inserted by AI
        let content_v2 = "Base line\nAI Line 1";
        let mut attributions_v2 = tracker
            .update_attributions(old_content, content_v2, &Vec::new(), &CheckpointKind::AiAgent.to_str(), TEST_TS)
            .unwrap();

        // Step 2: Human adds "Human Line 1"
        let content_v3 = "Base line\nAI Line 1\nHuman Line 1";
        let mut attributions_v3 = tracker
            .update_attributions(content_v2, content_v3, &attributions_v2, &CheckpointKind::Human.to_str(), TEST_TS)
            .unwrap();

        // Step 3: AI adds "AI Line 2"
        let content_v4 = "Base line\nAI Line 1\nHuman Line 1\nAI Line 2";
        let mut attributions_v4 = tracker
            .update_attributions(content_v3, content_v4, &attributions_v3, &CheckpointKind::AiAgent.to_str(), TEST_TS)
            .unwrap();

        // Convert to line attributions
        let line_attrs = attributions_to_line_attributions(&attributions_v4, content_v4);

        // Expected result after discarding Human:
        // Line 1 ("Base line") - was Human, now discarded, should have no attribution
        // Line 2 ("AI Line 1") - AI, should be attributed to AI
        // Line 3 ("Human Line 1") - was Human, now discarded, should have no attribution  
        // Line 4 ("AI Line 2") - AI, should be attributed to AI
        for line_attr in line_attrs {
            match line_attr.start_line {
                1 | 3 => {
                    assert_eq!(line_attr.author_id, CheckpointKind::Human.to_str());
                }
                2 | 4 => {
                    assert_eq!(line_attr.author_id, CheckpointKind::AiAgent.to_str());
                }
                _ => {
                    panic!("Unexpected line number: {:?}. Expected 1, 2, 3, or 4. Got: {:?}", line_attr.start_line, line_attr);
                }
            }
        }
    }

    #[test]
    fn test_human_replaces_ai_line() {
        // 1. Initial commit has "Line 1\nLine 2" (Human)
        // 2. AI replaces line 2 with "AI modification of line 2"
        // 3. Human replaces line 2 with "Human modification of line 2"
        // After step 3, line 2 should be attributed to Human (no AI attribution)

        let tracker = AttributionTracker::new();

        // Step 1: Initial state "Line 1\nLine 2" with no attributions (Human)
        let v1_content = "Line 1\nLine 2\n";
        let v1_attributions = Vec::new();

        // Step 2: AI modifies line 2
        let v2_content = "Line 1\nAI modification of line 2\n";
        let mut v2_attributions = tracker
            .update_attributions(v1_content, v2_content, &v1_attributions, &CheckpointKind::AiAgent.to_str(), TEST_TS)
            .unwrap();


        let v2_line_attrs = attributions_to_line_attributions(&v2_attributions, v2_content);

        // After discarding Human attributions, only line 2 should be attributed to AI
        for line_attr in v2_line_attrs {
            match line_attr.start_line {
                2 => {
                    assert_eq!(line_attr.author_id, CheckpointKind::AiAgent.to_str());
                }
                _ => {
                    panic!("Unexpected line number: {:?}. Expected 2. Got: {:?}", line_attr.start_line, line_attr);
                }
            }
        }

        // Step 3: Human replaces line 2 with different content
        let v3_content = "Line 1\nHuman modification of line 2\n";
        let v3_attributions = tracker
            .update_attributions(v2_content, v3_content, &v2_attributions, &CheckpointKind::Human.to_str(), TEST_TS)
            .unwrap();

        let v3_line_attrs = attributions_to_line_attributions(&v3_attributions, v3_content);

        // Assert that line 2 is attributed to Human
        for line_attr in v3_line_attrs {
            match line_attr.start_line {
                2 => {
                    assert_eq!(line_attr.author_id, CheckpointKind::Human.to_str());
                }
                _ => {
                    panic!("Unexpected line number: {:?}. Expected 2. Got: {:?}", line_attr.start_line, line_attr);
                }
            }
        }
    }

    #[test]
    fn test_add_multiple_lines() {
        // Simulates: Human writes 3 lines, then AI adds 2 lines
        let tracker = AttributionTracker::new();

        // Step 1: Human creates file
        let v1_content = "Line 1 from human\nLine 2 from human\nLine 3 from human\n||__AI LINE__ PENDING__||\n||__AI LINE__ PENDING__||";
        let mut v1_attributions = tracker
            .update_attributions("", v1_content, &Vec::new(), &CheckpointKind::Human.to_str(), TEST_TS)
            .unwrap();


        // Step 2: Replaces the two lines at the end
        let v2_content = "Line 1 from human\nLine 2 from human\nLine 3 from human\nLine 4 from AI\nLine 5 from AI";
        let mut v2_attributions = tracker
            .update_attributions(v1_content, v2_content, &v1_attributions, &CheckpointKind::AiAgent.to_str(), TEST_TS+1)
            .unwrap();


        let v2_line_attrs = attributions_to_line_attributions(&v2_attributions, v2_content);

        // Lines 4-5 should be attributed to AI
        assert!(v2_line_attrs.len() >= 1, "Should have at least 1 line attribution. Got: {:?}", v2_line_attrs);

        // Find the AI attribution
        let ai_attr = v2_line_attrs.iter().find(|attr| attr.author_id == CheckpointKind::AiAgent.to_str());
        assert!(ai_attr.is_some(), "Should have AI attribution. Got: {:?}", v2_line_attrs);

        let ai_attr = ai_attr.unwrap();
        assert_eq!(ai_attr.start_line, 4, "AI attribution should start at line 4");
        assert_eq!(ai_attr.end_line, 5, "AI attribution should end at line 5");
    }

    #[test]
    fn test_replace_one_human_line_with_ai_line() {
        // Simulates: Human writes 4 lines, then AI replaces one of them with an AI line
        let tracker = AttributionTracker::new();

        // Step 1: Human creates file
        let v1_content = "Line 1\nLine 2\n||__AI LINE__ PENDING__||\nLine 4";
        let mut v1_attributions = tracker
            .update_attributions("", v1_content, &Vec::new(), &CheckpointKind::Human.to_str(), TEST_TS)
            .unwrap();


        // Step 2: Replaces the two lines at the end
        let v2_content = "Line 1\nLine 2\nLine 3\nLine 4";
        let mut v2_attributions = tracker
            .update_attributions(v1_content, v2_content, &v1_attributions, &CheckpointKind::AiAgent.to_str(), TEST_TS+1)
            .unwrap();

        let v2_line_attrs = attributions_to_line_attributions(&v2_attributions, v2_content);

        // Lines 4-5 should be attributed to AI
        assert!(v2_line_attrs.len() >= 1, "Should have at least 1 line attribution. Got: {:?}", v2_line_attrs);

        // Check that line 3 is attributed to AI
        for attr in v2_line_attrs {
            match attr.start_line {
                3 => {
                    assert_eq!(attr.author_id, CheckpointKind::AiAgent.to_str());
                    assert_eq!(attr.end_line, 3);
                    break;
                }
                _ => {
                    assert_eq!(attr.author_id, CheckpointKind::Human.to_str());
                }
            }
        }
    }

    // ========== Unattributed Attribution Tests ==========

    #[test]
    fn test_attribute_unattributed_lines_empty_content() {
        let tracker = AttributionTracker::new();
        let content = "";
        let prev_attributions = vec![];
        
        let result = tracker.attribute_unattributed_ranges(content, &prev_attributions, "Alice", TEST_TS);
        
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn test_attribute_unattributed_lines_no_previous_attributions() {
        let tracker = AttributionTracker::new();
        let content = "Hello world";
        let prev_attributions = vec![];
        
        let result = tracker.attribute_unattributed_ranges(content, &prev_attributions, "Alice", TEST_TS);
        
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].start, 0);
        assert_eq!(result[0].end, 11);
        assert_eq!(result[0].author_id, "Alice");
        assert_eq!(result[0].ts, TEST_TS);
    }

    #[test]
    fn test_attribute_unattributed_lines_partially_attributed() {
        let tracker = AttributionTracker::new();
        let content = "Hello beautiful world";
        let prev_attributions = vec![
            Attribution::new(0, 6, "Bob".to_string(), TEST_TS),     // "Hello "
            Attribution::new(15, 21, "Charlie".to_string(), TEST_TS), // " world"
        ];
        
        let result = tracker.attribute_unattributed_ranges(content, &prev_attributions, "Alice", TEST_TS + 1);
        
        // Should have original attributions plus one new attribution for "beautiful"
        assert_eq!(result.len(), 3);
        
        // Check that "beautiful" (chars 6-15) is attributed to Alice
        let alice_attrs: Vec<_> = result.iter().filter(|a| a.author_id == "Alice").collect();
        assert_eq!(alice_attrs.len(), 1);
        assert_eq!(alice_attrs[0].start, 6);
        assert_eq!(alice_attrs[0].end, 15);
        assert_eq!(alice_attrs[0].ts, TEST_TS + 1);
        
        // Check that original attributions are preserved
        let bob_attrs: Vec<_> = result.iter().filter(|a| a.author_id == "Bob").collect();
        let charlie_attrs: Vec<_> = result.iter().filter(|a| a.author_id == "Charlie").collect();
        assert_eq!(bob_attrs.len(), 1);
        assert_eq!(charlie_attrs.len(), 1);
    }

    #[test]
    fn test_attribute_unattributed_lines_multiple_gaps() {
        let tracker = AttributionTracker::new();
        let content = "A B C D E F";
        let prev_attributions = vec![
            Attribution::new(0, 1, "Bob".to_string(), TEST_TS),     // "A"
            Attribution::new(4, 5, "Charlie".to_string(), TEST_TS), // "C"
            Attribution::new(8, 9, "Dave".to_string(), TEST_TS),    // "E"
        ];
        
        let result = tracker.attribute_unattributed_ranges(content, &prev_attributions, "Alice", TEST_TS + 1);
        
        // Should have 3 original attributions + 3 new Alice attributions for gaps
        assert_eq!(result.len(), 6);
        
        // Check Alice attributions for the gaps
        let alice_attrs: Vec<_> = result.iter().filter(|a| a.author_id == "Alice").collect();
        assert_eq!(alice_attrs.len(), 3);
        
        // Gap 1: " B " (chars 1-4)
        // Gap 2: " D " (chars 5-8) 
        // Gap 3: " F" (chars 9-11)
        let gap_ranges: Vec<_> = alice_attrs.iter().map(|a| (a.start, a.end)).collect();
        assert!(gap_ranges.contains(&(1, 4)), "Should have gap 1-4");
        assert!(gap_ranges.contains(&(5, 8)), "Should have gap 5-8");
        assert!(gap_ranges.contains(&(9, 11)), "Should have gap 9-11");
    }

    #[test]
    fn test_attribute_unattributed_lines_contiguous_gaps() {
        let tracker = AttributionTracker::new();
        let content = "ABC";
        let prev_attributions = vec![
            Attribution::new(0, 1, "Bob".to_string(), TEST_TS),     // "A"
        ];
        
        let result = tracker.attribute_unattributed_ranges(content, &prev_attributions, "Alice", TEST_TS + 1);
        
        // Should have 1 original attribution + 1 new attribution for "BC"
        assert_eq!(result.len(), 2);
        
        let alice_attrs: Vec<_> = result.iter().filter(|a| a.author_id == "Alice").collect();
        assert_eq!(alice_attrs.len(), 1);
        assert_eq!(alice_attrs[0].start, 1);
        assert_eq!(alice_attrs[0].end, 3);
    }

    #[test]
    fn test_attribute_unattributed_lines_fully_attributed() {
        let tracker = AttributionTracker::new();
        let content = "Hello";
        let prev_attributions = vec![
            Attribution::new(0, 5, "Bob".to_string(), TEST_TS),
        ];
        
        let result = tracker.attribute_unattributed_ranges(content, &prev_attributions, "Alice", TEST_TS + 1);
        
        // Should have only the original attribution, no new ones
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].author_id, "Bob");
    }

    #[test]
    fn test_attribute_unattributed_lines_multiline_content() {
        let tracker = AttributionTracker::new();
        let content = "Line 1\nLine 2\nLine 3\n";
        let prev_attributions = vec![
            Attribution::new(0, 7, "Bob".to_string(), TEST_TS),     // "Line 1\n"
            // Line 2 is unattributed
            Attribution::new(14, 21, "Charlie".to_string(), TEST_TS), // "Line 3\n"
        ];
        
        let result = tracker.attribute_unattributed_ranges(content, &prev_attributions, "Alice", TEST_TS + 1);
        
        // Should have 2 original attributions + 1 new attribution for "Line 2\n"
        assert_eq!(result.len(), 3);
        
        let alice_attrs: Vec<_> = result.iter().filter(|a| a.author_id == "Alice").collect();
        assert_eq!(alice_attrs.len(), 1);
        assert_eq!(alice_attrs[0].start, 7);
        assert_eq!(alice_attrs[0].end, 14);
    }

    #[test]
    fn test_attribute_unattributed_lines_preserves_timestamps() {
        let tracker = AttributionTracker::new();
        let content = "Hello world";
        let prev_attributions = vec![
            Attribution::new(0, 6, "Bob".to_string(), 1000),
        ];
        
        let result = tracker.attribute_unattributed_ranges(content, &prev_attributions, "Alice", 2000);
        
        assert_eq!(result.len(), 2);
        
        // Check that original timestamp is preserved
        let bob_attr = result.iter().find(|a| a.author_id == "Bob").unwrap();
        assert_eq!(bob_attr.ts, 1000);
        
        // Check that new attribution has correct timestamp
        let alice_attr = result.iter().find(|a| a.author_id == "Alice").unwrap();
        assert_eq!(alice_attr.ts, 2000);
    }

    #[test]
    fn test_attribute_unattributed_lines_complex_overlapping() {
        let tracker = AttributionTracker::new();
        let content = "ABCDEFGHIJ";
        let prev_attributions = vec![
            Attribution::new(1, 3, "Bob".to_string(), TEST_TS),     // "BC"
            Attribution::new(4, 6, "Charlie".to_string(), TEST_TS), // "EF"
            Attribution::new(7, 9, "Dave".to_string(), TEST_TS),    // "HI"
        ];
        
        let result = tracker.attribute_unattributed_ranges(content, &prev_attributions, "Alice", TEST_TS + 1);
        
        // Should have 3 original attributions + 4 new Alice attributions for gaps
        assert_eq!(result.len(), 7);
        
        let alice_attrs: Vec<_> = result.iter().filter(|a| a.author_id == "Alice").collect();
        assert_eq!(alice_attrs.len(), 4);
        
        // Gap 1: "A" (char 0)
        // Gap 2: "D" (char 3) 
        // Gap 3: "G" (char 6)
        // Gap 4: "J" (char 9)
        let gap_ranges: Vec<_> = alice_attrs.iter().map(|a| (a.start, a.end)).collect();
        assert!(gap_ranges.contains(&(0, 1)), "Should have gap 0-1");
        assert!(gap_ranges.contains(&(3, 4)), "Should have gap 3-4");
        assert!(gap_ranges.contains(&(6, 7)), "Should have gap 6-7");
        assert!(gap_ranges.contains(&(9, 10)), "Should have gap 9-10");
    }
}

