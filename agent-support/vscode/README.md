# git-ai Extension for VS Code & Cursor

A VS Code and Cursor extension that tracks AI-generated code using [git-ai](https://github.com/acunniffe/git-ai).

## VS Code

VS Code/GitHub Copilot do not expose any events for AI-related code changes (yet, at least).

In an ideal world this extension would be able to listen for events like `onAIChangesAccepted` or `onAIChangesApplied`, but instead we are forced to use hueristics based on the internals of the GitHub Copilot chat implementation in VS Code.

You can enable toast messages from the extension when it calls checkpoints to get a feel for the effectiveness of the hueritics add this option to your settings:

```json
"gitai.enableCheckpointLogging": true
```

### VS Code Installation

1. **Install the extension** We recommend installing from the [VS Code Extension marketplace](https://marketplace.visualstudio.com/items?itemName=git-ai.git-ai-vscode)
2. **Install [`git-ai`](https://github.com/acunniffe/git-ai)** Follow the `git-ai` installation [instructions](https://github.com/acunniffe/git-ai#installation) for your platform.
3. **Restart VS Code**

## License

MIT
