use crate::commands::commit_hooks;
use crate::commands::commit_hooks::get_commit_default_author;
use crate::commands::fetch_hooks;
use crate::commands::push_hooks;
use crate::config;
use crate::git::cli_parser::is_dry_run;
use crate::git::cli_parser::{ParsedGitInvocation, parse_git_cli_args};
use crate::git::find_repository;
use crate::git::repository::Repository;
use crate::git::rewrite_log::MergeSquashEvent;
use crate::git::rewrite_log::RewriteLogEvent;
use crate::utils::debug_log;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;
use std::process::Command;
#[cfg(unix)]
use std::sync::atomic::{AtomicI32, Ordering};

#[cfg(unix)]
static CHILD_PGID: AtomicI32 = AtomicI32::new(0);

#[cfg(unix)]
extern "C" fn forward_signal_handler(sig: libc::c_int) {
    let pgid = CHILD_PGID.load(Ordering::Relaxed);
    if pgid > 0 {
        unsafe {
            // Send to the whole child process group
            let _ = libc::kill(-pgid, sig);
        }
    }
}

#[cfg(unix)]
fn install_forwarding_handlers() {
    unsafe {
        let handler = forward_signal_handler as usize;
        let _ = libc::signal(libc::SIGTERM, handler);
        let _ = libc::signal(libc::SIGINT, handler);
        let _ = libc::signal(libc::SIGHUP, handler);
        let _ = libc::signal(libc::SIGQUIT, handler);
    }
}

#[cfg(unix)]
fn uninstall_forwarding_handlers() {
    unsafe {
        let _ = libc::signal(libc::SIGTERM, libc::SIG_DFL);
        let _ = libc::signal(libc::SIGINT, libc::SIG_DFL);
        let _ = libc::signal(libc::SIGHUP, libc::SIG_DFL);
        let _ = libc::signal(libc::SIGQUIT, libc::SIG_DFL);
    }
}

struct CommandHooksContext {
    pre_commit_hook_result: Option<bool>,
    rebase_original_head: Option<String>,
    _rebase_onto: Option<String>,
}

pub fn handle_git(args: &[String]) {
    // If we're being invoked from a shell completion context, bypass git-ai logic
    // and delegate directly to the real git so existing completion scripts work.
    if in_shell_completion_context() {
        let orig_args: Vec<String> = std::env::args().skip(1).collect();
        proxy_to_git(&orig_args, true);
        return;
    }

    let mut command_hooks_context = CommandHooksContext {
        pre_commit_hook_result: None,
        rebase_original_head: None,
        _rebase_onto: None,
    };

    let parsed_args = parse_git_cli_args(args);

    let mut repository_option = find_repository(&parsed_args.global_args).ok();

    let has_repo = repository_option.is_some();

    // println!("command: {:?}", parsed_args.command);
    // println!("global_args: {:?}", parsed_args.global_args);
    // println!("command_args: {:?}", parsed_args.command_args);
    // println!("to_invocation_vec: {:?}", parsed_args.to_invocation_vec());

    let config = config::Config::get();

    let skip_hooks = !config.is_allowed_repository(&repository_option);
    if skip_hooks {
        debug_log(
            "Skipping git-ai hooks because repository does not have at least one remote in allow_repositories list",
        );
    }

    // run with hooks
    let exit_status = if !parsed_args.is_help && has_repo && !skip_hooks {
        let repository = repository_option.as_mut().unwrap();
        run_pre_command_hooks(&mut command_hooks_context, &parsed_args, repository);
        let exit_status = proxy_to_git(&parsed_args.to_invocation_vec(), false);

        run_post_command_hooks(
            &mut command_hooks_context,
            &parsed_args,
            exit_status,
            repository,
        );
        exit_status
    } else {
        // run without hooks
        proxy_to_git(&parsed_args.to_invocation_vec(), false)
    };
    exit_with_status(exit_status);
}

