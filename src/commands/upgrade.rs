use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};
use std::process::Command;

const GITHUB_REPO: &str = "acunniffe/git-ai";
const UPDATE_CHECK_INTERVAL_HOURS: u64 = 24;
const INSTALL_SCRIPT_URL: &str = "https://raw.githubusercontent.com/acunniffe/git-ai/main/install.sh";

#[derive(Debug, PartialEq)]
enum UpgradeAction {
    UpgradeAvailable,
    AlreadyLatest,
    RunningNewerVersion,
    ForceReinstall,
}

fn get_update_check_cache_path() -> Option<PathBuf> {
    #[cfg(test)]
    {
        if let Ok(test_cache_dir) = std::env::var("GIT_AI_TEST_CACHE_DIR") {
            return Some(PathBuf::from(test_cache_dir).join(".update_check"));
        }
    }

    dirs::home_dir().map(|home| home.join(".git-ai").join(".update_check"))
}

fn should_check_for_updates() -> bool {
    let cache_path = match get_update_check_cache_path() {
        Some(path) => path,
        None => return true,
    };

    if !cache_path.exists() {
        return true;
    }

    let metadata = match fs::metadata(&cache_path) {
        Ok(m) => m,
        Err(_) => return true,
    };

    let modified = match metadata.modified() {
        Ok(m) => m,
        Err(_) => return true,
    };

    let elapsed = SystemTime::now()
        .duration_since(modified)
        .unwrap_or(Duration::from_secs(0));

    elapsed.as_secs() > UPDATE_CHECK_INTERVAL_HOURS * 3600
}

fn update_check_cache() {
    if let Some(cache_path) = get_update_check_cache_path() {
        if let Some(parent) = cache_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(&cache_path, "");
    }
}

fn is_newer_version(latest: &str, current: &str) -> bool {
    let parse_version = |v: &str| -> Vec<u32> {
        v.split('.')
            .filter_map(|s| s.parse::<u32>().ok())
            .collect()
    };

    let latest_parts = parse_version(latest);
    let current_parts = parse_version(current);

    for i in 0..latest_parts.len().max(current_parts.len()) {
        let latest_part = latest_parts.get(i).copied().unwrap_or(0);
        let current_part = current_parts.get(i).copied().unwrap_or(0);

        if latest_part > current_part {
            return true;
        } else if latest_part < current_part {
            return false;
        }
    }

    false
}

pub fn run_with_args(args: &[String]) {
    let mut force = false;

    for arg in args {
        match arg.as_str() {
            "--force" => force = true,
            _ => {
                eprintln!("Unknown argument: {}", arg);
                eprintln!("Usage: git-ai upgrade [--force]");
                std::process::exit(1);
            }
        }
    }

    run_impl(force);
}

fn run_impl(force: bool) {
    let _ = run_impl_with_url(force, None);
}

