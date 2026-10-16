# pinch

Read [DEVELOPMENT.md](DEVELOPMENT.md) for setup, runtime behavior, and release
workflows. Keep it current when those change.

- Rust workspace, edition 2024. Crates live in `crates/`; shared deps go in
  `[workspace.dependencies]`.
- GPUI comes from Zeron's fork `zeronsh/zui`, pinned to the same rev Zeron uses.
  Don't bump it casually. When building UI, port from Zeron
  (`/tmp/zeron-src/zeron` or github.com/zeronsh/zeron, MIT) before writing from
  scratch, and never use Zed's GPL crates.
- The owner is new to Rust: briefly explain non-obvious Rust choices
  (ownership, lifetimes, async, unsafe/objc2) in PR descriptions or chat,
  not in code comments.
- Before finishing a change: `cargo fmt`, `cargo clippy --workspace`, `cargo test --workspace`.

## Local development preview

- For Rust UI and engine iteration, use `pnpm dev` (or `python3 scripts/dev.py`)
  instead of manually rebuilding and relaunching the installed app after each edit.
- Reuse an existing watcher for this checkout; the script rejects duplicates.
  It watches Rust sources and Cargo configuration, rebuilds incrementally, and
  restarts only changed components. Failed builds keep the previous preview.
- The preview uses a standalone UI that stays open on blur, a headless engine,
  and separate state in `target/pinch-dev/state`. To inspect or populate it, use
  `PINCH_HOME="$PWD/target/pinch-dev/state" target/pinch-dev/pinch ...`.
  The installed app and its state are separate from this preview.
- Verify changes against the refreshed preview. For menu bar, notifications,
  signing, or packaging changes, also verify a bundled app with
  `PROFILE=debug scripts/bundle.sh`; the headless preview does not cover those.
- Stop a watcher you started when verification is complete unless the user wants
  it left running. Ctrl-C cleans up the preview UI and engine; managed processes
  survive, so stop any test processes you started through the development CLI.

## Engine and desktop boundaries

Follow the ownership rules in [DEVELOPMENT.md](DEVELOPMENT.md#engine-and-desktop-boundaries).
Keep desktop/native imports in `crates/engine/src/desktop/`; the engine library
and RPC server must not depend on the desktop shell. Socket inspection commands
and parsing belong in `crates/engine/src/discovery/`. Decision callbacks run
under the registry lock: enqueue work, never block or call back into the engine.
Preserve restored-decision synchronization before starting the sampler.

Run `python3 scripts/check_architecture.py` and
`cargo check -p pinch-engine -p pinch-cli --no-default-features --locked` with the
normal checks. Do not bypass these checks with allowlists or feature flags.
If ownership changes intentionally, update the checks and these rules together
and explain the change. This separation does not imply Linux support.
