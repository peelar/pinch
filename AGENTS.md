# Agent instructions

Read and follow [CLAUDE.md](CLAUDE.md) before working in this repository. It
contains the shared project conventions and the required local preview workflow.

## Delivering features

Before presenting a completed feature, rebuild the bundled app, install it into
`~/Applications/Pinch.app`, and relaunch that installed copy so Adrian can try
it immediately. A development preview alone does not count as delivery. Use
`PROFILE=debug scripts/bundle.sh` and
`scripts/install.sh --app "$PWD/dist/Pinch.app" --no-modify-path`, then open the
installed app and verify the feature there. Preserve the installed app's state
and running managed servers during the update.

## Changesets

Every PR must include a Changeset for `pinch`, including documentation and tooling
changes. Run `pnpm changeset` and commit the generated `.changeset/*.md` file
with the change. Use a patch bump for fixes. Only the automated
`changeset-release/main` version PR is exempt because it consumes pending
Changesets. See [.changeset/README.md](.changeset/README.md).