fn run_pre_command_hooks(
    command_hooks_context: &mut CommandHooksContext,
    parsed_args: &ParsedGitInvocation,
    repository: &mut Repository,
) {
    // Pre-command hooks
    match parsed_args.command.as_deref() {
        Some("commit") => {
            command_hooks_context.pre_commit_hook_result = Some(
                commit_hooks::commit_pre_command_hook(parsed_args, repository),
            );
        }
        Some("rebase") => {
            // Check if we're continuing an existing rebase or starting a new one
            let rebase_dir = repository.path().join("rebase-merge");
            let rebase_apply_dir = repository.path().join("rebase-apply");
            let rebase_in_progress = rebase_dir.exists() || rebase_apply_dir.exists();

            // Check if there's an active Start event in the log that matches
            let is_continuing = rebase_in_progress && has_active_rebase_start_event(repository);

            if !is_continuing {
                // Starting a new rebase - capture original HEAD and log Start event
                if let Ok(head) = repository.head() {
                    if let Ok(target) = head.target() {
                        command_hooks_context.rebase_original_head = Some(target.clone());

                        // Determine if interactive
                        let is_interactive = parsed_args.has_command_flag("-i")
                            || parsed_args.has_command_flag("--interactive");

                        // Log the rebase start event
                        let start_event = RewriteLogEvent::rebase_start(
                            crate::git::rewrite_log::RebaseStartEvent::new(
                                target.clone(),
                                is_interactive,
                            ),
                        );

                        // Write to rewrite log
                        let _ = repository.storage.append_rewrite_event(start_event);
                    }
                }
            }
            // If continuing, we'll read the original head from the existing Start event in post-hook
        }
        _ => {}
    }
}

fn run_post_command_hooks(
    command_hooks_context: &mut CommandHooksContext,
    parsed_args: &ParsedGitInvocation,
    exit_status: std::process::ExitStatus,
    repository: &mut Repository,
) {
    // Post-command hooks
    match parsed_args.command.as_deref() {
        Some("commit") => {
            if let Some(pre_commit_hook_result) = command_hooks_context.pre_commit_hook_result {
                if !pre_commit_hook_result {
                    debug_log("Skipping git-ai post-commit hook because pre-commit hook failed");
                    return;
                }
            }
            let supress_output = parsed_args.has_command_flag("--porcelain")
                || parsed_args.has_command_flag("--quiet")
                || parsed_args.has_command_flag("-q")
                || parsed_args.has_command_flag("--no-status");

            commit_hooks::commit_post_command_hook(
                parsed_args,
                exit_status,
                repository,
                supress_output,
            );
        }
        Some("fetch") => fetch_hooks::fetch_post_command_hook(parsed_args, exit_status),
        Some("push") => push_hooks::push_post_command_hook(parsed_args, exit_status),
        Some("reset") => {
            if parsed_args.has_command_flag("--hard") {
                let base_head = repository.head().unwrap().target().unwrap().to_string();
                let _ = repository
                    .storage
                    .delete_working_log_for_base_commit(&base_head);

                debug_log(&format!(
                    "Reset --hard: deleted working log for {}",
                    base_head
                ));
            }
            // soft and mixed coming soon
        }
        Some("merge") => {
            if parsed_args.has_command_flag("--squash")
                && exit_status.success()
                && !is_dry_run(&parsed_args.command_args)
            {
                let base_branch = repository.head().unwrap().name().unwrap().to_string();
                let base_head = repository.head().unwrap().target().unwrap().to_string();

                let commit_author =
                    get_commit_default_author(&repository, &parsed_args.command_args);

                let source_branch = parsed_args.pos_command(0).unwrap();

                let source_head_sha = match repository
                    .revparse_single(source_branch.as_str())
                    .and_then(|obj| obj.peel_to_commit())
                {
                    Ok(commit) => commit.id(),
                    Err(_) => {
                        // If we can't resolve the branch, skip logging this event
                        return;
                    }
                };

                // println!("source_head_sha: {}", source_head_sha);
                // println!("source_branch: {}", source_branch);

                // println!("base_branch: {}", base_branch);
                // println!("base_sha: {}", base_head);

                repository.handle_rewrite_log_event(
                    RewriteLogEvent::merge_squash(MergeSquashEvent::new(
                        source_branch.clone(),
                        source_head_sha,
                        base_branch,
                        base_head,
                    )),
                    commit_author,
                    false,
                    true,
                );
            }
        }
        Some("rebase") => {
            handle_rebase_post_command(command_hooks_context, parsed_args, exit_status, repository);
        }
        _ => {}
    }
}

