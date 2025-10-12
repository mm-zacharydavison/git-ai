use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq)]
pub enum AuthorType {
    Human,
    Ai,
}

#[derive(Debug, Clone)]
pub struct ExpectedLine {
    pub contents: String,
    pub author_type: AuthorType,
}

impl ExpectedLine {
    fn new(contents: String, author_type: AuthorType) -> Self {
        if contents.contains('\n') {
            panic!(
                "fluent test file API does not support strings with new lines (must be a single line): {:?}",
                contents
            );
        }
        Self {
            contents,
            author_type,
        }
    }
}

/// Trait to add .ai() and .human() methods to string types
pub trait ExpectedLineExt {
    fn ai(self) -> ExpectedLine;
    fn human(self) -> ExpectedLine;
}

impl ExpectedLineExt for &str {
    fn ai(self) -> ExpectedLine {
        ExpectedLine::new(self.to_string(), AuthorType::Ai)
    }

    fn human(self) -> ExpectedLine {
        ExpectedLine::new(self.to_string(), AuthorType::Human)
    }
}

impl ExpectedLineExt for String {
    fn ai(self) -> ExpectedLine {
        ExpectedLine::new(self, AuthorType::Ai)
    }

    fn human(self) -> ExpectedLine {
        ExpectedLine::new(self, AuthorType::Human)
    }
}

/// Default conversion from &str to ExpectedLine (defaults to Human authorship)
impl From<&str> for ExpectedLine {
    fn from(s: &str) -> Self {
        ExpectedLine::new(s.to_string(), AuthorType::Human)
    }
}

/// Default conversion from String to ExpectedLine (defaults to Human authorship)
impl From<String> for ExpectedLine {
    fn from(s: String) -> Self {
        ExpectedLine::new(s, AuthorType::Human)
    }
}

#[derive(Debug, Clone)]
pub struct TestFile<'a> {
    pub lines: Vec<ExpectedLine>,
    pub file_path: PathBuf,
    pub repo: &'a super::test_repo::TestRepo,
}

impl<'a> TestFile<'a> {
    pub fn new_with_filename(
        file_path: PathBuf,
        lines: Vec<ExpectedLine>,
        repo: &'a super::test_repo::TestRepo,
    ) -> Self {
        Self {
            lines,
            file_path: file_path,
            repo,
        }
    }

    fn forks_with_new_lines(&self, lines: Vec<ExpectedLine>) -> Self {
        Self {
            lines,
            file_path: self.file_path.clone(),
            repo: self.repo,
        }
    }

    pub fn assert_file_contents_expected(&self) {
        let contents = fs::read_to_string(&self.file_path).unwrap();
        assert_eq!(
            contents,
            self.to_contents(),
            "Unexpected contents in file: {}",
            self.file_path.display(),
        );
    }

    /// Assert that the file at the given path matches the expected contents and authorship
    pub fn assert_blame_contents_expected(&self) {
        // Get blame output
        let blame_output = self
            .repo
            .git_ai(&format!("blame {}", self.file_path.display()))
            .expect("git-ai blame should succeed");

        println!(
            "\n=== Git-AI Blame Output ===\n{}\n===========================\n",
            blame_output
        );

        // Parse the blame output to extract authors for each line
        let lines_by_author = self.parse_blame_output(&blame_output);

        println!("Parsed authors: {:?}", lines_by_author);

        // Compare with expected authorship
        assert_eq!(
            lines_by_author.len(),
            self.lines.len(),
            "Number of lines in blame output ({}) doesn't match expected ({})",
            lines_by_author.len(),
            self.lines.len()
        );

        for (i, (actual_author, expected_line)) in
            lines_by_author.iter().zip(&self.lines).enumerate()
        {
            let line_num = i + 1;
            match &expected_line.author_type {
                AuthorType::Ai => {
                    assert!(
                        self.is_ai_author(actual_author),
                        "Line {}: Expected AI author but got '{}'. Expected line: {:?}",
                        line_num,
                        actual_author,
                        expected_line
                    );
                }
                AuthorType::Human => {
                    assert!(
                        !self.is_ai_author(actual_author),
                        "Line {}: Expected Human author but got AI author '{}'. Expected line: {:?}",
                        line_num,
                        actual_author,
                        expected_line
                    );
                }
            }
        }
    }

