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
