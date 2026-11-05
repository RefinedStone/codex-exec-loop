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

## Commands

```bash
npm ci                 # ELECTRON_SKIP_BINARY_DOWNLOAD=1 when the Electron binary is not needed
npm run check          # strict TypeScript for main/preload/tests and renderer
npm test               # vitest, including the fake Gateway over a unix socket or named pipe
npm run build          # electron-vite bundle into out/
npm run dev            # open the app; starts akra-gateway from ../target if none is running
```
