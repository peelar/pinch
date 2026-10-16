# pinch

## 0.2.1

### Patch Changes

- 8c24f34: Allow removing stopped processes from their detail view and return to the list after successful removal.
- 8c24f34: Apply the daemon identity with original app and menu bar icons, high-contrast acid and neutral themes, a pixel wordmark, and concise product copy.

## 0.2.0

### Minor Changes

- 648b784: Require an explicit agent harness for new processes, persist thread and agent metadata, and group registered processes by harness, workspace, branch, and session in the app and CLI. Use compact harness icons with worktree and thread annotations in the app, with full metadata on hover.
- af73d23: Support named side-by-side app installations with isolated state, CLI commands, and macOS notification identities.

### Patch Changes

- 0f62571: Keep a visible gap between the process list and footer, including when the list scrolls.
- 2d91d9f: Show the Cloudflare icon for cloudflared processes.
- 31bfd30: Run GitHub Actions tooling on Node.js 24 and select Node.js 24 explicitly for CI version checks.
- f786b2a: Identify discovered listeners by repository and parent app, and add runtime icons for Node, Python, package managers, and Cargo.
- bc931d4: Allow stopping discovered listeners through the shared × action with explicit identity review and a live process identity check. Send SIGTERM only to the selected PID.
- beff4a5: Add a development watcher that rebuilds and restarts an isolated preview on source changes, with a UI preview mode that stays open when it loses focus.
- e06439d: Keep stop confirmation actions visible and collapse process details into an accordion.
- 1fdc8ca: Improve secondary text and label contrast on translucent backgrounds in light and dark mode.
- 3253f32: Keep the menu bar UI ready between openings, show compact system CPU and RAM meters with hover details, and label process CPU, RAM, and uptime more clearly. Allow stopped managed entries to be cleared from the list.
- af73d23: Make stale-process suggestions account for recent use, estimate RAM savings, and keep native notifications in sync with persistent decisions. Ignore cached CPU readings once a process stops accumulating CPU time, so an earlier burst cannot suppress suggestions forever.
- fd48080: Sort each process list by descending CPU usage, then RAM, with stopped entries last.
- 85a1d2f: Require a changeset with a Pinch version bump on every pull request, except the automated version PR.
- 84c0d6b: Show a compact, always-visible × button for stopped jobs in the popover process list.

  Tighten stopped job rows and use the existing status dot instead of a separate stopped label.

  Use green status dots for all running processes, including discovered listeners.

## 0.1.0

Initial release: managed dev servers, shared logs, process discovery, stale-server
decisions, a macOS menu bar app, and agent setup for Claude Code and Codex.
