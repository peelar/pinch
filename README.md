# pinch

A penny-pinching resource manager for coding agents. Agents start dev servers
through pinch, so every server is registered by name, port and kind, its logs
are readable by any agent, and forgotten servers get noticed and turned off.

```bash
pinch ensure --harness terminal --name web --wait -- pnpm dev   # start, or reuse the one already running in this repo
pinch ps                                     # what's running, on which port, using how much
pinch logs web --tail 200 --follow
pinch stop web
```

A small engine (`pinchd`) runs in the background and starts itself the first
time you use the CLI. It lives in the menu bar: click the icon for a popover
with every server, its CPU/memory and logs. After a managed process has been
quiet for 2 hours, Pinch asks whether to stop it, showing the estimated RAM
that could be freed. Notifications offer **Stop / Keep running / Snooze 1h**;
the same suggestions stay in the popover and `pinch inbox`.

Log output, observed TCP connections, process-tree CPU usage of at least 5%,
and reuse through `pinch ensure` each postpone suggestions for a full quiet
interval. Connections are sampled every 2 seconds, so very short requests may
be missed. Keep exempts that process until it is restarted; snooze postpones
it for an hour. Decisions and answers survive engine updates. Discovered
listeners never receive automatic stop suggestions.

macOS notification permission is required. If notifications are off, the
popover links to Notification settings. Suggestions still appear in the inbox.
RAM estimates use process-tree resident memory and are approximate.

## Test separate installations

Build a named variant to try a branch alongside your normal Pinch installation:

```bash
PROFILE=debug scripts/bundle.sh --variant proactive
scripts/install.sh --app "$PWD/dist/Pinch Proactive.app" --no-modify-path
open "$HOME/Applications/Pinch Proactive.app"
pinch-proactive ps
```

The variant is stored inside the signed bundle, so Finder, the CLI symlink,
and the engine all select the same installation automatically. This creates
`Pinch Proactive.app`, bundle ID `sh.pinch.Pinch.proactive`, command
`pinch-proactive`, and state in `~/.pinch-proactive` (registry, socket, logs,
decisions, and onboarding). The menu bar and popover show the variant name.
Notification permission is separate for each variant.

Rebuild and reinstall with the same name to update that variant and retain its
state and managed servers. Use another slug, such as `ui-test`, for another
installation. Names use lowercase letters, digits, and single hyphens, start
with a letter, and have at most 32 characters. The installer detects the name
from the bundle; optional `--variant proactive` verifies the expected name.
Updating or quitting one installation leaves the others running.

Without `--variant`, the build and installer continue to use `Pinch.app`,
`pinch`, and `~/.pinch`. `PINCH_HOME` is still an explicit override for CLI and
engine testing; omit it for normal variant use. Installation updates always
use the bundle's own default state, ignoring an inherited `PINCH_HOME`.
Each engine owns its own managed processes; other installations' listeners
can appear in discovery and require the usual separate confirmation to stop.
Variants do not isolate TCP ports: give test servers different ports.
The fast `pnpm dev` preview remains separate per checkout and headless.

## Install

Prebuilt releases use a direct installer; Homebrew and Rust are not required.
Install the latest published release:

```bash
curl -fsSL https://github.com/peelar/pinch/releases/latest/download/install.sh | bash
pinch setup
pinch ps
```

The installer supports macOS 13+ (Apple Silicon initially). It installs
`~/Applications/Pinch.app`, links the CLI into `~/.local/bin`, and adds that
folder to the zsh/bash PATH when needed. Open a new terminal after installation.
The CLI starts the engine and menu bar on demand.

Rerun the installer to update. The existing engine quits; managed servers keep
running. Agent skill updates remain explicit: run `pinch setup` after upgrading.
Pass `--no-modify-path` to manage PATH yourself, or `--version v0.1.0` to choose
a release, using `bash -s -- --version v0.1.0` in the command above.

To install a local build:

```bash
scripts/bundle.sh
scripts/install.sh --app dist/Pinch.app
```

To uninstall, run `pinch quit`, then remove `~/Applications/Pinch.app` and the
`~/.local/bin/pinch` symlink. State and logs remain in `~/.pinch`, and installed
agent skills remain in each agent's skills directory.

## Distributing releases

`scripts/release.sh` packages `dist/Pinch.app` into an architecture-specific ZIP,
its SHA-256 checksum, and an installer. Set `PINCH_REPOSITORY=owner/repo` to bake
the download location into the installer. Local packages use the existing ad hoc
signature; Apple signing credentials are optional. The installer checks the
checksum and bundle signature and accepts ad hoc signed releases.

To sign and notarize releases, optionally provide `SIGNING_IDENTITY` and `NOTARY_PROFILE` (a
`notarytool` keychain profile) when running `scripts/release.sh`. It signs each
binary with hardened runtime, signs the bundle, notarizes it, staples the ticket,
and packages the result.

The tag-triggered GitHub workflow builds an Apple Silicon release and creates a
**draft** GitHub release. Without signing credentials it distributes an ad hoc
signed build. To enable Developer ID signing and notarization later, configure
these repository secrets:

- `MACOS_CERTIFICATE_BASE64`: base64-encoded Developer ID Application `.p12`.
- `MACOS_CERTIFICATE_PASSWORD`: password for that certificate export.
- `MACOS_SIGNING_IDENTITY`: full Developer ID Application signing identity.
- `APPLE_ID`, `APPLE_APP_PASSWORD`, `APPLE_TEAM_ID`: notarization credentials.

