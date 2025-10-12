use git_ai::git::repo_storage::PersistedWorkingLog;
use git_ai::git::repository as GitAiRepository;
use git2::Repository;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;
use std::{fs, fs::File, io::Read};

use super::test_file::TestFile;

#[derive(Clone, Debug)]
pub struct TestRepo {
    path: PathBuf,
}

impl TestRepo {
    pub fn new() -> Self {
        let mut buf = [0u8; 8];
        File::open("/dev/urandom")
            .expect("failed to open /dev/urandom")
            .read_exact(&mut buf)
            .expect("failed to read random bytes");

        let n = u64::from_le_bytes(buf) % 10000000000;
        let base = std::env::temp_dir();
        let path = base.join(n.to_string());
        let repo = Repository::init(&path).expect("failed to initialize git2 repository");
        let mut config = Repository::config(&repo).expect("failed to initialize git2 repository");
        config
            .set_str("user.name", "Test User")
            .expect("failed to initialize git2 repository");
        config
            .set_str("user.email", "test@example.com")
            .expect("failed to initialize git2 repository");

        Self { path }
    }

    pub fn git_ai(&self, command: &str) -> Result<String, String> {
        let binary_path = get_binary_path();
        let args: Vec<&str> = command.split_whitespace().collect();

        let output = Command::new(binary_path)
            .args(&args)
            .current_dir(&self.path)
            .output()
            .expect(&format!("Failed to execute git-ai command: {}", command));

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if output.status.success() {
            // Combine stdout and stderr since git-ai often writes to stderr
            let combined = if stdout.is_empty() {
                stderr
            } else if stderr.is_empty() {
                stdout
            } else {
                format!("{}{}", stdout, stderr)
            };
            Ok(combined)
        } else {
            Err(stderr)
        }
    }

    pub fn git(&self, command: &str) -> Result<String, String> {
        let args: Vec<&str> = command.split_whitespace().collect();

        let mut full_args = vec!["-C", self.path.to_str().unwrap()];
        full_args.extend(args);

        let output = Command::new("git")
            .args(&full_args)
            .output()
            .expect(&format!("Failed to execute git command: {}", command));

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if output.status.success() {
            // Combine stdout and stderr since git often writes to stderr
            let combined = if stdout.is_empty() {
                stderr
            } else if stderr.is_empty() {
                stdout
            } else {
                format!("{}{}", stdout, stderr)
            };
            Ok(combined)
        } else {
            Err(stderr)
        }
    }

    pub fn filename(&self, filename: &str) -> TestFile {
        TestFile::new_with_filename(self.path.join(filename), vec![], self)
    }

    pub fn current_working_logs(&self) -> PersistedWorkingLog {
        let repo = GitAiRepository::find_repository_in_path(self.path.to_str().unwrap())
            .expect("Failed to find repository");

        // Get the current HEAD commit SHA, or use "initial" for empty repos
        let commit_sha = repo
            .head()
            .ok()
            .and_then(|head| head.target().ok())
            .unwrap_or_else(|| "initial".to_string());

        // Get the working log for the current HEAD commit
        repo.storage.working_log_for_base_commit(&commit_sha)
    }

    pub fn stage_all_and_commit(&self, message: &str) -> Result<String, String> {
        self.git("add -A").expect("add --all should succeed");

        // Use git_with_args for proper argument handling
        let full_args = vec!["-C", self.path.to_str().unwrap(), "commit", "-m", message];

        let output = Command::new("git")
            .args(&full_args)
            .output()
            .expect("Failed to execute git commit command");

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if output.status.success() {
            let combined = if stdout.is_empty() {
                stderr
            } else if stderr.is_empty() {
                stdout
            } else {
                format!("{}{}", stdout, stderr)
            };
            Ok(combined)
        } else {
            Err(stderr)
        }
    }

    pub fn read_file(&self, filename: &str) -> Option<String> {
        let file_path = self.path.join(filename);
        fs::read_to_string(&file_path).ok()
    }
}

static COMPILED_BINARY: OnceLock<PathBuf> = OnceLock::new();

fn compile_binary() -> PathBuf {
    println!("Compiling git-ai binary for tests...");

    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let output = Command::new("cargo")
        .args(&["build", "--bin", "git-ai"])
        .current_dir(manifest_dir)
        .output()
        .expect("Failed to compile git-ai binary");

    if !output.status.success() {
        panic!(
            "Failed to compile git-ai:\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let binary_path = PathBuf::from(manifest_dir).join("target/debug/git-ai");
    println!("Binary compiled at: {}", binary_path.display());
    binary_path
}

fn get_binary_path() -> &'static PathBuf {
    COMPILED_BINARY.get_or_init(compile_binary)
}

#[cfg(test)]
mod tests {
    use super::super::test_file::ExpectedLineExt;
    use super::TestRepo;

    #[test]
    fn test_invoke_git() {
        let repo = TestRepo::new();
        let output = repo.git("status").expect("git status should succeed");
        println!("output: {}", output);
        assert!(output.contains("On branch"));
    }

    #[test]
    fn test_invoke_git_ai() {
        let repo = TestRepo::new();
        let output = repo
            .git_ai("version")
            .expect("git-ai version should succeed");
        assert!(!output.is_empty());
    }

    #[test]
    fn test_exp() {
        let repo = TestRepo::new();

        let example_txt = repo.filename("example.txt");
        let example_txt = example_txt.set_contents(vec!["HUMAN".human(), "Hello, world!".ai()]);

        let output = repo.stage_all_and_commit("mix ai human").unwrap();

        println!("{:?}", repo.path);
        // Assert that blame output matches expected authorship
        example_txt.assert_blame_contents_expected();
    }
}
