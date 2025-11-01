use std::{ops::Add, time::Duration};

use serde_json::json;

use crate::{
    authorship::working_log::CheckpointKind, observability::log_performance, utils::debug_log,
};

pub fn log_performance_target_if_violated(
    command: &str,
    pre_command: Duration,
    git_duration: Duration,
    post_command: Duration,
) {
    let total_duration = pre_command + git_duration + post_command;
    let within_target: bool = match command {
        "commit" => git_duration.mul_f32(1.1) >= total_duration,
        "rebase" => git_duration.mul_f32(1.1) >= total_duration,
        "cherry-pick" => git_duration.mul_f32(1.1) >= total_duration,
        "reset" => git_duration.mul_f32(1.1) >= total_duration,
        "fetch" => git_duration.mul_f32(1.5) >= total_duration,
        "pull" => git_duration.mul_f32(1.5) >= total_duration,
        "push" => git_duration.mul_f32(1.5) >= total_duration,
        _ => git_duration.add(Duration::from_millis(100)) >= total_duration,
    };

    if !within_target {
        debug_log(&format!(
            "ᕽ Performance target violated for command: {}. Total duration: {}ms, Git duration: {}ms. Pre-command: {}ms, Post-command: {}ms.",
            command,
            total_duration.as_millis(),
            git_duration.as_millis(),
            pre_command.as_millis(),
            post_command.as_millis(),
        ));
        log_performance(
            "performance_target_violated",
            total_duration,
            Some(json!({
                "command": command,
                "total_duration": total_duration.as_millis(),
                "git_duration": git_duration.as_millis(),
                "pre_command": pre_command.as_millis(),
                "post_command": post_command.as_millis(),
            })),
        );
    } else {
        debug_log(&format!(
            "✓ Performance target met for command: {}. Total duration: {}ms, Git duration: {}ms",
            command,
            total_duration.as_millis(),
            git_duration.as_millis(),
        ));
    }
}

pub fn log_performance_for_checkpoint(
    files_edited: usize,
    duration: Duration,
    checkpoint_kind: CheckpointKind,
) {
    if Duration::from_millis(50 * files_edited as u64) >= duration {
        log_performance(
            "checkpoint",
            duration,
            Some(json!({
                "files_edited": files_edited,
                "checkpoint_kind": checkpoint_kind.to_string(),
                "duration": duration.as_millis(),
            })),
        );

        debug_log(&format!(
            "ᕽ Performance target violated for checkpoint: {}. Total duration. Files edited: {}",
            duration.as_millis(),
            files_edited,
        ));
    } else {
        debug_log(&format!(
            "✓ Performance target met for checkpoint: {}. Total duration. Files edited: {}",
            duration.as_millis(),
            files_edited,
        ));
    }
}
