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
