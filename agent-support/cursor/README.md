# VSCode / Cursor `git-ai` extension

# [Install `git-ai for Cursor here!`](https://marketplace.cursorapi.com/items/?itemName=acunniffe.cursor-git-ai)

A VS Code extension that tracks AI-generated code using [git-ai](https://github.com/acunniffe/git-ai).

## Workaround Alert

Cursor does not expose any of its composer events or any hooks for `PostEdit` ([upvote this Feature Request](https://forum.cursor.com/t/request-hooks-support-post-edit-pre-edit-etc/114716) to help).

In an ideal world this extension would be able to listen for events like `onAIChangesAccepted` or `onAIChangesApplied`, but instead we are forced to use hueristics.

1. All single charecter inserts AND code changes trigged by paste, undo or redo shortcuts will be debounced for 4 seconds then trigger a human edit checkpoint.
2. All multi-line edits, not triggered by paste, undo or redo will immediatly trigger an AI edit checkpoint.

Known limitations:

- Checking out new HEADs may trigger an AI checkpoint on ustaged changes. Stash first.

You can enable toast messages from the extension when it calls checkpoints to get a feel for the effectiveness of the hueritics add this option to your settings:

```json
"cursorGitAi.enableCheckpointLogging: true"
```

## Installation

1. **Install the extension** in Cursor
2. **Install [`git-ai`](https://github.com/acunniffe/git-ai)** `curl -sSL https://gitai.run/install.sh | bash`
3. **Ensure `git-ai` is on your PATH** so the extension can find it
4. **Restart Cursor**

## License

MIT
