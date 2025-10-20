use crate::git::cli_parser::{ParsedGitInvocation, is_dry_run};
use crate::git::repository::Repository;
use crate::git::sync_authorship::fetch_remote_from_args;

pub fn fetch_pull_post_command_hook(
    repository: &Repository,
    parsed_args: &ParsedGitInvocation,
    exit_status: std::process::ExitStatus,
) {
    if is_dry_run(&parsed_args.command_args) || !exit_status.success() {
        return;
    }

    let remote = fetch_remote_from_args(repository, parsed_args).ok();
    let _ = repository.fetch_authorship(remote.unwrap().as_str());
}
