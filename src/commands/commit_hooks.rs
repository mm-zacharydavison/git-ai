use crate::git::cli_parser::{ParsedGitInvocation, is_dry_run};
use crate::git::post_commit;
use crate::git::pre_commit;
use crate::git::repository::Repository;
use crate::git::rewrite_log::RewriteLogEvent;

pub fn commit_pre_command_hook(
    parsed_args: &ParsedGitInvocation,
    repository: &mut Repository,
) -> bool {
    if is_dry_run(&parsed_args.command_args) {
        return false;
    }

    // store HEAD context for post-command hook
    repository.require_pre_command_head();

    let default_author = get_commit_default_author(&repository, &parsed_args.command_args);

    // Run pre-commit logic
    if let Err(e) = pre_commit::pre_commit(&repository, default_author.clone()) {
        if e.to_string()
            .contains("Cannot run checkpoint on bare repositories")
        {
            eprintln!(
                "Cannot run checkpoint on bare repositories (skipping git-ai pre-commit hook)"
            );
            return false;
        }
        eprintln!("Pre-commit failed: {}", e);
        std::process::exit(1);
    }
    return true;
}

pub fn commit_post_command_hook(
    parsed_args: &ParsedGitInvocation,
    exit_status: std::process::ExitStatus,
    repository: &mut Repository,
) {
    if is_dry_run(&parsed_args.command_args) {
        return;
    }

    if !exit_status.success() {
        return;
    }

    if parsed_args.has_command_flag("--amend") {
        repository.handle_rewrite_log_event(RewriteLogEvent::commit_amend(
            repository.pre_command_base_commit.clone().unwrap(),
            repository.head().unwrap().target().unwrap(),
        ));
    } else {
        repository.handle_rewrite_log_event(RewriteLogEvent::commit(
            repository.pre_command_base_commit.clone().unwrap(),
            repository.pre_command_refname.clone().unwrap(),
        ));
    }

    if let Err(e) = post_commit::post_commit(&repository) {
        eprintln!("git-ai authorship log generation failed: {}", e);
    }
}

fn get_commit_default_author(repo: &Repository, args: &[String]) -> String {
    // According to git commit manual, --author flag overrides all other author information
    if let Some(author_spec) = extract_author_from_args(args) {
        if let Ok(Some(resolved_author)) = repo.resolve_author_spec(&author_spec) {
            if !resolved_author.trim().is_empty() {
                return resolved_author.trim().to_string();
            }
        }
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
    if let Ok(Some(name)) = repo.config_get_str("user.name") {
        if !name.trim().is_empty() {
            return name.trim().to_string();
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
