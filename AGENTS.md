# Agent rules

- Read and follow [CLAUDE.md](CLAUDE.md) before working here.
- Before delivering a feature, run `PROFILE=debug scripts/bundle.sh` and
  `scripts/install.sh --app "$PWD/dist/Pinch.app" --no-modify-path`. Relaunch
  `~/Applications/Pinch.app` and verify the feature there. A development preview
  alone is insufficient. Preserve app state and running managed servers.
- Add a `pinch` Changeset for changes users should receive in a release. Skip
  it for internal refactors, tests, CI, and contributor docs that preserve app
  behavior. Run `pnpm changeset` and commit the generated `.changeset/*.md` file.
  Use patch bumps for fixes. See [.changeset/README.md](.changeset/README.md).
- Follow [engine ownership rules](DEVELOPMENT.md#engine-and-desktop-boundaries).
  Keep desktop/native imports in `crates/engine/src/desktop/`. The engine library
  and RPC server must not depend on the desktop shell.
- Keep socket inspection commands and parsing in `crates/engine/src/discovery/`.
- Decision callbacks hold the registry lock. Enqueue work; never block or call
  back into the engine.
- Synchronize restored decisions before starting the sampler.
- Run `python3 scripts/check_architecture.py` and
  `cargo check -p pinch-engine -p pinch-cli --no-default-features --locked`
  alongside normal checks. Never bypass them with allowlists or feature flags.
