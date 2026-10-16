# Developing Pinch

Setup, local previews, installation, and release tooling. For agent integration,
see [agent instructions](docs/agent-instructions.md). Read [CLAUDE.md](CLAUDE.md)
for project conventions before editing code.

## Requirements

- macOS 13 or later and Xcode Command Line Tools (`xcode-select --install`).
- Rust with the toolchain and components in `rust-toolchain.toml`.
- Python 3 for the development watcher and script tests.
- Node.js 24 and the pnpm version in `package.json` for release tooling and
  the `pnpm dev` shortcut.

```sh
pnpm install --frozen-lockfile
cargo build --workspace --locked
```

GPUI is pinned to Zeron's `zeronsh/zui` fork. Follow the existing revision and
UI patterns; retain license notices when porting code or assets.

## Local preview

```sh
pnpm dev
# Or, without pnpm:
python3 scripts/dev.py
```

Reuse the watcher for this checkout; it rejects duplicates. It watches Rust
sources, Cargo configuration, and `assets/brand`, builds incrementally, and
runs a standalone UI with a headless engine. The preview stays open on blur.
UI-only edits retain the engine. Engine changes re-adopt managed processes;
failed builds leave the last working preview running.

Preview state is isolated in `target/pinch-dev/state`. Use its CLI to inspect
or populate it:

```sh
PINCH_HOME="$PWD/target/pinch-dev/state" target/pinch-dev/pinch ps
PINCH_HOME="$PWD/target/pinch-dev/state" target/pinch-dev/pinch ensure \
  --harness terminal --name web --wait -- pnpm dev
```

Run the second command from a project with a dev-server script, using absolute
paths to the preview CLI and state directory when switching directories.

Ctrl-C stops the watcher, preview UI, and preview engine. Managed servers survive;
stop any test servers through the preview CLI before finishing. Temporary UI
state, such as the selected log view, resets when the UI restarts. The preview
does not exercise menu bar, notification, signing, or packaging behavior.

## Build and install the app

```sh
PROFILE=debug scripts/bundle.sh
scripts/install.sh --app "$PWD/dist/Pinch.app" --no-modify-path
open "$HOME/Applications/Pinch.app"
```

Omit `PROFILE=debug` for a release build. The bundle contains `pinch`, `pinchd`,
and `pinch-ui`, and receives an ad hoc signature. Before delivering a feature,
install and verify the bundled app as required by [AGENTS.md](AGENTS.md).
Preserve installed state and running managed servers during updates.

## Install and update