Record changes with `pnpm changeset`, then run `pnpm version` to update the
package and Rust workspace versions together. Check them with
`pnpm check:version`, commit the version changes, and push the matching `vX.Y.Z` tag,
then review and publish the draft release. Intel builds and an installer
on a custom domain can be added later.

## Build from source

```bash
scripts/bundle.sh                       # builds dist/Pinch.app (use PROFILE=debug for a faster build)
open dist/Pinch.app                     # menu bar icon appears; allow notifications when asked
ln -sf "$PWD/dist/Pinch.app/Contents/MacOS/pinch" ~/.local/bin/pinch

cd some-repo && pinch ensure --harness terminal --name web --wait -- pnpm dev
pinch ps
```

To see the stale-server flow quickly, start the engine with a short threshold:
`PINCH_STALE_AFTER_SECS=30 pinch ps` (the default is 2 hours). This setting is
read when the engine starts; it does not change an already-running engine.
After an engine restart, processes without a pending suggestion receive a fresh
quiet interval because Pinch could not observe activity while it was stopped.

The first time you use pinch (in a terminal, or the first time `Pinch.app` opens) it detects Claude Code / Codex and offers to
install the pinch skill ([skill/SKILL.md](skill/SKILL.md)); from the app it appears as a card in the popover; rerun any time with `pinch setup`
(`-y` to skip the questions). Prefer plain instructions? See
[docs/agent-instructions.md](docs/agent-instructions.md).
Design and scope: [ARCHITECTURE.md](ARCHITECTURE.md).

## Development

Building requires macOS 13+, the Rust toolchain, and Xcode Command Line Tools
(`xcode-select --install`). Node.js and pnpm are needed only for release versioning;
install those dependencies with `pnpm install --frozen-lockfile`.

```bash
cargo build --workspace
python3 scripts/dev.py                  # watch, rebuild, and reopen the development UI
cargo run -p pinch-ui                   # popover on its own (talks to a running engine)
pinchd --headless                       # engine without the menu bar, for tests
PINCH_HOME=/tmp/pinch-test pinch ps     # isolated state dir
```

The watcher requires Python 3 and can also be started with `pnpm dev`. It runs a
standalone UI preview (which stays open when you switch to your editor) and a
headless engine with separate state in `target/pinch-dev/state`, leaving the
installed app alone. Rust source and Cargo
configuration changes trigger incremental debug builds. UI-only changes restart
just the preview; engine changes restart both components and re-adopt managed
processes. Build errors keep the previous preview running. Temporary UI state,
such as the selected log view, resets on restart.

To populate the preview, use its CLI in another terminal:

```bash
PINCH_HOME="$PWD/target/pinch-dev/state" target/pinch-dev/pinch ps
```

Ctrl-C stops the watcher, preview, and development engine. Processes registered
through the development CLI survive; stop them through that CLI before exiting
if they are no longer needed. The watcher does not rebuild the installed `.app`
bundle or exercise menu bar and notification behavior.

Discovered listeners are listed separately from processes registered through
Pinch. They are read-only: `stop` operates only on registered processes. The CLI
shows executable, command, working directory and PID for discovered listeners;
app-bundle paths are used to label bundled runtimes such as Raycast's Node process.
A runtime name and port alone do not identify a dev server.

Before committing, run `cargo fmt --all -- --check`,
`cargo clippy --workspace --locked`, and `cargo test --workspace --locked`.

## Platform and status

Pinch is early-stage software for macOS 13 or later. Building from source needs
Rust and Xcode Command Line Tools (`xcode-select --install`). Linux and Windows
are not supported. Release automation currently builds for Apple Silicon.
Releases are currently ad hoc signed, without an Apple developer identity or
notarization. If macOS blocks Pinch, try opening `~/Applications/Pinch.app`, then
use **System Settings → Privacy & Security → Open Anyway** if offered. See
[Apple's approval instructions](https://support.apple.com/102445).
The installer does not remove quarantine or disable Gatekeeper.
Developer ID signing can be enabled later; see the release guide.

## Issues and releases

Bug reports and feature requests are welcome through GitHub Issues. This is a
maintainer-led project; please discuss changes before opening a pull request.
See [CONTRIBUTING.md](CONTRIBUTING.md).

Versions and changelogs use Changesets. See the
[release guide](.changeset/README.md) for the version PR and draft release flow.

## License

[MIT](LICENSE). Third-party dependencies retain their own licenses.

### Agent attribution

`run` and `ensure` require `--harness` or `PINCH_HARNESS`: `claude-code`,
`codex`, `cursor`, `gemini-cli`, `copilot`, `opencode`, `zeron`, or `terminal`
(for manual starts). Supply `--thread-id` and `--agent-id` when available, or
set `PINCH_THREAD_ID` / `PINCH_AGENT_ID`. Codex also reads `CODEX_THREAD_ID`.
Repository/worktree and branch are captured automatically from the working directory.
The CLI and app group registered processes by harness, workspace, branch, thread,
and agent. `ensure` keeps the original starter's attribution when reusing a process.
Older records remain unattributed; to restart one, stop it and register it again
with a harness. Discovered listeners keep their separate section.
