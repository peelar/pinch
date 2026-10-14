# pinch

A penny-pinching resource manager for coding agents. Agents start dev servers
through pinch, so every server is registered by name, port and kind, its logs
are readable by any agent, and forgotten servers get noticed and turned off.

```bash
pinch ensure --name web --wait -- pnpm dev   # start, or reuse the one already running in this repo
pinch ps                                     # what's running, on which port, using how much
pinch logs web --tail 200 --follow
pinch stop web
```

A small engine (`pinchd`) runs in the background and starts itself the first
time you use the CLI. It lives in the menu bar: click the icon for a popover
with every server, its CPU/memory and logs. When a server has been silent for
a long time and nothing is connected to it, you get a notification with
**Kill / Keep / Snooze** buttons.

## Install

Prebuilt releases use a direct installer; Homebrew and Rust are not required.
Once this repository has a published release, replace `OWNER/REPO` below with
its GitHub repository:

```bash
curl -fsSL https://github.com/OWNER/REPO/releases/latest/download/install.sh | bash
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

No public release is configured yet. To install a local build:

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
signature; the download installer requires a Developer ID signed, notarized app.

For public releases, provide `SIGNING_IDENTITY` and `NOTARY_PROFILE` (a
`notarytool` keychain profile) when running `scripts/release.sh`. It signs each
binary with hardened runtime, signs the bundle, notarizes it, staples the ticket,
and packages the result.

The tag-triggered GitHub workflow builds an Apple Silicon release and creates a
**draft** GitHub release. Configure these repository secrets first:

- `MACOS_CERTIFICATE_BASE64`: base64-encoded Developer ID Application `.p12`.
- `MACOS_CERTIFICATE_PASSWORD`: password for that certificate export.
- `MACOS_SIGNING_IDENTITY`: full Developer ID Application signing identity.
- `APPLE_ID`, `APPLE_APP_PASSWORD`, `APPLE_TEAM_ID`: notarization credentials.

Record changes with `pnpm changeset`, then run `pnpm version` to update the
package and Rust workspace versions together. Check them with
`pnpm check:version`, commit the version changes, and push the matching `vX.Y.Z` tag,
then review and publish the draft release. Signing credentials are required;
the workflow does not publish unsigned downloads. Intel builds and an installer
on a custom domain can be added later.

## Build from source

```bash
scripts/bundle.sh                       # builds dist/Pinch.app (use PROFILE=debug for a faster build)
open dist/Pinch.app                     # menu bar icon appears; allow notifications when asked
ln -sf "$PWD/dist/Pinch.app/Contents/MacOS/pinch" ~/.local/bin/pinch

cd some-repo && pinch ensure --name web --wait -- pnpm dev
pinch ps
```

To see the stale-server flow quickly, start the engine with a short threshold:
`PINCH_STALE_AFTER_SECS=30 pinch ps` (the default is 2 hours).

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
cargo run -p pinch-ui                   # popover on its own (talks to a running engine)
pinchd --headless                       # engine without the menu bar, for tests
PINCH_HOME=/tmp/pinch-test pinch ps     # isolated state dir
```

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
Local app bundles are ad-hoc signed. GitHub release builds require Developer ID
signing and notarization credentials; see the release guide.

## Issues and releases

Bug reports and feature requests are welcome through GitHub Issues. This is a
maintainer-led project; please discuss changes before opening a pull request.
See [CONTRIBUTING.md](CONTRIBUTING.md).

Versions and changelogs use Changesets. See the
[release guide](.changeset/README.md) for the version PR and draft release flow.

## License

[MIT](LICENSE). Third-party dependencies retain their own licenses.
