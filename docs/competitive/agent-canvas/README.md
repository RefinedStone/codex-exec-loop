# OpenHands Agent Canvas

## Snapshot

| Field | Value |
| --- | --- |
| Official repository | <https://github.com/OpenHands/agent-canvas> |
| Release | [v1.6.1](https://github.com/OpenHands/agent-canvas/releases/tag/v1.6.1) |
| Source commit | [`43f091baf135142ed6c146f888f44a957141193f`](https://github.com/OpenHands/agent-canvas/tree/43f091baf135142ed6c146f888f44a957141193f) |
| Release date | 2026-07-24 |
| Audit date | 2026-08-08 (Asia/Seoul) |
| Previous Akra pin | v1.2.1, `56d51c0767fb6fedc51c466f5138fdfc116a2707` |
| Akra baseline | `0c94f7e8b4549c2358f8b6d4b0c1ebc7e206dd48` |

Evidence is release/source inspection. This refresh did not install the desktop binaries or rerun
the remote/cloud automation stack.

## Verdict

Agent Canvas is a broad control and inspection surface over OpenHands Agent Server, ACP agents,
remote/cloud backends, and Automation. Its selected-conversation workspace remains a strong
reference: chat, files, diffs, terminal output, screenshots, tasks, and run history are connected
inside one responsive UI.

The old “browser only, no desktop” weakness is obsolete. v1.6.1 contains an Electron application,
macOS packaging, a Windows installer workflow, startup-log console, and update UI. The structural
opening remains: Canvas routes other runtimes and does not own Akra's exact-source-to-reviewed-merge
lifecycle.

## Material Change Since v1.2.1

The [compare range](https://github.com/OpenHands/agent-canvas/compare/v1.2.1...v1.6.1) contains 67
commits. Decision-relevant changes include:

- Electron desktop packaging, macOS DMG support, Windows installer/build fixes, and an expandable
  startup-log console;
- authoritative final-event reconciliation and reconnect side-effect deduplication;
- automation import/export and dependency updates;
- transcript export and settings/update surfaces;
- Software Agent SDK 1.37 and Automation updates;
- release-ready checks and desktop-specific workflows.

These changes remove a material distribution gap and improve stream correctness.

## Current Competitive Shape

### Strengths

- Rich browser and desktop inspection of one selected conversation.
- Local, remote, and cloud backends plus durable scheduled/event-triggered automation.
- Responsive information architecture with files, diffs, terminal, browser, tasks, and logs.
- Explicit reconnect reconciliation and canonical final-event handling.

### Limits Akra Can Exploit

- Agent Server/ACP and Automation own execution semantics; Canvas is not a Codex-native authority.
- End-to-end correctness crosses frontend, backend, agent runtime, ACP, and automation contracts.
- Multi-session inspection is not worktree lease, review incorporation, serialized integration, or
  cleanup proof.
- Desktop packaging adds another surface without changing that authority boundary.

## Akra Decisions

### Adopt

- Connected conversation inspection and truthful reconnect reconciliation.
- Durable trigger/run provenance and links from automation outcomes back to sessions.
- Startup diagnostics that remain available when the main UI fails.
- Read-only remote node health once it can project the same application truth.

### Reject

- ACP as Akra's core Codex runtime.
- A generic browser IDE or backend marketplace.
- Browser/desktop ownership of credentials or execution policy that belongs in the application.
- Calling session navigation “parallel delivery” without isolation and integration evidence.

### Differentiate

- Native Codex semantics and approvals.
- Planning/worktree/review/delivery authority.
- A bounded Admin cockpit backed by the same state as the TUI instead of a generic remote IDE.

## Evidence

- [v1.6.1 release](https://github.com/OpenHands/agent-canvas/releases/tag/v1.6.1)
- [Pinned source](https://github.com/OpenHands/agent-canvas/tree/43f091baf135142ed6c146f888f44a957141193f)
- [Electron entry](https://github.com/OpenHands/agent-canvas/blob/43f091baf135142ed6c146f888f44a957141193f/electron/main.mjs)
- [Windows desktop workflow](https://github.com/OpenHands/agent-canvas/blob/43f091baf135142ed6c146f888f44a957141193f/.github/workflows/desktop-windows.yml)
- [v1.2.1 to v1.6.1 comparison](https://github.com/OpenHands/agent-canvas/compare/v1.2.1...v1.6.1)

Watch whether the desktop becomes an authority rather than a wrapper and whether automation gains
reviewed integration guarantees.