fn handle_rebase_post_command(
    context: &CommandHooksContext,
    parsed_args: &ParsedGitInvocation,
    exit_status: std::process::ExitStatus,
    repository: &mut Repository,
) {
    // Check if rebase is still in progress
    let rebase_dir = repository.path().join("rebase-merge");
    let rebase_apply_dir = repository.path().join("rebase-apply");
    let is_in_progress = rebase_dir.exists() || rebase_apply_dir.exists();

    if is_in_progress {
        // Rebase still in progress (conflict or not finished)
        debug_log("Rebase in progress, waiting for completion");
        return;
    }

    if is_dry_run(&parsed_args.command_args) {
        return;
    }

    // Rebase is done (completed or aborted)
    // Try to find the original head from context OR from the rewrite log
    let original_head = context.rebase_original_head.clone().or_else(|| {
        // Check the rewrite log for a Start event
        find_rebase_start_event_original_head(repository)
    });

    if !exit_status.success() {
        // Rebase was aborted or failed - log Abort event
        if let Some(orig_head) = original_head {
            debug_log(&format!("Rebase aborted from {}", orig_head));
            let abort_event = RewriteLogEvent::rebase_abort(
                crate::git::rewrite_log::RebaseAbortEvent::new(orig_head),
            );
            let _ = repository.storage.append_rewrite_event(abort_event);
        }
        return;
    }

    // Rebase completed successfully!
    if let Some(original_head) = original_head {
        process_completed_rebase(repository, &original_head, parsed_args);
    } else {
        debug_log("Rebase completed but couldn't determine original head");
    }
}

/// Check if there's an active rebase Start event (not followed by Complete or Abort)
fn has_active_rebase_start_event(repository: &Repository) -> bool {
    let events = match repository.storage.read_rewrite_events() {
        Ok(events) => events,
        Err(_) => return false,
    };

    // Events are newest-first
    // If we find Complete or Abort before Start, there's no active rebase
    // If we find Start before Complete/Abort, there's an active rebase
    for event in events {
        match event {
            RewriteLogEvent::RebaseComplete { .. } | RewriteLogEvent::RebaseAbort { .. } => {
                return false; // Found completion/abort first, no active rebase
            }
            RewriteLogEvent::RebaseStart { .. } => {
                return true; // Found start first, active rebase
            }
            _ => continue,
        }
    }

    false // No rebase events found
}

/// Find the original head from the most recent Rebase Start event in the log
fn find_rebase_start_event_original_head(repository: &Repository) -> Option<String> {
    let events = repository.storage.read_rewrite_events().ok()?;

    // Find the most recent Start event (events are newest-first)
    for event in events {
        match event {
            RewriteLogEvent::RebaseStart { rebase_start } => {
                return Some(rebase_start.original_head.clone());
            }
            _ => continue,
        }
    }

    None
}

fn process_completed_rebase(
    repository: &mut Repository,
    original_head: &str,
    parsed_args: &ParsedGitInvocation,
) {
    // Get the new HEAD
    let new_head = match repository.head() {
        Ok(head) => match head.target() {
            Ok(target) => target,
            Err(_) => return,
        },
        Err(_) => return,
    };

    // If HEAD didn't change, nothing to do
    if original_head == new_head {
        debug_log("Rebase resulted in no changes (fast-forward or empty)");
        return;
    }

    // Build commit mappings
    let (original_commits, new_commits) =
        match build_rebase_commit_mappings(repository, original_head, &new_head) {
            Ok(mappings) => mappings,
            Err(e) => {
                debug_log(&format!("Failed to build rebase mappings: {}", e));
                return;
            }
        };

    if original_commits.is_empty() {
        debug_log("No commits to rewrite authorship for");
        return;
    }

    // Determine rebase type
    let is_interactive =
        parsed_args.has_command_flag("-i") || parsed_args.has_command_flag("--interactive");

    let rebase_event =
        RewriteLogEvent::rebase_complete(crate::git::rewrite_log::RebaseCompleteEvent::new(
            original_head.to_string(),
            new_head.clone(),
            is_interactive,
            original_commits.clone(),
            new_commits.clone(),
        ));

    let commit_author = get_commit_default_author(repository, &parsed_args.command_args);

    repository.handle_rewrite_log_event(
        rebase_event,
        commit_author,
        false, // don't suppress output
        true,  // save to log
    );
}

fn build_rebase_commit_mappings(
    repository: &Repository,
    original_head: &str,
    new_head: &str,
) -> Result<(Vec<String>, Vec<String>), crate::error::GitAiError> {
    // Get commits from new_head and original_head
    let new_head_commit = repository.find_commit(new_head.to_string())?;
    let original_head_commit = repository.find_commit(original_head.to_string())?;

    // Find merge base between original and new
    let merge_base = repository.merge_base(original_head_commit.id(), new_head_commit.id())?;

    // Walk from original_head to merge_base to get the commits that were rebased
    let original_commits = walk_commits_to_base(repository, original_head, &merge_base)?;

    // Walk from new_head to get the same number of commits (the rebased ones)
    let new_commits = walk_commits_from_head(repository, new_head, original_commits.len())?;

    // Reverse both so they're in chronological order (oldest first)
    let mut original_commits = original_commits;
    let mut new_commits = new_commits;
    original_commits.reverse();
    new_commits.reverse();

    Ok((original_commits, new_commits))
}

