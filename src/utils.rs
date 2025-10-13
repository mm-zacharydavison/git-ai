use crate::git::diff_tree_to_tree::Diff;
use std::time::{Duration, Instant};

/// Debug logging utility function
///
/// Prints debug messages with a colored prefix when debug assertions are enabled.
/// This function only outputs messages when the code is compiled with debug assertions.
///
/// # Arguments
///
/// * `msg` - The debug message to print
pub fn debug_log(msg: &str) {
    if cfg!(debug_assertions) {
        eprintln!("\x1b[1;33m[git-ai]\x1b[0m {}", msg);
    }
}

/// Print a git diff in a readable format
///
/// Prints the diff between two commits/trees showing which files changed and their status.
/// This is useful for debugging and understanding what changes occurred.
///
/// # Arguments
///
/// * `diff` - The git diff object to print
/// * `old_label` - Label for the "old" side (e.g., commit SHA or description)
/// * `new_label` - Label for the "new" side (e.g., commit SHA or description)
pub fn _print_diff(diff: &Diff, old_label: &str, new_label: &str) {
    println!("Diff between {} and {}:", old_label, new_label);

    let mut file_count = 0;
    for delta in diff.deltas() {
        file_count += 1;
        let old_file = delta.old_file().path().unwrap_or(std::path::Path::new(""));
        let new_file = delta.new_file().path().unwrap_or(std::path::Path::new(""));
        let status = delta.status();

        println!(
            "  File {}: {} -> {} (status: {:?})",
            file_count,
            old_file.display(),
            new_file.display(),
            status
        );
    }

    if file_count == 0 {
        println!("  No changes between {} and {}", old_label, new_label);
    }
}

/// Timer utility for measuring execution time
///
/// Provides a clean API for timing operations with automatic printing.
/// Useful for performance debugging and optimization.
///
/// # Example
///
/// ```
/// let end = Timer::default().start("my operation");
/// // ... do work ...
/// end(); // prints "[timer] my operation took 123ms"
///
/// // Or capture the duration:
/// let duration = end();
///
/// // For quiet timing without logging:
/// let end = Timer::default().start_quiet("background task");
/// let duration = end(); // just returns duration, no printing
/// ```
pub struct Timer {
    enabled: bool,
    pub epoch: Instant,
}

impl Timer {
    /// Create a new Timer instance
    pub fn new() -> Self {
        Timer {
            epoch: Instant::now(),
            enabled: cfg!(debug_assertions) || std::env::var("GIT_AI_PROFILE").is_ok(),
        }
    }

    /// Start timing an operation
    ///
    /// Returns a closure that when called will print the elapsed time and return the duration.
    ///
    /// # Arguments
    ///
    /// * `label` - A descriptive label for this timing operation
    ///
    /// # Returns
    ///
    /// A closure that prints the elapsed time and returns a `Duration`
    pub fn start(self, label: &str) -> impl FnOnce() -> Duration {
        let start_time = Instant::now();
        let enabled = self.enabled;
        let label = label.to_string();

        move || {
            let duration = start_time.elapsed();
            if enabled {
                self.print_duration(&label, duration);
            }
            duration
        }
    }

    pub fn print_duration(self, label: &str, duration: Duration) {
        println!(
            "\x1b[1;33m[timer]\x1b[0m {} {:?}ms",
            label,
            duration.as_millis()
        );
    }

    /// Start timing an operation quietly
    ///
    /// Returns a closure that when called will return the duration without printing.
    /// Useful when you want to measure time but control logging yourself.
    ///
    /// # Arguments
    ///
    /// * `_label` - A descriptive label (unused, kept for API consistency)
    ///
    /// # Returns
    ///
    /// A closure that returns a `Duration` without printing
    pub fn start_quiet(self, _label: &str) -> impl FnOnce() -> Duration {
        let start_time = Instant::now();

        move || start_time.elapsed()
    }
}

impl Default for Timer {
    fn default() -> Self {
        Self::new()
    }
}
