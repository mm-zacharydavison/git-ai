# VSCode `git-ai` extension

# [Install `git-ai for VS Code here!`](https://open-vsx.org/extension/acunniffe/git-ai)

Search for "git-ai for VS Code" in VS Code Extensions tab

A VS Code extension that tracks AI-generated code using [git-ai](https://github.com/acunniffe/git-ai).

## Workaround Alert

VS Code/GitHub Copilot do not expose any events for AI-related code changes (yet, at least).

In an ideal world this extension would be able to listen for events like `onAIChangesAccepted` or `onAIChangesApplied`, but instead we are forced to use hueristics.

1. All single charecter inserts AND code changes trigged by paste, undo or redo shortcuts will be debounced for 4 seconds then trigger a human edit checkpoint.
2. All multi-line edits, not triggered by paste, undo or redo will immediatly trigger an AI edit checkpoint.

Known limitations:

- Checking out new HEADs may trigger an AI checkpoint on ustaged changes. Stash first.

You can enable toast messages from the extension when it calls checkpoints to get a feel for the effectiveness of the hueritics add this option to your settings:

```json
"vscodeGitAi.enableCheckpointLogging: true"
```

## Installation

1. **Install the extension** in VS Code
2. **Install [`git-ai`](https://github.com/acunniffe/git-ai)** `curl -sSL https://gitai.run/install.sh | bash`
3. **Ensure `git-ai` is on your PATH** so the extension can find it
4. **Restart VS Code**

## License

MIT
