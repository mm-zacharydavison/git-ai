use crate::git::cli_parser::{ParsedGitInvocation, is_dry_run};
use crate::git::find_repository;
use crate::git::post_commit;
use crate::git::pre_commit;
use crate::git::repo_storage::RepoStorage;
use crate::git::rewrite_log::{CommitAmendEvent, RewriteLogEvent};

pub fn commit_pre_command_hook(parsed_args: &ParsedGitInvocation) {
    if is_dry_run(&parsed_args.command_args) {
        return;
    }

    // TODO Take global args into account
    // TODO Remove this once we migrate off of libgit2
    // Find the git repository
    let repo = match find_repository() {
        Ok(repo) => repo,
        Err(e) => {
            eprintln!("Failed to find repository: {}", e);
            std::process::exit(1);
        }
    };

    let default_user_name = get_commit_default_user_name(&repo, &parsed_args.command_args);

    // Run pre-commit logic
    if let Err(e) = pre_commit::pre_commit(&repo, default_user_name.clone()) {
        eprintln!("Pre-commit failed: {}", e);
        std::process::exit(1);
    }
}

pub fn commit_post_command_hook(
    parsed_args: &ParsedGitInvocation,
    exit_status: std::process::ExitStatus,
) {
    if is_dry_run(&parsed_args.command_args) {
        return;
    }

    if !exit_status.success() {
        return;
    }

    // TODO Take global args into account
    // TODO Remove this once we migrate off of libgit2
    // Find the git repository
    let repo = match find_repository() {
        Ok(repo) => repo,
        Err(e) => {
            eprintln!("Failed to find repository: {}", e);
            std::process::exit(1);
        }
    };

    if let Err(e) = post_commit::post_commit(&repo, false) {
        eprintln!("Post-commit failed: {}", e);
    }

    let amended_commit_sha = repo.head().unwrap().target().unwrap().to_string();

    let repo_storage = RepoStorage::for_repo_path(repo.path());
    repo_storage
        .append_rewrite_event(RewriteLogEvent::CommitAmend {
            commit_amend: CommitAmendEvent {
                original_commit: parsed_args.command_args[0].clone(),
                amended_commit_sha,
                success: true,         // success - assuming it will succeed
                changed_files: vec![], // changed_files - could be populated from git diff
            },
        })
        .unwrap();
}

fn get_commit_default_user_name(repo: &git2::Repository, args: &[String]) -> String {
    // According to git commit manual, --author flag overrides all other author information
    if let Some(author_spec) = extract_author_from_args(args) {
        return resolve_author_spec(repo, &author_spec);
    }

    // Normal precedence when --author is not specified:
    // 1. GIT_AUTHOR_NAME environment variable
    // 2. user.name config variable
    // 3. EMAIL environment variable
    // 4. System user name and hostname (we'll use 'unknown' as fallback)

    // Check GIT_AUTHOR_NAME environment variable
    if let Ok(author_name) = std::env::var("GIT_AUTHOR_NAME") {
        if !author_name.trim().is_empty() {
            return author_name.trim().to_string();
        }
    }

    // Fall back to git config user.name
    if let Ok(config) = repo.config() {
        if let Ok(name) = config.get_string("user.name") {
            if !name.trim().is_empty() {
                return name.trim().to_string();
            }
        }
    }

    // Check EMAIL environment variable as fallback
    if let Ok(email) = std::env::var("EMAIL") {
        if !email.trim().is_empty() {
            // Extract name part from email if it contains a name
            if let Some(at_pos) = email.find('@') {
                let name_part = &email[..at_pos];
                if !name_part.is_empty() {
                    return name_part.to_string();
                }
            }
            return email;
        }
    }

    // Final fallback (instead of trying to get system user name and hostname)
    eprintln!("Warning: No author information found. Using 'unknown' as author.");
    "unknown".to_string()
}

fn extract_author_from_args(args: &[String]) -> Option<String> {
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];

        // Handle --author=<author> format
        if let Some(author_value) = arg.strip_prefix("--author=") {
            return Some(author_value.to_string());
        }

        // Handle --author <author> format (separate arguments)
        if arg == "--author" && i + 1 < args.len() {
            return Some(args[i + 1].clone());
        }

        i += 1;
    }
    None
}

fn resolve_author_spec(repo: &git2::Repository, author_spec: &str) -> String {
    // According to git commit docs, --author can be:
    // 1. "A U Thor <author@example.com>" format - use as explicit author
    // 2. A pattern to search for existing commits via git rev-list --all -i --author=<pattern>

    // If it looks like "Name <email>" format, extract the name part
    if let Some(email_start) = author_spec.rfind('<') {
        let name_part = author_spec[..email_start].trim();
        if !name_part.is_empty() {
            return name_part.to_string();
        }
    }

    // If it doesn't look like an explicit format, treat it as a search pattern
    // Try to find an existing commit by that author
    if let Ok(mut revwalk) = repo.revwalk() {
        if revwalk.push_glob("refs/*").is_ok() {
            for oid_result in revwalk {
                if let Ok(oid) = oid_result {
                    if let Ok(commit) = repo.find_commit(oid) {
                        let author = commit.author();
                        if let Some(author_name) = author.name() {
                            // Case-insensitive search (like git rev-list -i --author)
                            if author_name
                                .to_lowercase()
                                .contains(&author_spec.to_lowercase())
                            {
                                return author_name.to_string();
                            }
                        }
                    }
                }
            }
        }
    }

    // If no matching commit found, use the pattern as-is
    author_spec.trim().to_string()
}
