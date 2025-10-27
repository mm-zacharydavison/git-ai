use crate::ci::github::{get_github_ci_context, install_github_ci_workflow};
use crate::utils::debug_log;

pub fn handle_ci(args: &[String]) {
    if args.is_empty() {
        print_ci_help_and_exit();
    }

    match args[0].as_str() {
        "github" => {
            handle_ci_github(&args[1..]);
        }
        _ => {
            eprintln!("Unknown ci subcommand: {}", args[0]);
            print_ci_help_and_exit();
        }
    }
}

fn handle_ci_github(args: &[String]) {
    if args.is_empty() {
        print_ci_github_help_and_exit();
    }
    // Subcommands: install | (default: run in CI context)
    match args[0].as_str() {
        "run" => {
            let ci_context = get_github_ci_context();
            match ci_context {
                Ok(Some(ci_context)) => {
                    debug_log(&format!("GitHub CI context: {:?}", ci_context));
                    if let Err(e) = ci_context.run() {
                        eprintln!("Error running GitHub CI context: {}", e);
                        std::process::exit(1);
                    }
                    if let Err(e) = ci_context.teardown() {
                        eprintln!("Error tearing down GitHub CI context: {}", e);
                        std::process::exit(1);
                    }
                    debug_log("GitHub CI context teared down");
                    std::process::exit(0);
                }
                Err(e) => {
                    eprintln!("Failed to get GitHub CI context: {}", e);
                    std::process::exit(1);
                }
                Ok(None) => {
                    eprintln!("No GitHub CI context found");
                    std::process::exit(1);
                }
            }
        }
        "install" => {
            match install_github_ci_workflow() {
                Ok(path) => {
                    println!(
                        "Installed GitHub Actions workflow to {}",
                        path.display()
                    );
                    std::process::exit(0);
                }
                Err(e) => {
                    eprintln!("Failed to install GitHub CI workflow: {}", e);
                    std::process::exit(1);
                }
            }
        }
        other => {
            eprintln!("Unknown ci github subcommand: {}", other);
            print_ci_help_and_exit();
        }
    }
}

fn print_ci_help_and_exit() -> ! {
    eprintln!("git-ai ci - Continuous integration utilities");
    eprintln!("");
    eprintln!("Usage: git-ai ci <subcommand> [args...]");
    eprintln!("");
    eprintln!("Subcommands:");
    eprintln!("  github           GitHub CI");
    eprintln!("    run            Run GitHub CI in current repo");
    eprintln!("    install        Install/update workflow in current repo");
    std::process::exit(1);
}

fn print_ci_github_help_and_exit() -> ! {
    eprintln!("git-ai ci github - GitHub CI utilities");
    eprintln!("");
    eprintln!("Usage: git-ai ci github <subcommand> [args...]");
    eprintln!("");
    eprintln!("Subcommands:");
    eprintln!("  run            Run GitHub CI in current repo");
    eprintln!("  install        Install/update workflow in current repo");
    std::process::exit(1);
}
