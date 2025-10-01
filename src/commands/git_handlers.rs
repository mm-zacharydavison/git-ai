use crate::git::cli_parser::parse_git_cli_args;
use crate::config;
use std::process::Command;
#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;

pub fn handle_git(args: &[String]) {
    let parsed_args = parse_git_cli_args(args);
    // println!("command: {:?}", parsed_args.command);
    // println!("global_args: {:?}", parsed_args.global_args);
    // println!("command_args: {:?}", parsed_args.command_args);
    reset_sigpipe_to_default();
    let exit_status = _proxy_to_git(&parsed_args.to_invocation_vec(), true);
    exit_with_status(exit_status);
}

fn _proxy_to_git(args: &[String], exit_on_completion: bool) -> std::process::ExitStatus {
    // debug_log(&format!("proxying to git with args: {:?}", args));
    // debug_log(&format!("prepended global args: {:?}", prepend_global(args)));
    // Use spawn for interactive commands
    let child = Command::new(config::Config::get().git_cmd())
        .args(args)
        .spawn();

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

// Ensure SIGPIPE default action, even if inherited ignored from a parent shell
fn reset_sigpipe_to_default() {
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
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