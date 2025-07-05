use crate::commands::{blame, checkpoint, post_commit};
use crate::error::GitAiError;
use git2::{Repository, Signature};
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

pub struct TmpFile {
    repo: TmpRepo,
    filename: String,
    contents: String,
}

impl TmpFile {
    /// Updates the entire contents of the file
    pub fn update(&mut self, new_contents: &str) -> Result<(), GitAiError> {
        self.contents = new_contents.to_string();
        self.write_to_disk()?;
        self.flush_to_disk()
    }

    /// Appends content to the end of the file
    pub fn append(&mut self, content: &str) -> Result<(), GitAiError> {
        self.contents.push_str(content);
        self.write_to_disk()?;
        self.flush_to_disk()
    }

    /// Inserts content at a specific position
    pub fn insert_at(&mut self, position: usize, content: &str) -> Result<(), GitAiError> {
        if position > self.contents.len() {
            return Err(GitAiError::Generic(format!(
                "Position {} is out of bounds for file with {} characters",
                position,
                self.contents.len()
            )));
        }

        let mut new_contents = String::new();
        new_contents.push_str(&self.contents[..position]);
        new_contents.push_str(content);
        new_contents.push_str(&self.contents[position..]);

        self.contents = new_contents;
        self.write_to_disk()?;
        self.flush_to_disk()
    }

    /// Replaces content at a specific position with new content
    pub fn replace_at(&mut self, position: usize, new_content: &str) -> Result<(), GitAiError> {
        if position > self.contents.len() {
            return Err(GitAiError::Generic(format!(
                "Position {} is out of bounds for file with {} characters",
                position,
                self.contents.len()
            )));
        }
        let mut new_contents = self.contents.clone();
        new_contents.replace_range(position..position + new_content.len(), new_content);
        self.contents = new_contents;
        self.write_to_disk()?;
        self.flush_to_disk()
    }

    /// Replaces a range of content with new content
    pub fn replace_range(
        &mut self,
        start: usize,
        end: usize,
        new_content: &str,
    ) -> Result<(), GitAiError> {
        if start >= self.contents.len() || end > self.contents.len() || start >= end {
            return Err(GitAiError::Generic(format!(
                "Invalid range [{}, {}) for file with {} characters",
                start,
                end,
                self.contents.len()
            )));
        }

        let mut new_contents = String::new();
        new_contents.push_str(&self.contents[..start]);
        new_contents.push_str(new_content);
        new_contents.push_str(&self.contents[end..]);

        self.contents = new_contents;
        self.write_to_disk()
    }

    /// Gets the current contents of the file
    pub fn contents(&self) -> &str {
        &self.contents
    }

    /// Gets the filename
    pub fn filename(&self) -> &str {
        &self.filename
    }

    /// Gets the full path of the file
    pub fn path(&self) -> PathBuf {
        self.repo.path.join(&self.filename)
    }

    /// Gets the length of the file contents
    pub fn len(&self) -> usize {
        self.contents.len()
    }

    /// Checks if the file is empty
    pub fn is_empty(&self) -> bool {
        self.contents.is_empty()
    }

    /// Clears all contents from the file
    pub fn clear(&mut self) -> Result<(), GitAiError> {
        self.contents.clear();
        self.write_to_disk()?;
        self.flush_to_disk()
    }

    /// Writes the current contents to disk
    fn write_to_disk(&self) -> Result<(), GitAiError> {
        let file_path = self.repo.path.join(&self.filename);

        // Create parent directories if they don't exist
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Write the file
        fs::write(&file_path, &self.contents)?;

        // Add to git index using the filename directly
        let mut index = self.repo.repo.index()?;
        index.add_path(&std::path::Path::new(&self.filename))?;
        index.write()?;

        Ok(())
    }

    /// Flushes the file to disk to ensure all changes are written
    fn flush_to_disk(&self) -> Result<(), GitAiError> {
        use std::fs::OpenOptions;
        use std::io::Write;
        let file_path = self.repo.path.join(&self.filename);
        if let Ok(mut file) = OpenOptions::new().write(true).open(&file_path) {
            file.flush()?;
        }
        Ok(())
    }
}

