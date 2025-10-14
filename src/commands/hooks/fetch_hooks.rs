use crate::git::cli_parser::{ParsedGitInvocation, is_dry_run};
use crate::git::find_repository;
use crate::git::refs::{copy_ref, merge_notes_from_ref, ref_exists, tracking_ref_for_remote};
use crate::git::repository::{Repository, exec_git};
use crate::git::sync_authorship::{fetch_authorship_notes, fetch_remote_from_args};
use crate::utils::debug_log;

pub fn fetch_pull_post_command_hook(
    repository: &Repository,
    parsed_args: &ParsedGitInvocation,
    exit_status: std::process::ExitStatus,
) {
    if is_dry_run(&parsed_args.command_args) || !exit_status.success() {
        return;
    }

    let remote = fetch_remote_from_args(repository, parsed_args).ok();
    _ = fetch_authorship_notes(repository, parsed_args, remote.unwrap().as_str());
}
