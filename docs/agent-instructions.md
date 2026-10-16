# Telling agents to use pinch

Paste this into a project's `AGENTS.md` / `CLAUDE.md` (or your global one):

```markdown
## Dev servers

Never start a dev server directly (`pnpm dev`, `npm run dev`, `next dev`, …).
Use `pinch` so every server is registered and no one starts a duplicate.

- Start or reuse:  `pinch ensure --harness codex --name web --wait -- pnpm dev`
  Set `--harness` to your actual harness (`claude-code`, `codex`, `cursor`,
  `gemini-cli`, `copilot`, `opencode`, `zeron`). Include `--thread-id` and
  `--agent-id` when known; do not invent IDs. You can instead set
  `PINCH_HARNESS`, `PINCH_THREAD_ID`, and `PINCH_AGENT_ID`. Codex also reads
  `CODEX_THREAD_ID`. Worktree and branch are captured automatically.
  Idempotent: if `web` is already running in this repo it returns the existing
  one instead of starting another. `--wait` blocks until it is listening.
- See everything: `pinch ps` (add `--json` for machine-readable output).
  Check this before starting anything on a port.
- Read output:    `pinch logs web --tail 100` (`--follow`, `--grep error`).
- Restart / stop: `pinch restart web`, `pinch stop web`. Servers you started
  are yours to stop when you're done with them.
- If a start fails because a port is taken, run `pinch ps`: it lists servers
  that were started outside pinch too ("foreign"). Don't kill things you
  didn't start without asking.
```

## Make it stick (optional)

Instructions get ignored sometimes. A Claude Code `PreToolUse` hook can block
bare dev-server commands and point the agent at `pinch ensure`:

```jsonc
// .claude/settings.json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash",
        "hooks": [{ "type": "command", "command": "~/.pinch/hooks/block-bare-dev.sh" }]
      }
    ]
  }
}
```

The hook script reads the tool call on stdin and, when `tool_input.command`
matches `(pnpm|npm run|yarn|bun run) dev` or `next dev` and doesn't start with
`pinch`, exits with code 2 and a message like
"Use `pinch ensure --harness codex --name web -- pnpm dev` instead." (Not written yet.)
