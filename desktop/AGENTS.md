# Akra Desktop Agent Guide

Electron client for `akra-gateway` (plan: [`docs/plan/17-akra-desktop`](../docs/plan/17-akra-desktop/README.md)).
The repository-wide rules in [`../AGENTS.md`](../AGENTS.md) still apply.

## Rules

- The renderer never uses Node APIs. Windows use `sandbox`, `contextIsolation`, and no `nodeIntegration`.
- main and renderer talk only through the `window.akra` message bus (`src/shared/bridge.ts`).
- main does not interpret JSON-RPC. It relays strings between a window and that window's Gateway connection.
  Conversation logic lives in the renderer.
- Do not edit `src/shared/protocol/generated/` or `src/shared/protocol/version.ts` by hand. Regenerate with
  `npm run generate:protocol` after a codex upgrade.
- Follow the Codex usage policy (plan D-13): development and tests send no model-calling requests
  (`turn/start`, `turn/steer`, …). Use the fake Gateway in `test/fake-gateway/` with recordings.
  Committed fixtures contain only altered content; never commit real recordings.

- `src/renderer/blueprint/` is the target UX with mock data. It imports nothing from `session/`, `rpc/`, or
  `window.akra`. Move a real component to the blueprint look one at a time; colors and sizes come from
  `src/renderer/design/tokens.css`.

## Commands

```bash
npm ci                 # ELECTRON_SKIP_BINARY_DOWNLOAD=1 when the Electron binary is not needed
npm run check          # strict TypeScript for main/preload/tests and renderer
npm test               # vitest, including the fake Gateway over a unix socket or named pipe
npm run build          # electron-vite bundle into out/
npm run dev            # fetch a missing Electron binary, cargo build akra-gateway, then open the app
                       # (starts the newer of ../target/{release,debug}/akra-gateway if none is running)
npm run blueprint      # open the UX blueprint screen (src/renderer/blueprint): mock data only, no gateway, no codex
npm run e2e            # build, then drive the app against the fake gateway (CI runs it under xvfb-run)
```

`AKRA_GATEWAY_ENDPOINT` points the app at another gateway endpoint (the fake gateway in tests); `npm run dev` then skips the cargo build.
`AKRA_GATEWAY_BIN` starts that gateway executable instead of a `../target` build; `npm run dev` then skips the cargo build.
`AKRA_DESKTOP_HIDDEN=1` keeps windows hidden so screen tests do not cover the user's other apps.
`AKRA_DESKTOP_USER_DATA=<dir>` moves Electron userData (browser profiles, cookies, settings) to a test folder.
The terminal e2e and the real pty test skip on Windows (a real shell may open a window); set `AKRA_E2E_TERMINAL=1` to run them.
