//! Attribution tracking through file changes
//!
//! This library maintains attribution ranges as files are edited, preserving
//! authorship information even through moves, edits, and whitespace changes.

use diff_match_patch_rs::dmp::Diff;
use crate::error::GitAiError;
use diff_match_patch_rs::{Compat, DiffMatchPatch, Ops};
use std::collections::HashMap;

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
}

impl Attribution {
    pub fn new(start: usize, end: usize, author_id: String) -> Self {
        Attribution {
            start,
            end,
            author_id,
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

    /// Convert character position to line number (1-indexed)
    pub fn char_pos_to_line_num(content: &str, pos: usize) -> u32 {
        content[..pos.min(content.len())]
            .chars()
            .filter(|&c| c == '\n') // TODO More robust line counting
            .count() as u32
            + 1
    }

    /// Convert this attribution to a list of line numbers it covers
    pub fn to_line_numbers(&self, content: &str) -> Vec<u32> {
        if self.is_empty() {
            return Vec::new();
        }

        let start_line = Self::char_pos_to_line_num(content, self.start);
        let end_line = Self::char_pos_to_line_num(content, self.end.saturating_sub(1));

        (start_line..=end_line).collect()
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

                    for search_len in (1..=insertion_text.len()).rev() {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_insertion() {
        let tracker = AttributionTracker::new();

        let old_content = "Hello world";
        let new_content = "Hello beautiful world";

        let old_attributions = vec![Attribution::new(0, 11, "Alice".to_string())];

        let new_attributions = tracker
            .update_attributions(old_content, new_content, &old_attributions, "Bob")
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
            Attribution::new(0, 6, "Alice".to_string()),
            Attribution::new(6, 16, "Bob".to_string()),
            Attribution::new(16, 21, "Alice".to_string()),
        ];

        let new_attributions = tracker
            .update_attributions(old_content, new_content, &old_attributions, "Charlie")
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
            Attribution::new(0, 34, "Alice".to_string()), // fn helper() { ... }
            Attribution::new(36, 70, "Bob".to_string()),   // fn main() { ... }
        ];

        let new_attributions = tracker
            .update_attributions(old_content, new_content, &old_attributions, "Charlie")
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

        let old_attributions = vec![Attribution::new(0, 23, "Alice".to_string())];

        let new_attributions = tracker
            .update_attributions(old_content, new_content, &old_attributions, "Bob")
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
            Attribution::new(0, 11, "Alice".to_string()),
            Attribution::new(0, 5, "Bob".to_string()),
        ];

        let new_attributions = tracker
            .update_attributions(old_content, new_content, &old_attributions, "Charlie")
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

        let old_attributions = vec![Attribution::new(0, 19, "Alice".to_string())];

        let new_attributions = tracker
            .update_attributions(old_content, new_content, &old_attributions, "Bob")
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
            .update_attributions(old_content, new_content, &old_attributions, "Alice")
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

        let old_attributions = vec![Attribution::new(0, 11, "Alice".to_string())];

        let new_attributions = tracker
            .update_attributions(old_content, new_content, &old_attributions, "Bob")
            .unwrap();

        assert_eq!(new_attributions.len(), 1);
        assert_eq!(new_attributions[0].author_id, "Alice");
        assert_eq!(new_attributions[0].start, 0);
        assert_eq!(new_attributions[0].end, 11);
    }

    #[test]
    fn test_attribution_overlap() {
        let attr = Attribution::new(10, 20, "Alice".to_string());

        assert!(attr.overlaps(15, 25));
        assert!(attr.overlaps(5, 15));
        assert!(attr.overlaps(5, 25));
        assert!(attr.overlaps(12, 18));
        assert!(!attr.overlaps(0, 10));
        assert!(!attr.overlaps(20, 30));
    }

    #[test]
    fn test_attribution_intersection() {
        let attr = Attribution::new(10, 20, "Alice".to_string());

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
            Attribution::new(0, 10, "Alice".to_string()),  // // Header
            Attribution::new(10, 34, "Bob".to_string()),   // fn foo() { bar(); }
            Attribution::new(35, 63, "Charlie".to_string()), // fn main() { foo(); }
        ];

        let new_attributions = tracker
            .update_attributions(old_content, new_content, &old_attributions, "Dave")
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
        let v1_attributions = vec![Attribution::new(0, 24, "A".to_string())];

        // B adds a line
        let v2_content = "console.log(\"A- HELLO\")\nconsole.log(\"B- HELLO\")\n";
        let v2_attributions = tracker
            .update_attributions(v1_content, v2_content, &v1_attributions, "B")
            .unwrap();

        // A adds three empty lines between B's line and the next line
        let v3_content = "console.log(\"A- HELLO\")\nconsole.log(\"B- HELLO\")\n\n\n\n";
        let v3_attributions = tracker
            .update_attributions(v2_content, v3_content, &v2_attributions, "A")
            .unwrap();

        // C adds a line
        let v4_content = "console.log(\"A- HELLO\")\nconsole.log(\"B- HELLO\")\n\n\n\nconsole.log(\"C- HELLO\")";
        let v4_attributions = tracker
            .update_attributions(v3_content, v4_content, &v3_attributions, "C")
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

        let old_attributions = vec![Attribution::new(0, old_content.len(), "A".to_string())];

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
            .update_attributions(old_content, new_content, &old_attributions, "B")
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
}
