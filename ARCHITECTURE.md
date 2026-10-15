# pinch — architecture & v1 scope

pinch keeps coding agents from wasting resources. Every dev server an agent
starts is registered by name, port and kind, its logs are readable by any
other agent, its CPU and memory are tracked, and forgotten servers are
noticed and offered up for killing. It runs in the background; the desktop
app is optional, but it should be a pleasure to open.

The name comes from *pinching pennies*, the English cousin of the Polish
*sknera*.

## Shape

```
Pinch.app (signed bundle, one install)
├── pinchd    engine: always running, small (launchd agent, auto-spawned by the CLI)
│   ├── supervises managed processes via per-process shims
│   ├── samples CPU/mem, discovers unmanaged listeners
│   ├── staleness rules → pending decisions
│   ├── SQLite registry, Unix socket RPC (pinch-proto)
│   └── macOS layer: menu bar item + actionable notifications (pinch-macos)
├── pinch     CLI: what agents use
└── pinch-ui  GPUI app, prewarmed by the engine for instant opening
```

| Crate | Binary | Role |
|---|---|---|
| `crates/proto` | — | Shared types and RPC messages (newline-delimited JSON, versioned) |
| `crates/engine` | `pinchd` | Supervision, metrics, rules, decisions, socket server; `pinchd shim` is the per-process supervisor |
| `crates/macos` | — | Status item and notifications via `objc2`, used by the engine |
| `crates/cli` | `pinch` | CLI |
| `crates/ui` | `pinch-ui` | Desktop app (GPUI, pinned to Zeron's `zui` fork) |

### Key decisions

- **The UI runtime stays ready.** The engine prewarms `pinch-ui` with a hidden
  window at startup. Clicks are forwarded over stdin; closing a popover removes
  its window while retaining the runtime. Opening uses a cached snapshot and
  refreshes in the background. This spends idle UI memory (roughly 200 MB) to
  avoid process and renderer startup on each click. The UI exits when the
  engine closes its stdin.
- **Each process gets its own shim.** A shim holds the process's PTY, writes its log
  file, and records its exit code. The engine reconnects to shims on start, so
  restarting or upgrading the engine never kills a dev server.
- **`ensure` is the main command, and it's idempotent.** It's keyed on repo root plus command. It must be
  race-safe: concurrent calls from several agents yield exactly one process.
- **Everything runs on decisions.** Signals feed rules, rules produce pending
  decisions, and decisions are resolved by actions. Notifications, the UI inbox
  and `pinch inbox` all render the same decisions. Answering *Keep* is
  remembered per process.
- **UI comes from Zeron.** Zeron (github.com/zeronsh/zeron) is MIT-licensed. Port its frost/glass
  surfaces, `running_pill`, motion helpers, theme, and the `alacritty_terminal`
  log view rather than reinventing them, and keep its license notice. Do not use
  Zed's GPL crates (`ui`, `theme`, `editor`, `markdown`).

## v1 scope

### Engine
- launchd agent, auto-spawned by the first CLI call; Unix socket RPC.
- `run`, `ensure` (repo-scoped, race-safe), `stop`, `restart`.
- Per-process shim: PTY, log file rotated at ~10 MB, exit code. Stop sends
  SIGTERM then SIGKILL after a timeout, to the whole process group.
- Port detection by walking the process tree; kind detection from argv.
- Metrics sampled every 2 s, last ~10 min kept in memory for sparklines.
- Discovery of unmanaged listening ports (read-only inspection).
- Health: crashed/exited processes keep exit code and last log lines. One
  staleness rule, done well: silent for N min, no connections to the port, and
  running for at least M min.
- Decisions with Kill / Keep / Snooze.
- macOS layer: menu bar item that shows a dot only when a decision is pending,
  notifications with action buttons, click opens the UI.

### CLI
`ensure`, `run`, `ps`, `logs` (`--tail`, `--follow`, `--grep`), `stop`,
`restart`, `wait`, `inbox`, all with `--json`. Plus an `AGENTS.md` snippet and an
agent skill telling agents to use pinch.

### Desktop app
- **Popover:** live process list (name, port, kind, repo/branch, CPU/mem with
  sparkline, status, uptime). The inbox sits at the top when it isn't empty.
- **Detail window (detachable):** log pane with follow-tail, plus kill, restart,
  open in browser, and reveal in Finder.
- Frosted surfaces, light/dark following the system, monospace numerals, full
  keyboard navigation, animated list changes.

### Acceptance criteria
- Restarting the engine never kills a managed process.
- Five agents calling `ensure` at once → exactly one process.
- `stop` leaves no orphaned processes.
- Engine idle: under ~20 MB RSS and under 0.5% CPU.
- The popover appears in under ~150 ms.
- Staleness notifications fire only when several signals agree.

### Out of v1, with the structure ready for it
More staleness signals (deleted worktree or branch, the agent session has ended), metrics
history, weekly "freed X GB" stats, links to Zeron chats, a `PreToolUse`
hook that blocks bare `pnpm dev`, an MCP server, port allocation across worktrees, a global
hotkey, a settings UI (v1 uses a config file), custom themes, auto-update,
Linux.

## Build order

1. **macOS spike:** status item, a notification whose buttons call back into
   Rust, and a click that opens a borderless GPUI window under the icon. This
   de-risks the riskiest pieces first.
2. Engine + shim + CLI: `run`, `ps`, `logs`, `stop`, end to end.
3. `ensure` (race-safe), port and kind detection, metrics, unmanaged discovery.
4. Decisions, the staleness rule, `pinch inbox`; wire them into notifications.
5. GPUI popover and detail window, porting Zeron components.
6. Hardening against the acceptance criteria; agent snippet and skill.

## Status (what v1 actually does today)

Built, and exercised end to end (CLI + engine, with real servers):
- `ensure` / `run` / `ps` / `logs` (`--tail`, `--follow`, `--grep`) / `stop` /
  `restart` / `wait` / `inbox` / `answer` / `quit`, all with `--json`.
- `ensure` is race-safe (five concurrent calls give one process) and refuses a
  same-name request with a different command.
- Managed processes survive engine restarts and are re-adopted by pid + start time.
- `stop` signals the whole process group (SIGTERM, SIGKILL after 5 s).
- Metrics (CPU, RSS summed over the process tree), port detection, and
  "found running" discovery of servers started outside pinch.
- Staleness rule (silent too long and nothing connected) producing decisions
  with Kill / Keep / Snooze; `PINCH_STALE_AFTER_SECS` overrides the 2 h default.
- Menu bar item with a pending-decisions badge, actionable notifications, and a
  GPUI popover (list, decision cards, log view) that opens under the icon.
- `pinch setup` (also offered once on first interactive use) detects Claude Code and
  Codex and installs `skill/SKILL.md` into their skills dirs. The Codex path
  (`~/.codex/skills`) is unverified.
- `scripts/bundle.sh` builds the ad hoc signed `Pinch.app`.
- `scripts/dev.py` watches Rust sources and Cargo configuration, incrementally
  builds debug binaries, and restarts an isolated standalone UI/headless engine
  preview. UI-only edits retain the engine; build failures retain the previous
  preview. Development state lives in `target/pinch-dev/state`.
- Direct macOS installation uses `scripts/install.sh`: a prebuilt bundle in
  `~/Applications`, a CLI symlink, optional zsh/bash PATH setup, and rerun-to-update.
- `scripts/release.sh` supports Developer ID signing and notarization. Tagged
  GitHub builds create draft Apple Silicon releases; public distribution still
  uses `peelar/pinch`; Apple signing credentials are optional. Releases default
  to ad hoc signed builds, with macOS approval left to users. Auto-update remains
  out of scope.

Deliberate simplifications versus the plan above:
- **No per-process shim, no PTY.** Children are started with `setsid` and write
  straight to a log file, which already lets them outlive the engine. The cost:
  no TTY (so some tools drop colour), and an adopted process's exit code is unknown.
- **Port discovery shells out to `lsof`** every 2 s instead of using libproc.
- **Logs in the popover are plain text** (ANSI stripped), not an
  `alacritty_terminal` view, and not yet virtualized.
- **System CPU/RAM meters** use cached counters from the engine's existing
  two-second sampling loop, even with the popover closed. Opening reads a fresh
  snapshot before rendering, so reopening needs no sampling warmup. CPU still
  needs two readings when the engine itself first starts. Both use green below
  60%, amber from 60%, and red from 85% utilization.
  Hover details include available RAM and swap; RAM fill measures occupancy.
- Zeron's frost/running_pill/theme code is not ported; the popover uses native
  window vibrancy and its own small theme.
- The engine is not installed as a launchd agent yet; the CLI starts it on demand.

The labeled CPU/RAM/uptime layout has been checked in a packaged preview.
Resident UI startup, opening over stdin, and shutdown on engine disconnect have
been smoke-tested. Notification actions and measured menu bar click latency
remain unverified.

Next up: launchd agent + login item, alacritty-based log view, the Claude Code
hook from docs/agent-instructions.md, more staleness signals (deleted
worktree, ended agent session), sparklines.

### Process provenance and identification

Registered processes and discovered listeners are separate sections in both the
popover and CLI. Stopped registered processes show a visible × button to clear their registry
entry. Only registered processes have Stop actions; the engine rejects
Stop requests for discovered listeners. Runtime names and listening ports do not
prove that a process is a dev server. Discovery remains a runtime-name heuristic.
Discovered records include the executable path, command, cwd, PID, and any app
bundle found in executable/command paths. When no direct app bundle exists,
the repository directory names the listener instead of only its runtime. The
nearest ancestor app is recorded separately and displayed as “via App”, so an
agent-launched server is not confused with the launching desktop app itself.
Parent lookup is bounded to 64 hops; reparented processes may have no launcher.
Runtime icons (Node, Python, npm, pnpm, Yarn, Bun, Cargo) are embedded SVGs
ported from Zeron’s MIT-licensed file icons, retaining their license. App bundle names are path-derived,
not verified bundle metadata; processes outside bundles can remain unidentified.
Discovered IDs include start time as well as PID. Protocol version 2 requires
updating the engine and clients together.
