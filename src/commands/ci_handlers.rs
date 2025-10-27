use crate::ci::github::get_github_ci_context;
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

fn handle_ci_github(_args: &[String]) {
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

fn print_ci_help_and_exit() -> ! {
    eprintln!("git-ai ci - Continuous integration utilities");
    eprintln!("");
    eprintln!("Usage: git-ai ci <subcommand> [args...]");
    eprintln!("");
    eprintln!("Subcommands:");
    eprintln!("  github           GitHub CI helpers");
    std::process::exit(1);
}


