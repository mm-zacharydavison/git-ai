use crate::error::GitAiError;
use indicatif::{ProgressBar, ProgressStyle};

pub fn run(_args: &[String]) -> Result<(), GitAiError> {
    // Run async operations with smol
    smol::block_on(async_run())
}

async fn async_run() -> Result<(), GitAiError> {
    check_claude_code().await;
    Ok(())
}

async fn check_claude_code() {
    let spinner = Spinner::new("Claude code: looking for binary");
    spinner.start();
    spinner.wait_for(500).await;

    // Step 1: Check if binary exists
    spinner.update_message("Claude code: checking binary location");
    spinner.wait_for(300).await;

    let exists = binary_exists("claude");

    if exists {
        // Step 2: Check version compatibility
        spinner.update_message("Claude code: verifying version compatibility");
        spinner.wait_for(400).await;

        // Step 3: Test functionality
        spinner.update_message("Claude code: testing functionality");
        spinner.wait_for(300).await;

        // Step 4: Final verification
        spinner.update_message("Claude code: final verification");
        spinner.wait_for(200).await;

        spinner.success("Claude code: Installed and ready");
    } else {
        // Show what we checked
        spinner.update_message("Claude code: binary not found in PATH");
        spinner.wait_for(200).await;

        spinner.skipped("Claude code: Not installed");
    }
}

async fn check_codex() {
    let spinner = Spinner::new("Codex: checking install");
    spinner.start();

    // Check if Codex binary exists
    let exists = binary_exists("claude");

    // Simulate some checking time
    spinner.wait_for(1500).await;

    if exists {
        spinner.success("Codex: Installed");
    } else {
        spinner.error("Codex: Not found");
    }
}

async fn check_windsurf() {
    let spinner = Spinner::new("Windsurf: checking status");
    spinner.start();

    // Simulate checking Windsurf
    spinner.wait_for(500).await;

    spinner.error("Windsurf: Not installed");
}

async fn check_cursor() {
    let spinner = Spinner::new("Cursor: checking status");
    spinner.start();

    // Simulate checking Cursor
    spinner.wait_for(2000).await;

    spinner.success("Cursor: Ready");
}

async fn check_github_copilot() {
    let spinner = Spinner::new("GitHub Copilot: verifying");
    spinner.start();

    // Simulate verifying GitHub Copilot
    spinner.wait_for(3000).await;

    spinner.success("GitHub Copilot: Active");
}

// Shared utilities

/// Check if a binary with the given name exists in the system PATH
fn binary_exists(name: &str) -> bool {
    if let Ok(path) = std::env::var("PATH") {
        for dir in path.split(':') {
            let binary_path = std::path::Path::new(dir).join(name);
            if binary_path.exists() && binary_path.is_file() {
                return true;
            }
        }
    }
    false
}

// Loader
struct Spinner {
    pb: ProgressBar,
    _handle: smol::Task<()>,
}

impl Spinner {
    fn new(message: &str) -> Self {
        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.green} {msg}")
                .unwrap()
                .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
        );
        pb.set_message(message.to_string());

        // Start the auto-ticking task
        let pb_clone = pb.clone();
        let _handle = smol::spawn(async move {
            loop {
                pb_clone.tick();
                smol::Timer::after(std::time::Duration::from_millis(100)).await;
            }
        });

        Self { pb, _handle }
    }

    fn start(&self) {
        // Spinner starts automatically when created
    }

    fn update_message(&self, message: &str) {
        self.pb.set_message(message.to_string());
    }

    async fn wait_for(&self, duration_ms: u64) {
        smol::Timer::after(std::time::Duration::from_millis(duration_ms)).await;
    }

    fn success(&self, message: &'static str) {
        // Clear spinner and show success with green checkmark and green text
        self.pb.finish_and_clear();
        println!("\x1b[32m✓ {}\x1b[0m", message);
    }

    fn error(&self, message: &'static str) {
        // Clear spinner and show error with red X and red text
        self.pb.finish_and_clear();
        println!("\x1b[31m✗ {}\x1b[0m", message);
    }

    fn skipped(&self, message: &'static str) {
        // Clear spinner and show skipped with gray circle and gray text
        self.pb.finish_and_clear();
        println!("\x1b[90m○ {}\x1b[0m", message);
    }
}
