# Versioning Pinch

PRs that change app behavior must add a changeset: run `pnpm changeset`, select
a version bump for `pinch`, and commit the generated Markdown file. Use patch
bumps for fixes. For internal refactors, tests, CI, and contributor docs that
preserve app behavior, apply the `skip changeset` PR label instead.

The **Require changeset** check honors this label and reruns when labels are
added or removed. The automated `changeset-release/main` version PR is also
exempt because it consumes the pending changesets. Configure this check as
required in the `main` branch protection rules when available for the
repository's GitHub plan.

Pinch has one product version across every Rust crate and
the macOS app; the private root package exists only to drive Changesets.

The Version workflow opens or updates a release PR. `pnpm run version` consumes
changesets, writes `CHANGELOG.md`, and synchronizes Cargo.toml and Cargo.lock.
The app bundle reads its version from Cargo.toml. Nothing publishes to npm or
crates.io.

Enable **Settings → Actions → General → Allow GitHub Actions to create and
approve pull requests** after creating the repository. Version PRs made with
GITHUB_TOKEN do not trigger other workflows automatically; push an update or
run checks locally before merging.

After merging a version PR, verify main and push its matching tag:

```sh
git switch main
git pull --ff-only
pnpm install --frozen-lockfile
pnpm check:version
git tag "v$(node -p 'require("./package.json").version')"
git push origin "v$(node -p 'require("./package.json").version')"
```

The Release workflow builds an Apple Silicon app and creates a draft GitHub
release with a zip and SHA-256 checksum. Review its notes and test the downloaded
app before publishing. By default releases are ad hoc signed, with no Apple
notarization. Developer ID signing is optional: configure these secrets to enable it:
`MACOS_CERTIFICATE_BASE64`, `MACOS_CERTIFICATE_PASSWORD`,
`MACOS_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_APP_PASSWORD`, and `APPLE_TEAM_ID`.
If macOS blocks a download, use the [installation help](../DEVELOPMENT.md#install-and-update).
Intel builds are not currently distributed.

For the initial release, tag the current 0.1.0 after CI passes. Subsequent
versions come from changesets.
