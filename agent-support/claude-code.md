# Using Claude Code with `git-ai`

Add hooks to your Claude Code Settings [`~/.claude/settings.json`](https://docs.anthropic.com/en/docs/claude-code/hooks).

Notes:
You can check in these updated hook settings. `2>/dev/null` will swallow errors when `git-ai` is not installed or errors out removing the risk of breaking teammates workflows.

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Write|Edit|MultiEdit",
        "hooks": [
          {
            "type": "command",
            "command": "git-ai checkpoint 2>/dev/null || true"
          }
        ]
      }
    ],
    "PostToolUse": [
      {
        "matcher": "Write|Edit|MultiEdit",
        "hooks": [
          {
            "type": "command",
            "command": "git-ai checkpoint --author \"Claude Code\" 2>/dev/null || true"
          }
        ]
      }
    ]
  }
}
```
