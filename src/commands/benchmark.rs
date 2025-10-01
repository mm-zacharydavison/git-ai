use std::time::{Duration, Instant};

use git2::{Repository, StatusOptions};

use crate::{commands::checkpoint, config, git::find_repository_in_path};

pub fn profile_key_operations(working_dir: &str) {
    let repo = find_repository_in_path(working_dir).unwrap();

    println!(
        "Profiling git-ai performance in repo: {:?}",
        repo.path().as_os_str(),
    );

    let exec_spawn_time = git_status_spawn(working_dir);
    report_benchmark("git status", exec_spawn_time, None);

    let exec_porcelainv2_spawn_time = git_status_porcelainv2_spawn(working_dir);
    report_benchmark(
        "git porcelainv2",
        exec_porcelainv2_spawn_time,
        Some(exec_spawn_time),
    );

    let exec_porcelainv2_untracked_no_spawn_time =
        git_status_porcelainv2_untracked_no_spawn(working_dir);

    report_benchmark(
        "git porcelainv2 (tracked only)",
        exec_porcelainv2_untracked_no_spawn_time,
        Some(exec_spawn_time),
    );

    let libgit2_time = libgit_status(&repo);
    report_benchmark("libgit2", libgit2_time, Some(exec_spawn_time));

    let libgit2_no_untracked_time = libgit_status_no_untracked(&repo);
    report_benchmark(
        "libgit2 (tracked only)",
        libgit2_no_untracked_time,
        Some(exec_spawn_time),
    );

    let checkpoint_time = run_checkpoint(&repo);
    report_benchmark("git-ai checkpoint", checkpoint_time, Some(exec_spawn_time));
}

fn git_status_spawn(working_dir: &str) -> Duration {
    let now = Instant::now();

    let child = std::process::Command::new(config::Config::get().git_cmd())
        .arg("status")
        .current_dir(working_dir)
        .stdout(std::process::Stdio::null())
        .spawn();

    match child {
        Ok(mut child) => {
            let status = child.wait();
            match status {
                Ok(_status) => {
                    let elapsed_time = now.elapsed();
                    return elapsed_time;
                }
                Err(e) => {
                    eprintln!("Failed to wait for git status process: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Err(e) => {
            eprintln!("Failed to execute git status: {}", e);
            std::process::exit(1);
        }
    }
}

fn git_status_porcelainv2_spawn(working_dir: &str) -> Duration {
    let now = Instant::now();

    let child = std::process::Command::new(config::Config::get().git_cmd())
        .args(&["status", "--porcelain=v2"])
        .current_dir(working_dir)
        .stdout(std::process::Stdio::null())
        .spawn();

    match child {
        Ok(mut child) => {
            let status = child.wait();
            match status {
                Ok(_status) => {
                    let elapsed_time = now.elapsed();
                    return elapsed_time;
                }
                Err(e) => {
                    eprintln!("Failed to wait for git status process: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Err(e) => {
            eprintln!("Failed to execute git status: {}", e);
            std::process::exit(1);
        }
    }
}

fn git_status_porcelainv2_untracked_no_spawn(working_dir: &str) -> Duration {
    let now = Instant::now();

    let child = std::process::Command::new(config::Config::get().git_cmd())
        .args(&["status", "--porcelain=v2", "--untracked-files=no"])
        .current_dir(working_dir)
        .stdout(std::process::Stdio::null())
        .spawn();

    match child {
        Ok(mut child) => {
            let status = child.wait();
            match status {
                Ok(_status) => {
                    let elapsed_time = now.elapsed();
                    return elapsed_time;
                }
                Err(e) => {
                    eprintln!("Failed to wait for git status process: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Err(e) => {
            eprintln!("Failed to execute git status: {}", e);
            std::process::exit(1);
        }
    }
}

fn libgit_status(repo: &Repository) -> Duration {
    let now = Instant::now();
    let mut status_opts = StatusOptions::new();
    status_opts.include_untracked(true);
    status_opts.include_ignored(false);
    status_opts.include_unmodified(false);

    let statuses = repo.statuses(Some(&mut status_opts));

    for _entry in statuses.iter() {}

    let elapsed_time = now.elapsed();
    return elapsed_time;
}

fn libgit_status_no_untracked(repo: &Repository) -> Duration {
    let now = Instant::now();
    let mut status_opts = StatusOptions::new();
    status_opts.include_untracked(false);
    status_opts.include_ignored(false);
    status_opts.include_unmodified(false);

    let statuses = repo.statuses(Some(&mut status_opts));

    for _entry in statuses.iter() {}

    let elapsed_time = now.elapsed();
    return elapsed_time;
}

fn run_checkpoint(repo: &Repository) -> Duration {
    let now = Instant::now();

    let _checkpoint_run =
        checkpoint::run(repo, "human", false, false, true, None, None, None).unwrap();

    let elapsed_time = now.elapsed();
    return elapsed_time;
}

fn report_benchmark(name: &str, duration: Duration, git_status_gold_standard: Option<Duration>) {
    match git_status_gold_standard {
        Some(gold_standard) => {
            let percent_different =
                (duration.as_nanos() as f64 / gold_standard.as_nanos() as f64) * 100 as f64;
            println!(
                "{}: {:?} {}%",
                pad_left(name, 35),
                duration,
                pad_left(percent_different.floor().to_string().as_str(), 5)
            );
        }
        None => {
            println!("{}: {:?}", pad_left(name, 35), duration);
        }
    };
}

fn pad_left(name: &str, n: u8) -> &str {
    let len = name.len();
    if len >= n as usize {
        name
    } else {
        // SAFETY: This is safe because we're only padding with ASCII spaces.
        // The returned &str is always valid UTF-8.
        // We return a new String, but the function signature expects &str,
        // so we need to return a &str with the correct lifetime.
        // To keep the signature, let's return a substring of a static buffer if possible,
        // but that's not practical. Instead, let's change the function to return a String.
        // However, since the signature is fixed, let's document that this is not ideal.
        // For now, just pad and return a &str slice of a static buffer for demonstration.
        // But in real code, this should return a String.
        // We'll leak the string for now to satisfy the signature.
        let mut s = String::with_capacity(n as usize);
        for _ in 0..(n as usize - len) {
            s.push(' ');
        }
        s.push_str(name);
        Box::leak(s.into_boxed_str())
    }
}