fn run_impl_with_url(force: bool, api_base_url: Option<&str>) -> UpgradeAction {
    let current_version = env!("CARGO_PKG_VERSION");

    println!("Checking for updates...");

    let url = if let Some(base_url) = api_base_url {
        format!("{}/repos/{}/releases/latest", base_url, GITHUB_REPO)
    } else {
        format!(
            "https://api.github.com/repos/{}/releases/latest",
            GITHUB_REPO
        )
    };

    let response = match ureq::get(&url)
        .set("User-Agent", &format!("git-ai/{}", current_version))
        .timeout(std::time::Duration::from_secs(5))
        .call()
    {
        Ok(resp) => resp,
        Err(e) => {
            eprintln!("Failed to check for updates: {}", e);
            std::process::exit(1);
        }
    };

    let json: serde_json::Value = match response.into_json() {
        Ok(j) => j,
        Err(e) => {
            eprintln!("Failed to parse GitHub API response: {}", e);
            std::process::exit(1);
        }
    };

    let latest_version = match json["tag_name"].as_str() {
        Some(v) => v.trim_start_matches('v'),
        None => {
            eprintln!("Failed to get version from GitHub API response");
            std::process::exit(1);
        }
    };

    update_check_cache();

    println!("Current version: v{}", current_version);
    println!("Latest version:  v{}", latest_version);
    println!();

    let action = if force {
        UpgradeAction::ForceReinstall
    } else if latest_version == current_version {
        UpgradeAction::AlreadyLatest
    } else if is_newer_version(latest_version, current_version) {
        UpgradeAction::UpgradeAvailable
    } else {
        UpgradeAction::RunningNewerVersion
    };

    match action {
        UpgradeAction::AlreadyLatest => {
            println!("You are already on the latest version!");
            println!();
            println!("To reinstall anyway, run:");
            println!("  \x1b[1;36mgit-ai upgrade --force\x1b[0m");
            return action;
        }
        UpgradeAction::RunningNewerVersion => {
            println!("You are running a newer version than the latest release.");
            println!("(This usually means you're running a development build)");
            println!();
            println!("To reinstall the latest release version anyway, run:");
            println!("  \x1b[1;36mgit-ai upgrade --force\x1b[0m");
            return action;
        }
        UpgradeAction::ForceReinstall => {
            println!("\x1b[1;33mForce mode enabled - reinstalling v{}\x1b[0m", latest_version);
        }
        UpgradeAction::UpgradeAvailable => {
            println!("\x1b[1;33mA new version is available!\x1b[0m");
        }
        _ => {}
    }
    println!();

    // Skip installation if api_base_url is provided (test mode)
    if api_base_url.is_some() {
        return action;
    }

    println!("Running installation script...");
    println!();

    // Run the install script via curl | bash
    let status = Command::new("bash")
        .arg("-c")
        .arg(format!("curl -fsSL {} | bash", INSTALL_SCRIPT_URL))
        .status();

    match status {
        Ok(exit_status) => {
            if exit_status.success() {
                println!();
                println!("\x1b[1;32m✓\x1b[0m Successfully installed v{}!", latest_version);
            } else {
                eprintln!();
                eprintln!("Installation script failed with exit code: {:?}", exit_status.code());
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("Failed to run installation script: {}", e);
            std::process::exit(1);
        }
    }

    action
}

pub fn check_for_updates() {
    check_for_updates_with_url(None);
}

fn check_for_updates_with_url(api_base_url: Option<&str>) {
    if !should_check_for_updates() {
        return;
    }

    let current_version = env!("CARGO_PKG_VERSION");

    let url = if let Some(base_url) = api_base_url {
        format!("{}/repos/{}/releases/latest", base_url, GITHUB_REPO)
    } else {
        format!(
            "https://api.github.com/repos/{}/releases/latest",
            GITHUB_REPO
        )
    };

    let response = match ureq::get(&url)
        .set("User-Agent", &format!("git-ai/{}", current_version))
        .timeout(std::time::Duration::from_secs(3))
        .call()
    {
        Ok(resp) => resp,
        Err(_) => {
            return;
        }
    };

    let json: serde_json::Value = match response.into_json() {
        Ok(j) => j,
        Err(_) => {
            return;
        }
    };

    let latest_version = match json["tag_name"].as_str() {
        Some(v) => v.trim_start_matches('v'),
        None => {
            return;
        }
    };

    update_check_cache();

    if latest_version != current_version && is_newer_version(latest_version, current_version) {
        eprintln!();
        eprintln!(
            "\x1b[1;33mA new version of git-ai is available: \x1b[1;32mv{}\x1b[0m → \x1b[1;32mv{}\x1b[0m",
            current_version, latest_version
        );
        eprintln!(
            "\x1b[1;33mRun \x1b[1;36mgit-ai upgrade\x1b[0m \x1b[1;33mto upgrade to the latest version.\x1b[0m"
        );
        eprintln!();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_newer_version() {
        assert!(!is_newer_version("1.0.0", "1.0.0"));
        assert!(!is_newer_version("1.0.10", "1.0.10"));

        assert!(is_newer_version("1.0.1", "1.0.0"));
        assert!(is_newer_version("1.0.11", "1.0.10"));
        assert!(!is_newer_version("1.0.0", "1.0.1"));
        assert!(!is_newer_version("1.0.10", "1.0.11"));

        assert!(is_newer_version("1.1.0", "1.0.0"));
        assert!(!is_newer_version("1.0.0", "1.1.0"));

        assert!(is_newer_version("2.0.0", "1.0.0"));
        assert!(is_newer_version("2.0.0", "1.9.9"));
        assert!(!is_newer_version("1.9.9", "2.0.0"));

        assert!(is_newer_version("1.0.0.1", "1.0.0"));
        assert!(!is_newer_version("1.0.0", "1.0.0.1"));

        assert!(is_newer_version("1.10.0", "1.9.0"));
        assert!(is_newer_version("1.0.100", "1.0.99"));
        assert!(is_newer_version("100.200.300", "100.200.299"));
    }

    #[test]
    fn test_run_impl_with_url() {
        let _temp_dir = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var("GIT_AI_TEST_CACHE_DIR", _temp_dir.path());
        }

        let mut server = mockito::Server::new();

        // Newer version available - should upgrade
        let mock = server
            .mock("GET", "/repos/acunniffe/git-ai/releases/latest")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"tag_name": "v999.0.0"}"#)
            .create();
        let action = run_impl_with_url(false, Some(&server.url()));
        assert_eq!(action, UpgradeAction::UpgradeAvailable);
        mock.assert();

        // Same version without --force - already latest
        let mock = server
            .mock("GET", "/repos/acunniffe/git-ai/releases/latest")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"tag_name": "v1.0.10"}"#)
            .create();
        let action = run_impl_with_url(false, Some(&server.url()));
        assert_eq!(action, UpgradeAction::AlreadyLatest);
        mock.assert();

        // Same version with --force - force reinstall
        let mock = server
            .mock("GET", "/repos/acunniffe/git-ai/releases/latest")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"tag_name": "v1.0.10"}"#)
            .create();
        let action = run_impl_with_url(true, Some(&server.url()));
        assert_eq!(action, UpgradeAction::ForceReinstall);
        mock.assert();

        // Older version without --force - running newer version
        let mock = server
            .mock("GET", "/repos/acunniffe/git-ai/releases/latest")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"tag_name": "v1.0.9"}"#)
            .create();
        let action = run_impl_with_url(false, Some(&server.url()));
        assert_eq!(action, UpgradeAction::RunningNewerVersion);
        mock.assert();

        // Older version with --force - force reinstall
        let mock = server
            .mock("GET", "/repos/acunniffe/git-ai/releases/latest")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"tag_name": "v1.0.9"}"#)
            .create();
        let action = run_impl_with_url(true, Some(&server.url()));
        assert_eq!(action, UpgradeAction::ForceReinstall);
        mock.assert();

        unsafe {
            std::env::remove_var("GIT_AI_TEST_CACHE_DIR");
        }
    }

    #[test]
    fn test_check_for_updates() {
        let temp_dir = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var("GIT_AI_TEST_CACHE_DIR", temp_dir.path());
        }

        let mut server = mockito::Server::new();

        let mock = server
            .mock("GET", "/repos/acunniffe/git-ai/releases/latest")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"tag_name": "v999.0.0"}"#)
            .expect(1)  // Expect exactly 1 call total
            .create();

        // No cache exists - should make API call
        check_for_updates_with_url(Some(&server.url()));
        mock.assert();

        // Cache exists - should not make API call
        check_for_updates_with_url(Some(&server.url()));
        mock.assert();

        unsafe {
            std::env::remove_var("GIT_AI_TEST_CACHE_DIR");
        }
    }
}