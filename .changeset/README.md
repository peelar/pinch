# Versioning Pinch

Changes that alter app behavior must add a changeset: run `pnpm changeset`,
select a version bump for `pinch`, and commit the generated Markdown file with
the change. Use patch bumps for fixes. Internal refactors, tests, CI, and
contributor docs that preserve app behavior need none. Commits may be pushed
straight to `main`. Pull requests run the **Require changeset** check, which the
`skip changeset` label waives.

Pinch has one product version across every Rust crate and
the macOS app; the private root package exists only to drive Changesets.

Every push to `main` releases what its changesets describe. When changesets
are pending, the Release workflow runs `pnpm run version`, which consumes them,
writes `CHANGELOG.md`, and synchronizes Cargo.toml and Cargo.lock, then commits
`Version Pinch vX.Y.Z` to `main`. When that version has no GitHub release yet,
the same run builds the Apple Silicon app and publishes release `vX.Y.Z` at the
versioned commit, with its `CHANGELOG.md` section as notes and a zip, SHA-256
checksum, installer, and signed Sparkle update feed. A push without a changeset
releases nothing. The version commit is pushed with GITHUB_TOKEN, so it does not
trigger CI or another release run. The app bundle reads its version from
Cargo.toml. Nothing publishes to npm or crates.io.

`SPARKLE_PRIVATE_KEY` must contain the private key matching
`native/updater/public-key.txt`. Publishing the stable release makes its feed
available to installed apps; drafts and prereleases are not offered. Releases
publish without review, so test changes in the bundled app before pushing them. By default releases are
ad hoc signed, with no Apple notarization. Developer ID signing is optional: configure these secrets to enable it:
`MACOS_CERTIFICATE_BASE64`, `MACOS_CERTIFICATE_PASSWORD`,
`MACOS_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_APP_PASSWORD`, and `APPLE_TEAM_ID`.
If macOS blocks a download, use the [installation help](../DEVELOPMENT.md#install-and-update).
Intel builds are not currently distributed.
