# Issues and development

Pinch is a maintainer-led personal project. Bug reports and feature requests are
welcome through GitHub Issues. There is no response-time or implementation promise.

Please open an issue before starting a pull request. Unsolicited pull requests
may be closed; publishing the source does not imply a commitment to review them.
Keep reports focused and remove credentials and sensitive logs.

For setup, previews, required checks, and release tooling, see
[DEVELOPMENT.md](DEVELOPMENT.md). Follow [CLAUDE.md](CLAUDE.md) for project conventions.

Every PR needs a Changeset for `pinch`, including documentation and tooling
changes. Run `pnpm changeset`, choose the appropriate bump, and commit the
generated file. Only the automated `changeset-release/main` version PR is exempt.
