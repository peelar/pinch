---
name: pinch
description: Start, find, read logs from and stop dev servers (pnpm dev, next dev, vite, etc.) through the `pinch` CLI instead of running them directly. Use whenever you need a dev server, a port, or server logs, so you never start duplicates or leave forgotten servers running.
---

# pinch: dev servers

Never start a dev server directly (`pnpm dev`, `npm run dev`, `next dev`, ...).
Start it through `pinch` so it is registered and nobody starts a duplicate.

- Start or reuse: `pinch ensure --name web --wait -- pnpm dev`
  Idempotent. If `web` already runs in this repo you get the existing one back.
  `--wait` blocks until it is listening and prints the port.
- See everything: `pinch ps` (add `--json` for machine-readable output).
  Check it before starting anything on a port.
- Read output: `pinch logs web --tail 100` (also `--follow`, `--grep error`).
  Works for servers other agents started too.
- Restart or stop: `pinch restart web`, `pinch stop web`. Stop the servers you
  started when you are done with them.
- Port already taken? Run `pinch ps`. It also lists servers started outside
  pinch ("foreign"). Do not kill processes you did not start without asking.
- If the user is asked whether to kill a stale server, `pinch inbox` shows the
  pending decisions. Leave them to the user.
