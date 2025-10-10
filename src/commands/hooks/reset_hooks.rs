use crate::{
    git::{cli_parser::ParsedGitInvocation, repository::Repository},
    utils::debug_log,
};

pub fn post_reset_hook(parsed_args: &ParsedGitInvocation, repository: &mut Repository) {
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
