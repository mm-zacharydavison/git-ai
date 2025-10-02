/// Parse the arguments that come *after* the `git` executable.
/// Example input corresponds to: `git -C .. commit -m foo`  => args = ["-C","..","commit","-m","foo"]
///
/// Rules:
/// - Only recognized Git *global* options are placed into `global_args`.
/// - The first non-option token (that isn't consumed as a value to a preceding global option)
///   is taken as the `command`.
/// - Everything after the command is `command_args`.
/// - If there is **no** command (e.g. `git --version`), then meta top-level options like
///   `--version`, `--help`, `--exec-path[=path]`, `--html-path`, `--man-path`, `--info-path`
///   are treated as `command_args` (never as `global_args`).
/// - Supports `--long=VAL`, `--long VAL`, `-Cpath`, `-C path`, `-cname=value`, and `-c name=value`.
///
/// This does *not* attempt to validate combinations or emulate Git's error paths.
/// It is intentionally permissive and order-preserving.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedGitInvocation {
    pub global_args: Vec<String>,
    pub command: Option<String>,
    pub command_args: Vec<String>,
    /// Whether a top-level `--` was present between global args and the command.
    pub saw_end_of_opts: bool,
    /// True if this invocation requests help: presence of -h/--help or `help` command.
    pub is_help: bool,
}

impl ParsedGitInvocation {
    /// Return the argv *after* `git` as tokens, in order:
    ///   global_args [+ command] + command_args
    ///
    /// Note: this reconstructs *what we stored*. Re-inserts a top-level `--` if it was present.
    pub fn to_invocation_vec(&self) -> Vec<String> {
        let mut v = Vec::with_capacity(
            self.global_args.len()
                + self.command_args.len()
                + usize::from(self.command.is_some())
                + usize::from(self.saw_end_of_opts),
        );
        v.extend(self.global_args.iter().cloned());
        if self.saw_end_of_opts {
            v.push("--".to_string());
        }
        if let Some(cmd) = &self.command {
            v.push(cmd.clone());
        }
        v.extend(self.command_args.iter().cloned());
        v
    }