The [README](README.md#get-pinch) has the public installer command. It installs
`~/Applications/Pinch.app`, links `pinch` into `~/.local/bin`, and can add that
directory to zsh/bash PATH configuration. Open a new terminal afterward.
The first CLI command starts the engine and menu bar on demand.

Rerun the installer to update. It shuts down the matching engine before replacing
the bundle; managed servers and saved state survive. Run `pinch setup` explicitly
after upgrading to refresh installed agent skills. Installer options include:

- `--no-modify-path`: leave shell configuration alone.
- `--version v0.1.0`: choose a published version.
- `--app /path/to/Pinch.app`: install a local bundle.

For a downloaded installer, pass options after `bash -s --`.

Public builds are currently ad hoc signed, without Apple notarization. If macOS
blocks Pinch, try opening `~/Applications/Pinch.app`, then use **System Settings →
Privacy & Security → Open Anyway** if offered. See
[Apple's approval instructions](https://support.apple.com/102445).
The installer does not remove quarantine or disable Gatekeeper.

To uninstall, run `pinch quit`, then remove `~/Applications/Pinch.app` and the
`~/.local/bin/pinch` symlink. State and logs remain in `~/.pinch`; installed
agent skills also remain. Linux and Windows are not supported, and published
builds currently target Apple Silicon.

## Separate installations

Use a named variant to test beside the normal installation:

```sh
PROFILE=debug scripts/bundle.sh --variant ui-test
scripts/install.sh --app "$PWD/dist/Pinch Ui Test.app" --no-modify-path
open "$HOME/Applications/Pinch Ui Test.app"
pinch-ui-test ps
```

This creates a separate app, bundle ID `sh.pinch.Pinch.ui-test`, CLI command,
and state directory `~/.pinch-ui-test`. Its registry, socket, logs, decisions,
onboarding, and notification permission are separate. The menu bar and popover
show the variant name. Reinstalling the same variant preserves its state and
servers; updating or quitting one installation leaves the others running.

Names start with a lowercase letter, use lowercase letters, digits, and single
hyphens, and have at most 32 characters. The installer reads the identity from
the signed bundle; optional `--variant ui-test` verifies the expected name.
Without a variant, the app uses `Pinch.app`, `pinch`, and `~/.pinch`.

`PINCH_HOME` overrides state for explicit CLI/engine testing; omit it for normal
variant use. The installer ignores an inherited override when shutting down an
installation. Variants share machine ports, so give test servers distinct ports.
They can discover one another's listeners but manage only their own registry.
Public release packaging uses the default `dist/Pinch.app`.

## Code and runtime

| Directory | Responsibility |
| --- | --- |
| `crates/proto` | Versioned newline-delimited JSON RPC and shared types |
| `crates/client` | Client requests, installation identity, engine startup, onboarding |
| `crates/engine` | Process supervision, metrics, discovery, decisions, persistence |
| `crates/macos` | Native menu bar and actionable notifications |
| `crates/cli` | Agent and terminal commands |
| `crates/ui` | GPUI process list and log detail view |
| `assets/brand` | Daemon, wordmark, menu-bar artwork, and app icon |

Managed children run in their own sessions and write to log files. There is no
PTY or per-process shim. State is persisted in `state.json`; the engine re-adopts
processes by PID and start time. An adopted process's exit code may be unknown.
Managed Stop signals the process group, escalating from SIGTERM to SIGKILL after
five seconds. Quitting Pinch leaves managed processes running.

The engine prewarms a resident UI and forwards menu-bar clicks over stdin. The
popover reads a cached snapshot and refreshes it while open. The log view strips
ANSI sequences and is not virtualized. The engine starts on demand; a launchd
agent and automatic updates are not implemented.

### Process identity

`run` and `ensure` require `--harness` or `PINCH_HARNESS`. Supported values are
`claude-code`, `codex`, `cursor`, `gemini-cli`, `copilot`, `opencode`, `zeron`,
and `terminal`. Optional `--thread-id` / `PINCH_THREAD_ID` and `--agent-id` /
`PINCH_AGENT_ID` identify sessions; Codex also reads `CODEX_THREAD_ID`.
Workspace and branch come from the working directory. Reusing a process preserves
its original attribution. Legacy unattributed entries must be stopped and
registered with a harness before they can be restarted.

`ensure` reuses an equivalent running command within the repo and rejects a
same-name process with a different command. Registered processes and discovered
listeners have separate controls. A runtime name or port does not establish
that a listener is a dev server. Discovered termination requires a separate
identity confirmation and rechecks PID/start time before sending SIGTERM to that
PID alone; it never signals the group or escalates to SIGKILL.

### Cleanup suggestions

A managed process becomes eligible after two quiet hours. Log output, observed
TCP connections, process-tree CPU usage of at least 5%, and `ensure` reuse each
reset the quiet interval. Connections and metrics are sampled every two seconds,
so brief requests may be missed. RAM estimates use process-tree RSS.

Stop, Keep running, and Snooze 1h are available through notifications and the
inbox. Keep lasts until the process restarts. Decisions and answers persist across
engine updates; restored decisions do not produce duplicate banners. Processes
without a pending decision receive a fresh observation interval after startup.
Discovered listeners never receive automatic stop suggestions.

macOS notification permission is required for banners. When notifications are
off, suggestions remain in the popover and `pinch inbox`. To test the quiet
interval, set `PINCH_STALE_AFTER_SECS=30` when starting an isolated engine; changing
it does not affect an already-running engine.

### Brand assets

```sh
uv run scripts/generate-brand.py
```

The generator pins Pillow and produces SVG, PNG, and ICNS assets from shared
geometry. Bundling uses the checked-in ICNS, so ordinary builds do not need image
tooling. Keep the daemon and wordmark consistent across app surfaces. Product
copy can be dry; action labels and errors must remain literal.

## Checks

```sh
cargo fmt --all -- --check
cargo clippy --workspace --locked
cargo test --workspace --locked
cargo build --workspace --locked
python3 -m unittest discover -s scripts/tests -v
pnpm check:version
bash -n scripts/bundle.sh scripts/install.sh scripts/release.sh
```

Add a Changeset for every PR, including documentation and tooling:

```sh
pnpm changeset
```

Select `pinch` and an appropriate bump; use patch for fixes. The automated
`changeset-release/main` version PR is the only exception. See
[the release workflow](.changeset/README.md).

## Release packaging

`scripts/release.sh` packages `dist/Pinch.app` into an architecture-specific ZIP,
its SHA-256 checksum, and an installer. Set `PINCH_REPOSITORY=owner/repo` to embed
the download location. By default it packages an ad hoc signed build.

For Developer ID signing and notarization, set `SIGNING_IDENTITY` and
`NOTARY_PROFILE` (a `notarytool` keychain profile). The script signs the binaries
and bundle, notarizes, staples the ticket, and packages the result.

Tagged GitHub builds create **draft** Apple Silicon releases. Signing uses these
optional repository secrets:

- `MACOS_CERTIFICATE_BASE64`: base64-encoded Developer ID Application `.p12`.
- `MACOS_CERTIFICATE_PASSWORD`: its password.
- `MACOS_SIGNING_IDENTITY`: full Developer ID Application signing identity.
- `APPLE_ID`, `APPLE_APP_PASSWORD`, `APPLE_TEAM_ID`: notarization credentials.

Changesets synchronizes the product version across package.json, Rust crates,
and app bundles. Use `pnpm run version`, then `pnpm check:version`. Follow the
[release workflow](.changeset/README.md) for the version PR, matching tag,
and review of the draft release before publication.
