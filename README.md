
<img src="assets/docs/git-ai.png" align="right"
     alt="Git AI by acunniffe/git-ai" width="120" height="120">

<h1 align="left"><b>git-ai</b></h1>
<p align="left">Track the AI Code in your repositories</p>

## Quick Start 

#### Mac, Linux, Windows (WSL)

```bash
curl -sSL https://raw.githubusercontent.com/acunniffe/git-ai/main/install.sh | bash
```

#### Windows (non-WSL - experimental)

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -Command "irm https://raw.githubusercontent.com/acunniffe/git-ai/main/install.ps1 | iex"
```

🎊 That's it! No per-repo setup. Once installed `git-ai` will work OOTB with any of **Supported Agents** *More coming soon*:

<img src="assets/docs/supported-agents.png" width="320" />

#### Next step: **Just code and commit!**
The Coding Agents (above) use `git-ai` to mark all the code they generate as AI-authored. 

On commit, `git-ai` adds a note that tracks which lines were AI-authored andoutput AI stats:

![alt](/assets/docs/graph.jpg) 

If you're curious about the AI authorship of any file `git-ai blame` will show you which lines are AI generated:

![alt](/assets/docs/blame-cmd.jpg)


## Goals of `git-ai` project

🤖 **Track AI code in a Multi-Agent** world. Because developers get to choose their tools, engineering teams need a **vendor agnostic** way to track AI impact and keep track of the AI code in their repos. 

🎯 **Accurate attribution** from Laptop → Pull Request → Merged. Claude Code, Cursor and Copilot do not count correctly because they can not see what happens to code after it's generated. 

🔄 **Support real-world git workflows** by making sure AI-Authorship annotations survive a `merge --squash`, `rebase`, `reset`, `cherry-pick` etc.

🔗 **Maintain link between prompts and code** - there is a lot of valuable context and requirments in your team's prompts -- don't throw them away. 

🚀 **Git-native + Fast** - `git-ai` is built on git plumbing commands. Unnoticiable impact even in xxl repos (<100ms). **we test in [Chromium](https://github.com/chromium/chromium)**








## Agent Support

`git-ai` automatically sets up all supported agent hooks using the `git-ai install-hooks` command

| Agent/IDE | Authorship | Prompts |
| --- | --- | --- |
| [Cursor >1.7](https://usegitai.com/docs/agent-support/cursor) | ✅ | ✅ |
| [Claude Code](https://usegitai.com/docs/agent-support/claude-code) | ✅ | ✅ |
| [GitHub Copilot in VSCode via Extension](https://usegitai.com/docs/agent-support/vs-code-github-copilot) | ✅ | ✅ |
| OpenAI Codex (waiting on [openai/codex #2904](https://github.com/openai/codex/pull/2904)) |  |  |
| Sourcegraph Cody + Amp |  |  |
| Windsurf |  |  |
| RovoDev CLI |  |  |
| _your agent here_ |  |  |

> **Want to add yours?** All PRs welcome! Add documentation to `docs/agent-support/`

### How it works

`git-ai` is a git CLI wrapper. It proxies commands, args, and flags to your `git` binary. You and your IDEs won't notice the difference, but all your code will be annotated with AI Authorship.

Internally, `git-ai` creates checkpoints to establish authorship of specific lines of code. Agents call `git-ai checkpoint` before they write to the file system to mark any previous edits as yours. After they write to the file system, they call `checkpoint agent-name ...` to mark their contributions as AI-generated and to save the associated propmpts. These checkpoints work similarly to how IDEs handle local history and they do not leave your machine. When you commit, `git-ai` compresses and packages the final authorship log and prompt transcripts into a git note attached to the commit.


### `git-ai` commands

All `git-ai` commands follow this pattern:

```bash
git-ai <command> [options]
```
##### `stats`

Show AI authorship statistics for a commit. Displays how much code was written by humans vs AI.

```bash
# Show stats for current HEAD
git-ai stats

# Show stats for specific commit
git-ai stats <commit-sha>

# Output in JSON format
git-ai stats --json
git-ai stats <commit-sha> --json
```

**Options:**
- `<commit-sha>` - Optional commit SHA (defaults to HEAD)
- `--json` - Output statistics in JSON format

##### `blame`

Enhanced version of `git blame` that shows AI authorship attribution alongside traditional git blame.

```bash
git-ai blame <file>
```

**Arguments:**
- `<file>` - Path to the file to blame (required)

**Options:**
Mostly API Compatible, supports same options as [`git blame`](https://git-scm.com/docs/git-blame). 

##### `install-hooks`

Automatically configure Claude Code, Cursor and GitHub Copilot to send authorship information to the `git-ai` binary 

```bash
git-ai install-hooks
```

### `git` proxy behavior 

After the `git-ai` binary is installed and put on the `$PATH`, it handles all invocations of `git` and `git-ai`. 

`git-ai` aims to be a transparent proxy with an unnoticeable performance impact. We reguarly run builds against [`git`'s unit tests](https://github.com/git/git/tree/master/t) to maximize cross platform compatibility and test the performance of our AI checkpointing code.

There two behavior changes `git-ai` introduces:

1. After commits, `git-ai` adds an AI Authorship log linked to the commit in `notes/ai` and print this visualization for developers: 

![alt](/assets/docs/graph.jpg)

2. In Git, notes do not sync by default. `git-ai` will append the refspec for `notes/ai` to `fetch` / `push` calls so they are always synced. 


### Known limitations

- Tab completions (from AI or traditional intellisense) are currently considered human edits.
- Authorship logs will not survive rebase, unless the rebase operation is run without git-ai (for ex, if the rebase is done on GitHub, the authorship logs from the affected commits will be quietly lost).
- AI deletions are not measured, only AI Additions and Total AI Line Count in the repo

## Developing `git-ai`

```bash
git clone https://github.com/acunniffe/git-ai.git
cd git-ai
cargo build
cargo test
```

Putting a development build of `git-ai` on your path

```
sh scripts/dev-symlinks.sh
task debug:local 
```
_you'll need to install [taskfile_](https://taskfile.dev/docs/installation)_


### License

MIT
