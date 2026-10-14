# Issues and development

Pinch is a maintainer-led personal project. Bug reports and feature requests are
welcome through GitHub Issues. There is no response-time or implementation promise.

Please open an issue before starting a pull request. Unsolicited pull requests
may be closed; publishing the source does not imply a commitment to review them.
Keep reports focused and remove credentials and sensitive logs.

For local development, use macOS, the Rust toolchain in `rust-toolchain.toml`,
Node.js 24, and the pnpm version in `package.json`. Node and pnpm are only needed
for release tooling. Read [ARCHITECTURE.md](ARCHITECTURE.md) for scope and design.

Before submitting changes:

```sh
pnpm install --frozen-lockfile
pnpm check:version
cargo fmt --all -- --check
cargo clippy --locked --workspace -- -D warnings
cargo test --locked --workspace
```

User-visible changes need a changeset: run `pnpm changeset`, select `pinch`,
choose a patch/minor/major bump, and describe the change for users. Documentation
and CI-only changes generally do not need one.
