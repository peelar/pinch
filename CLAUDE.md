# pinch

Read ARCHITECTURE.md first: it holds the design, the v1 scope, the acceptance
criteria and the build order. Keep it current when decisions change.

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