fn walk_commits_to_base(
    repository: &Repository,
    head: &str,
    base: &str,
) -> Result<Vec<String>, crate::error::GitAiError> {
    let mut commits = Vec::new();
    let mut current = repository.find_commit(head.to_string())?;
    let base_str = base.to_string();

    while current.id().to_string() != base_str {
        commits.push(current.id().to_string());
        current = current.parent(0)?;
    }

    Ok(commits)
}

fn walk_commits_from_head(
    repository: &Repository,
    head: &str,
    count: usize,
) -> Result<Vec<String>, crate::error::GitAiError> {
    let mut commits = Vec::new();
    let mut current = repository.find_commit(head.to_string())?;

    for _ in 0..count {
        commits.push(current.id().to_string());
        current = match current.parent(0) {
            Ok(parent) => parent,
            Err(_) => break,
        };
    }

    Ok(commits)
}

fn proxy_to_git(args: &[String], exit_on_completion: bool) -> std::process::ExitStatus {
    // debug_log(&format!("proxying to git with args: {:?}", args));
    // debug_log(&format!("prepended global args: {:?}", prepend_global(args)));
    // Use spawn for interactive commands
    let child = {
        #[cfg(unix)]
        {
            // Only create a new process group for non-interactive runs.
            // If stdin is a TTY, the child must remain in the foreground
            // terminal process group to avoid SIGTTIN/SIGTTOU hangs.
            let is_interactive = unsafe { libc::isatty(libc::STDIN_FILENO) == 1 };
            let should_setpgid = !is_interactive;

            let mut cmd = Command::new(config::Config::get().git_cmd());
            cmd.args(args);
            unsafe {
                let setpgid_flag = should_setpgid;
                cmd.pre_exec(move || {
                    if setpgid_flag {
                        // Make the child its own process group leader so we can signal the group
                        let _ = libc::setpgid(0, 0);
                    }
                    Ok(())
                });
            }
            // We return both the spawned child and whether we changed PGID
            match cmd.spawn() {
                Ok(child) => Ok((child, should_setpgid)),
                Err(e) => Err(e),
            }
        }
        #[cfg(not(unix))]
        {
            Command::new(config::Config::get().git_cmd())
                .args(args)
                .spawn()
        }
    };

    #[cfg(unix)]
    match child {
        Ok((mut child, setpgid)) => {
            #[cfg(unix)]
            {
                if setpgid {
                    // Record the child's process group id (same as its pid after setpgid)
                    let pgid: i32 = child.id() as i32;
                    CHILD_PGID.store(pgid, Ordering::Relaxed);
                    install_forwarding_handlers();
                }
            }
            let status = child.wait();
            match status {
                Ok(status) => {
                    #[cfg(unix)]
                    {
                        if setpgid {
                            CHILD_PGID.store(0, Ordering::Relaxed);
                            uninstall_forwarding_handlers();
                        }
                    }
                    if exit_on_completion {
                        exit_with_status(status);
                    }
                    return status;
                }
                Err(e) => {
                    #[cfg(unix)]
                    {
                        if setpgid {
                            CHILD_PGID.store(0, Ordering::Relaxed);
                            uninstall_forwarding_handlers();
                        }
                    }
                    eprintln!("Failed to wait for git process: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Err(e) => {
            eprintln!("Failed to execute git command: {}", e);
            std::process::exit(1);
        }
    }

    #[cfg(not(unix))]
    match child {
        Ok(mut child) => {
            let status = child.wait();
            match status {
                Ok(status) => {
                    if exit_on_completion {
                        exit_with_status(status);
                    }
                    return status;
                }
                Err(e) => {
                    eprintln!("Failed to wait for git process: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Err(e) => {
            eprintln!("Failed to execute git command: {}", e);
            std::process::exit(1);
        }
    }
}

// Exit mirroring the child's termination: same signal if signaled, else exit code
fn exit_with_status(status: std::process::ExitStatus) -> ! {
    #[cfg(unix)]
    {
        if let Some(sig) = status.signal() {
            unsafe {
                libc::signal(sig, libc::SIG_DFL);
                libc::raise(sig);
            }
            // Should not return
            unreachable!();
        }
    }
    std::process::exit(status.code().unwrap_or(1));
}

// Detect if current process invocation is coming from shell completion machinery
// (bash, zsh via bashcompinit). If so, we should proxy directly to the real git
// without any extra behavior that could interfere with completion scripts.
fn in_shell_completion_context() -> bool {
    std::env::var("COMP_LINE").is_ok()
        || std::env::var("COMP_POINT").is_ok()
        || std::env::var("COMP_TYPE").is_ok()
}
