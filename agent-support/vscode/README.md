# VS Code `git-ai` extension

# [Install `git-ai for VS Code here!`](https://marketplace.visualstudio.com/items?itemName=git-ai.git-ai-vscode)

Search for "git-ai for VS Code" in VS Code Extensions tab

A VS Code extension that tracks AI-generated code using [git-ai](https://github.com/acunniffe/git-ai).

## Workaround Alert

VS Code/GitHub Copilot do not expose any events for AI-related code changes (yet, at least).

In an ideal world this extension would be able to listen for events like `onAIChangesAccepted` or `onAIChangesApplied`, but instead we are forced to use hueristics based on the internals of the GitHub Copilot chat implementation in VS Code.

Known limitations:

- AI tab completions are treated as human edits, only chat/agent suggestions are marked as AI.

You can enable toast messages from the extension when it calls checkpoints to get a feel for the effectiveness of the hueritics add this option to your settings:

```json
"gitai.enableCheckpointLogging": true
```

## Installation

1. **Install the extension** in VS Code
2. **Install [`git-ai`](https://github.com/acunniffe/git-ai)** `curl -sSL https://gitai.run/install.sh | bash`
3. **Ensure `git-ai` is on your PATH** so the extension can find it
4. **Restart VS Code**

## License

MIT
