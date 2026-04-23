# Claude-First Headless CLI Runner

## Goal

Keep Akra's main interactive conversation runtime on `codex app-server` while preparing a
Claude-first headless CLI runner for hidden planning and future sub-task flows only.

## Why This Direction Exists

The runtime seam cleanup already gave the repo capability-owned names such as `StartupProbePort`,
`SessionCatalogPort`, `InteractiveTurnRuntimePort`, and `TerminalBridgeAttachmentProfile`.
That seam vocabulary should stay available for future work, but the product no longer needs a
tmux-backed bridge path in the shipped runtime.

The next bounded extension point is `PlanningWorkerPort`.
Today that port is still satisfied only by `AppServerPlanningWorkerAdapter`, so the immediate
direction is to define how a Claude-first headless worker can plug into that port without changing
the main interactive Codex session.

## Near-Term Focus

- keep the main shell bootstrap directly wired to `CodexAppServerAdapter`
- treat `PlanningWorkerPort` as the next concrete extension seam
- design the launch, streaming, failure, and changed-file contract for a Claude headless planning
  worker
- keep the runner scope limited to hidden planning and future sub-task execution
- leave main interactive Claude, SSH or tunnel mediation, proxy mediation, and broader wrapper
  rollout out of scope

## Acceptance

- the product runtime has one main interactive path: `Codex app-server`
- queue head work talks about `PlanningWorkerPort` and a Claude-first headless runner instead of a
  tmux bridge rollout
- attachment vocabulary stays generic enough for future runner truth, but the first runner slice
  does not pretend it is a full interactive attach story
- the current baseline is explicit: Codex remains the main conversation runtime, Claude headless
  work starts with hidden planning and sub-task runners only

## Supporting Docs

- `docs/plan/21-terminal-agent-bridge-research-and-capability-boundary.md`
- `docs/plan/22-terminal-agent-transport-and-attachment-matrix.md`
- `docs/plan/23-terminal-agent-capability-boundary-and-session-contract.md`
- `docs/plan/24-terminal-agent-bridge-experiment-matrix.md`
- `docs/plan/25-codex-assumption-to-capability-target-map.md`
- `docs/plan/26-capability-map-prioritized-seam-follow-ups.md`
- `src/application/port/outbound/planning_worker_port.rs`
- `src/adapter/outbound/app_server/planning_worker.rs`
- `src/adapter/inbound/tui/app/shell_entrypoint.rs`