    /// Convenience: return a shell-safe single-line string suitable for logging or
    /// re-running in POSIX-y shells. (This quotes conservatively.)
    /// If you want Windows/PowerShell quoting too, I can add a sibling helper.
    pub fn to_shell_line(&self) -> String {
        fn sh_quote(s: &str) -> String {
            // allow a common safe subset unquoted
            if !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_./:@%+,=~".contains(c))
            {
                s.to_string()
            } else {
                // POSIX single-quote, with embedded ' escaped as: '\''
                let mut out = String::from("'");
                for ch in s.chars() {
                    if ch == '\'' {
                        out.push_str("'\\''");
                    } else {
                        out.push(ch);
                    }
                }
                out.push('\'');
                out
            }
        }
        self.to_invocation_vec()
            .into_iter()
            .map(|t| sh_quote(&t))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

pub fn parse_git_cli_args(args: &[String]) -> ParsedGitInvocation {
    use Kind::*;

    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    enum Kind {
        GlobalNoValue,
        GlobalTakesValue, // e.g., --exec-path[=path]
        MetaNoValue,      // e.g., --version, --help, --html-path, --man-path, --info-path
        Unknown,          // something starting with '-' that isn't recognized at top-level
    }

    // Helpers to recognize/parse options.
    fn is_eq_form(tok: &str, long: &str) -> bool {
        tok.len() > long.len() + 1 && tok.starts_with(long) && tok.as_bytes()[long.len()] == b'='
    }

    fn classify(tok: &str) -> Kind {
        // Meta top-level (treated as command args when no command):
        // --version/-v, --help/-h, and the *-path* queries.
        match tok {
            "-v" | "--version" => return MetaNoValue,
            "-h" | "--help" => return MetaNoValue,
            "--html-path" | "--man-path" | "--info-path" => return MetaNoValue,
            _ => {}
        }
        if tok == "--exec-path" || is_eq_form(tok, "--exec-path") {
            return GlobalTakesValue;
        }

        // Global no-value options.
        match tok {
            "-p"
            | "--paginate"
            | "-P"
            | "--no-pager"
            | "--no-replace-objects"
            | "--no-lazy-fetch"
            | "--no-optional-locks"
            | "--no-advice"
            | "--bare"
            | "--literal-pathspecs"
            | "--glob-pathspecs"
            | "--noglob-pathspecs"
            | "--icase-pathspecs" => return GlobalNoValue,
            _ => {}
        }

        // Global takes-value options (support both `--opt=VAL` and `--opt VAL`).
        if tok == "-C" || tok.starts_with("-C") {
            return GlobalTakesValue;
        } // allow -Cpath
        if tok == "-c" || tok.starts_with("-c") {
            return GlobalTakesValue;
        } // allow -cname=value
        if tok == "--git-dir" || is_eq_form(tok, "--git-dir") {
            return GlobalTakesValue;
        }
        if tok == "--work-tree" || is_eq_form(tok, "--work-tree") {
            return GlobalTakesValue;
        }
        if tok == "--namespace" || is_eq_form(tok, "--namespace") {
            return GlobalTakesValue;
        }
        if tok == "--config-env" || is_eq_form(tok, "--config-env") {
            return GlobalTakesValue;
        }
        if tok == "--list-cmds" || is_eq_form(tok, "--list-cmds") {
            return GlobalTakesValue;
        }
        if tok == "--attr-source" || is_eq_form(tok, "--attr-source") {
            return GlobalTakesValue;
        }
        // Seen in some builds' SYNOPSIS; treat as value-taking if present.
        if tok == "--super-prefix" || is_eq_form(tok, "--super-prefix") {
            return GlobalTakesValue;
        }

        // A plain `--` (end-of-options) is handled in the main loop.
        if tok == "--" {
            return Unknown;
        }

        // Anything else starting with '-' is unknown to top-level git option parsing.
        if tok.starts_with('-') {
            return Unknown;
        }

        // Non-dash token => not an option (caller decides whether it's the command).
        Unknown
    }

    // Consume one token that *may* have an attached value (e.g. `--opt=VAL`, `-Cpath`, `-cname=val`).
    // Returns (tokens_to_push, tokens_consumed).
    fn take_valueish<'a>(all: &'a [String], i: usize, key: &str) -> (Vec<String>, usize) {
        let tok = &all[i];

        // Long form with '=' (e.g. --git-dir=/x, --exec-path=/x, --config-env=name=ENV).
        if let Some(eq) = tok.find('=') {
            if eq > 0 && tok.starts_with("--") {
                return (vec![tok.clone()], 1);
            }
        }

        // Short sticky for -Cpath / -cname=value
        if key == "-C" && tok != "-C" && tok.starts_with("-C") {
            return (vec![tok.clone()], 1);
        }
        if key == "-c" && tok != "-c" && tok.starts_with("-c") {
            return (vec![tok.clone()], 1);
        }

        // Separate value in next token (if present).
        if i + 1 < all.len() {
            return (vec![tok.clone(), all[i + 1].clone()], 2);
        }
        // No following value; just return the option and let downstream handle the error later.
        (vec![tok.clone()], 1)
    }

    let mut global_args = Vec::new();
    let mut command: Option<String> = None;
    let mut command_args = Vec::new();

    // If we see meta options *before* any command, we buffer them here.
    // If we end up with no command, we move them into command_args; otherwise we leave them out.
    // (Per your rule, e.g. `git --version` => command=None, command_args=["--version"]).
    let mut pre_command_meta: Vec<String> = Vec::new();

    // First pass: scan leading global options. Stop when we hit:
    // - `--` (then next token is *the command*, even if it starts with '-')
    // - a non-option token (that's the command)
    // - an unknown dash-option (treat as "no command", remaining go to command_args)
    let mut i = 0usize;
    let mut saw_end_of_opts = false;

    while i < args.len() {
        let tok = &args[i];

        if tok == "--" {
            saw_end_of_opts = true;
            i += 1;
            break;
        }

        match classify(tok) {
            GlobalNoValue => {
                global_args.push(tok.clone());
                i += 1;
            }
            GlobalTakesValue => {
                // Figure out which key we're handling to parse sticky forms.
                let key = if tok.starts_with("-C") {
                    "-C"
                } else if tok.starts_with("-c") {
                    "-c"
                } else if tok.starts_with("--git-dir") {
                    "--git-dir"
                } else if tok.starts_with("--work-tree") {
                    "--work-tree"
                } else if tok.starts_with("--namespace") {
                    "--namespace"
                } else if tok.starts_with("--config-env") {
                    "--config-env"
                } else if tok.starts_with("--list-cmds") {
                    "--list-cmds"
                } else if tok.starts_with("--attr-source") {
                    "--attr-source"
                } else if tok.starts_with("--super-prefix") {
                    "--super-prefix"
                } else {
                    ""
                };

                let (taken, consumed) = take_valueish(args, i, key);
                global_args.extend(taken);
                i += consumed;
            }
            MetaNoValue => {
                // Buffer meta; they'll become command_args iff no subcommand appears.
                pre_command_meta.push(tok.clone());
                i += 1;
            }
            Unknown => {
                if tok.starts_with('-') {
                    // Unknown top-level dash-option: treat as a meta-ish/invalid sequence.
                    // We won't assign a command; remaining tokens will become command_args later.
                    // Do not mutate `pre_command_meta` here; post-parse rewrites rely on it.
                    command = None;
                    break;
                } else {
                    // Non-dash token => this is the command.
                    break;
                }
            }
        }
    }

    // If we haven't decided the command yet:
    if command.is_none() {
        if i < args.len() {
            if saw_end_of_opts {
                // `--` forces the very next token to be "the command", even if it begins with '-'.
                command = Some(args[i].clone());
                i += 1;
            } else if !args[i].starts_with('-') {
                // Normal case: first non-dash token after globals is the command.
                command = Some(args[i].clone());
                i += 1;
            } else {
                // Only meta/unknown options; no command.
                command = None;
            }
        } else {
            command = None;
        }
    }

    // The remainder are command args (if we found a command).
    if command.is_some() {
        command_args.extend_from_slice(&args[i..]);
        // NOTE: we intentionally DO NOT inject pre_command_meta when a subcommand exists.
        // Example: `git --help commit` is internally converted to `git help commit`, but per
        // the project's requirement we treat meta as *not* global and don't try to rewrite.
        // If you want to emulate conversion, you can special-case it here.
    } else {
        // No command: meta options are considered "command args".
        command_args.extend(pre_command_meta.clone());
        command_args.extend_from_slice(&args[i..]);
    }

    // --- NEW: post-parse rewrite for help/version to match git(1) semantics ---
    // Top-level presence of -h/--help or -v/--version (before any command)
    let pre_has_help = pre_command_meta.iter().any(|t| t == "--help" || t == "-h");
    let pre_has_version = pre_command_meta
        .iter()
        .any(|t| t == "--version" || t == "-v");

    // NOTE: git docs: --help takes precedence over --version. (git(1) OPTIONS)
    // So we always check/perform help rewrites before version rewrites.
    if command.is_some() {
        // Case: `git --help <cmd> [rest]`  ==>  `git help <cmd> [rest]`
        if pre_has_help {
            let orig_cmd = command.take().unwrap();
            let mut new_args = vec![orig_cmd];
            // Pass trailing tokens after the command to `git help` unchanged.
            new_args.extend(command_args.drain(..));
            command = Some("help".into());
            command_args = new_args;
        }
        // NEW: `git --version ...` should rewrite to `git version` even if we
        // happened to parse a command token. Help still takes precedence.
        else if pre_has_version {
            // Drop the previously parsed command entirely and keep only version-relevant flags.
            command = Some("version".into());

            // Build args for `git version`: keep pre-command meta except the first -v/--version.
            let mut new_args = Vec::new();
            let mut dropped_one_version = false;
            for t in pre_command_meta.iter() {
                if !dropped_one_version && (t == "--version" || t == "-v") {
                    dropped_one_version = true;
                    continue;
                }
                new_args.push(t.clone()); // e.g., "--build-options"
            }

            // Do NOT carry over the previously parsed command or its args.
            command_args = new_args;
        }
    } else {
        // No subcommand parsed.

        // Case: `git --help [<cmd>|<help-opts>]`  ==>  `git help [<cmd>|<help-opts>]`
        if pre_has_help {
            command = Some("help".into());

            // Build args for `git help`: keep pre-command meta except the first help token.
            let mut new_args: Vec<String> = Vec::new();
            let mut dropped_one_help = false;
            for t in pre_command_meta.iter() {
                if !dropped_one_help && (t == "--help" || t == "-h") {
                    dropped_one_help = true;
                    continue;
                }
                // Help takes precedence: drop any version tokens when rewriting to help
                if t == "--version" || t == "-v" {
                    continue;
                }
                new_args.push(t.clone());
            }
            // Plus anything we already copied into `command_args` (drop stray help/version tokens)
            for t in command_args.iter() {
                if t == "--help" || t == "-h" || t == "--version" || t == "-v" {
                    continue;
                }
                new_args.push(t.clone());
            }
            command_args = new_args;
        }
        // Case: `git --version [--build-options]`  ==>  `git version [--build-options]`
        // (Only rewrite version when no command; --help would have taken precedence above.)
        else if pre_has_version {
            command = Some("version".into());
            // Remove the first occurrence of -v/--version; drop any non-dash tokens (e.g., stray commands)
            let mut new_args = Vec::new();
            let mut dropped_one_version = false;
            for t in command_args.iter() {
                if !dropped_one_version && (t == "--version" || t == "-v") {
                    dropped_one_version = true;
                    continue;
                }
                if t.starts_with('-') {
                    new_args.push(t.clone());
                }
            }
            command_args = new_args;
        }
    }
    // --- End NEW block ---

    // Determine whether this invocation represents a help request.
    let is_help = command.as_deref() == Some("help")
        || command.as_deref() == Some("--help")
        || pre_command_meta.iter().any(|t| t == "--help" || t == "-h")
        || command_args.iter().any(|t| t == "--help" || t == "-h");

    ParsedGitInvocation {
        global_args,
        command,
        command_args,
        saw_end_of_opts,
        is_help,
    }
}