pub struct TmpRepo {
    path: PathBuf,
    repo: Repository,
}

impl TmpRepo {
    /// Creates a new temporary repository at the given path
    pub fn new(tmp_dir: PathBuf) -> Result<Self, GitAiError> {
        // Create the directory if it doesn't exist
        fs::create_dir_all(&tmp_dir)?;

        // Initialize git repository
        let repo = Repository::init(&tmp_dir)?;

        // Configure git user for commits
        let mut config = repo.config()?;
        config.set_str("user.name", "Test User")?;
        config.set_str("user.email", "test@example.com")?;

        // (No initial empty commit)
        Ok(TmpRepo {
            path: tmp_dir,
            repo,
        })
    }

    /// Writes a file with the given filename and contents, returns a TmpFile for further updates
    pub fn write_file(
        &self,
        filename: &str,
        contents: &str,
        add_to_git: bool,
    ) -> Result<TmpFile, GitAiError> {
        let file_path = self.path.join(filename);

        // Create parent directories if they don't exist
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Write the file
        fs::write(&file_path, contents)?;

        if add_to_git {
            let mut index = self.repo.index()?;
            index.add_path(&file_path.strip_prefix(&self.path).unwrap())?;
            index.write()?;
        }

        Ok(TmpFile {
            repo: TmpRepo {
                path: self.path.clone(),
                repo: Repository::open(&self.path)?,
            },
            filename: filename.to_string(),
            contents: contents.to_string(),
        })
    }

    /// Triggers a checkpoint with the given author
    pub fn trigger_checkpoint_with_author(
        &self,
        author: &str,
    ) -> Result<(usize, usize, usize), GitAiError> {
        checkpoint(
            &self.repo, author, false, // show_working_log
            false, // reset
            true,  // quiet
            None,  // model
            None,  // human_author
        )
    }

    /// Commits all changes with the given message and runs post-commit hook
    pub fn commit_with_message(&self, message: &str) -> Result<(), GitAiError> {
        // Add all files to the index
        let mut index = self.repo.index()?;
        index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)?;
        index.write()?;

        // Create the commit
        let tree_id = index.write_tree()?;
        let tree = self.repo.find_tree(tree_id)?;

        let signature = Signature::now("Test User", "test@example.com")?;

        // Check if there's a parent commit before we use it
        let _has_parent = if let Ok(head) = self.repo.head() {
            if let Some(target) = head.target() {
                self.repo.find_commit(target).is_ok()
            } else {
                false
            }
        } else {
            false
        };

        // Get the current HEAD for the parent commit
        let parent_commit = if let Ok(head) = self.repo.head() {
            if let Some(target) = head.target() {
                Some(self.repo.find_commit(target)?)
            } else {
                None
            }
        } else {
            None
        };

        let _commit_id = if let Some(parent) = parent_commit {
            self.repo.commit(
                Some(&"HEAD"),
                &signature,
                &signature,
                message,
                &tree,
                &[&parent],
            )?
        } else {
            self.repo
                .commit(Some(&"HEAD"), &signature, &signature, message, &tree, &[])?
        };

        println!("Commit ID: {}", _commit_id);

        // Run the post-commit hook for all commits (including initial commit)
        post_commit(&self.repo, false)?; // false = not force

        Ok(())
    }

    /// Gets the repository path
    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    /// Gets a reference to the underlying git2 Repository
    pub fn repo(&self) -> &Repository {
        &self.repo
    }

    /// Runs blame on a file in the repository
    pub fn blame_for_file(
        &self,
        tmp_file: &TmpFile,
        line_range: Option<(u32, u32)>,
    ) -> Result<BTreeMap<u32, String>, GitAiError> {
        // Use the filename (relative path) instead of the absolute path
        // Convert the blame result to BTreeMap for deterministic order
        let blame_map = blame(&self.repo, &tmp_file.filename, line_range)?;
        Ok(blame_map.into_iter().collect())
    }
}