    /// Parse git-ai blame output and extract the author for each line
    /// Format: sha (author date line_num) content
    fn parse_blame_output(&self, blame_output: &str) -> Vec<String> {
        blame_output
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                // Find the author between the first '(' and the timestamp
                if let Some(start_paren) = line.find('(') {
                    let after_paren = &line[start_paren + 1..];
                    // Author is everything before the date/timestamp
                    // Date format is typically "YYYY-MM-DD" or similar
                    // Split by multiple spaces or look for year pattern
                    let parts: Vec<&str> = after_paren.trim().split_whitespace().collect();
                    if !parts.is_empty() {
                        // The author is typically the first part before the date
                        // Date usually starts with a year (4 digits) or a number
                        let mut author_parts = Vec::new();
                        for part in parts {
                            // Stop when we hit what looks like a date (starts with digit)
                            if part.chars().next().unwrap_or('a').is_ascii_digit() {
                                break;
                            }
                            author_parts.push(part);
                        }
                        return author_parts.join(" ");
                    }
                }
                "unknown".to_string()
            })
            .collect()
    }

    /// Check if an author string indicates AI authorship
    /// AI authors typically contain keywords like "mock_ai", agent names, etc.
    fn is_ai_author(&self, author: &str) -> bool {
        let author_lower = author.to_lowercase();
        author_lower.contains("mock_ai")
            || author_lower.contains("some-ai")
            || author_lower.contains("claude")
            || author_lower.contains("gpt")
            || author_lower.contains("copilot")
            || author_lower.contains("cursor")
    }

    /// Get the expected file contents as a single string (without authorship info)
    pub fn to_contents(&self) -> String {
        self.lines
            .iter()
            .map(|line| line.contents.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Get lines with a specific author type
    pub fn lines_by_author(&self, author_type: AuthorType) -> Vec<&ExpectedLine> {
        self.lines
            .iter()
            .filter(|line| line.author_type == author_type)
            .collect()
    }

    /// Insert lines at the specified index, returning a new ExpectedTextState
    /// Simplifying assumption: all lines are same author type or panic
    pub fn insert_at(&self, index: usize, lines: Vec<ExpectedLine>) -> Self {
        let mut new_lines = self.lines.clone();
        new_lines.splice(index..index, lines);
        Self::new_with_filename(self.file_path.clone(), new_lines, self.repo)
    }

    /// Replace a single line at the specified index, returning a new ExpectedTextState
    /// /// Simplifying assumption: all lines are same author type or panic
    pub fn replace_at(&self, index: usize, line: ExpectedLine) -> Self {
        let mut new_lines = self.lines.clone();
        if index < new_lines.len() {
            new_lines[index] = line;
        } else {
            panic!(
                "Index {} out of bounds for {} lines",
                index,
                new_lines.len()
            );
        }
        Self::new_with_filename(self.file_path.clone(), new_lines, self.repo)
    }

    pub fn set_contents(&self, lines: Vec<ExpectedLine>) -> Self {
        // stub in AI Lines
        let line_contents = lines
            .iter()
            .map(|s| {
                if s.author_type == AuthorType::Ai {
                    return "||__AI LINE__ PENDING__||".to_string();
                } else {
                    return s.contents.clone();
                }
            })
            .collect::<Vec<String>>()
            .join("\n");

        self.write_and_checkpoint_with_contents(&line_contents, &AuthorType::Human);

        let line_contents_with_ai = lines
            .iter()
            .map(|s| s.contents.clone())
            .collect::<Vec<String>>()
            .join("\n");

        self.write_and_checkpoint_with_contents(&line_contents_with_ai, &AuthorType::Ai);

        Self::new_with_filename(self.file_path.clone(), lines, self.repo)
    }

    /// Replace a range of lines [start..end) with new lines, returning a new ExpectedTextState
    /// Simplifying assumption: all lines are same author type or panic
    pub fn replace_range_at(&self, start: usize, end: usize, lines: Vec<ExpectedLine>) -> Self {
        let mut new_lines = self.lines.clone();
        new_lines.splice(start..end, lines);
        Self::new_with_filename(self.file_path.clone(), new_lines, self.repo)
    }

    fn _write_and_checkpoint(&self, _author_type: &AuthorType) {
        // TODO: Implement actual write and checkpoint
    }

    fn write_and_checkpoint_with_contents(&self, contents: &str, author_type: &AuthorType) {
        fs::write(&self.file_path, contents).unwrap();

        // Stage the file first
        self.repo
            .git(&format!("add {}", self.file_path.display()))
            .unwrap();

        let result = if author_type == &AuthorType::Ai {
            self.repo.git_ai("checkpoint mock_ai")
        } else {
            self.repo.git_ai("checkpoint")
        };

        // match &result {
        //     Ok(output) => println!("✓ checkpoint succeeded: {:?}", output),
        //     Err(error) => println!("✗ checkpoint failed: {:?}", error),
        // }

        result.unwrap();
    }
}

// /// Macro to build ExpectedTextState with a fluent syntax
// /// Accepts ExpectedLine or any type convertible to ExpectedLine (e.g., &str, String)
// /// Plain strings default to Human authorship
// #[macro_export]
// macro_rules! expect_contents {
//     ($($line:expr),+ $(,)?) => {{
//         TestFile::new_with_filename(PathBuf::from(""), vec![$($line.into()),+])
//     }};
// }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expected_line_creation() {
        let line = "Hello world".ai();
        assert_eq!(line.contents, "Hello world");
        assert_eq!(line.author_type, AuthorType::Ai);

        let line = "User input".human();
        assert_eq!(line.contents, "User input");
        assert_eq!(line.author_type, AuthorType::Human);
    }

    #[test]
    #[should_panic(
        expected = "fluent test file API does not support strings with new lines (must be a single line)"
    )]
    fn test_multiline_panic() {
        let _line = "Line 1\nLine 2".ai();
    }
}
