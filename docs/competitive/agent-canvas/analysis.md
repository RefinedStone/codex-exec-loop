# Agent Canvas v1.2.1 Deep Dive

This analysis compares OpenHands Agent Canvas v1.2.1 with Akra at prerelease commit
`0229ed71a541d91abc04ad13b188d28b9eafcaf8`. Reproduction commands, immutable source links, and
limitations are in [evidence.md](evidence.md). Akra decisions and reviewable implementation slices
are in [gap-matrix.md](gap-matrix.md).

## Executive Verdict

Agent Canvas is a broad browser control center for starting, switching, and inspecting coding-agent
sessions and automations across local, remote, and cloud backends. It is not a verified parallel
delivery orchestrator. Its strongest threat to Akra is the cohesion of its selected-conversation
inspector: chat, files and diffs, observed terminal output, browser screenshots, tasks, and some
planner state share one responsive workspace. A second threat is the combination of backend
switching with a durable Automation service that supports schedules, signed event triggers, run
history, and links back to conversations and logs.

The structural weakness is that Canvas is a presentation and routing layer over other runtimes. Its
released Codex path crosses Agent Server and ACP, loses or changes important native semantics, and
does not own commit-to-reviewed-integration state. Multi-session navigation is useful, but it is not
fleet topology, worktree authority, conflict ownership, exact-source validation, review handling,
or cleanup. The released stack also has concrete safety and contract weaknesses: Codex ACP
permissions are auto-selected, same-provider sessions using ambient login can share provider config
and lock state, backend keys are stored in browser local storage, and two Automation terminal
states are absent from the Canvas type and badge implementation.

Akra should adopt current-activity visibility, durable trigger/run provenance, and eventually
read-only remote Akra node health. It should reject ACP as a core runtime, multi-provider breadth,
browser IDE duplication, prompt-only delivery controls, browser-stored node credentials, and a
generic workflow marketplace. It should differentiate as:

> the Codex-first operating and delivery layer that turns accepted intent into an isolated,
> exact-source-validated, reviewed, remotely verified integration and cleanup.

## Product And Audience

Agent Canvas describes itself as a self-hosted developer control center for coding agents and
automations. The v1.2.1 README names OpenHands, Claude Code, Codex, Gemini, and other ACP agents and
supports local, remote, and cloud backends. The repository is MIT licensed and labels the product
Beta. These are `documented` product claims; the inspected source verifies the backend registry,
agent presets, and Automation UI.

The likely primary operator is a developer or self-hosted team that values provider and execution
backend choice, browser access, and a rich inspector more than one provider's native protocol or a
prescriptive delivery lifecycle. That audience inference follows from the shipped registry,
deployment modes, and UI, not from a reproduced user study.

The application is distributed as an npm CLI and Docker image. The full-stack mode starts a static
frontend/proxy, Agent Server, and Automation service. Its default Docker-less local mode is
explicitly unsandboxed and gives the agent access to the host filesystem; Docker sandboxing is an
optional safer mode. Public self-hosting requires a shared API key plus ingress, TLS, and network
hardening outside Canvas.

The audited release has no native TUI or separately packaged desktop/mobile client. Its responsive
browser application is the operator surface.

## Architecture

### Canvas Boundary

Canvas is a React 19 and React Router 7 frontend built with Vite. It talks to Agent Server over REST
and WebSocket and to a separate Automation service over HTTP. The browser can remember several
backends, check their health, and associate a conversation with the backend that owns it. Agent
Server, not Canvas, owns the conversation runtime, sandbox, workspace, event history, and ACP
subprocess. Automation owns trigger definitions, scheduling, run dispatch, recovery, and its own
database.

This separation keeps the frontend adaptable but makes end-to-end guarantees depend on several
contracts and processes. Canvas preloads history through REST, reconnects a conversation WebSocket,
and requests events since the last observed sequence. Message submission has an HTTP queue
fallback. Conversation lists poll every ten seconds; selected-conversation state polls more often
during transitions and more slowly when stable. WebSocket reconnect uses a fixed three-second
delay with unbounded retries by default, without exponential backoff or jitter.

Canvas v1.2.1 pins these released components:

```text
Agent Canvas 1.2.1
  -> OpenHands Agent Server / software-agent-sdk 1.33.0
    -> @zed-industries/codex-acp 0.16.0
      -> embedded Codex Rust crates from rust-v0.137.0
```

The released Codex ACP process embeds Codex core, auth, thread, and storage crates. It does not
spawn `codex app-server`. This matters because a later source snapshot has a different topology:

```text
software-agent-sdk 1.35.0 (not shipped by Canvas 1.2.1)
  -> @agentclientprotocol/codex-acp 1.1.2
    -> official codex app-server child from @openai/codex ^0.144.0
```

The second path is useful future evidence, not a v1.2.1 feature. It introduces an additional
process and JSON-RPC boundary, but this audit did not measure whether that creates a user-visible
latency or memory disadvantage.

### Workspaces And Continuity

The attached-local-workspace flow initializes its selector to `local_repo`; a user can change it to
`new_worktree`. The separate start-from-scratch flow omits both workspace and mode, and the service
falls back to a generated working directory with `worktree: true`. When requested, Agent Server
creates a branch and worktree below `/tmp/conversation-worktrees/<conversation-id>/`. Isolation is
therefore path-dependent rather than one global default. Deleting a conversation deliberately
preserves its workspace, and this audit found no corresponding backend worktree/branch cleanup path.

Selected repository, branch, workspace, and workspace mode are not Agent Server source-provenance
concepts. Canvas stores them as browser-local conversation metadata and hydrates the view on that
client. A different browser or cleared storage can therefore lose those labels even when the
conversation remains. That is materially weaker than Akra's durable accepted task, lease, branch,
and frozen-range provenance.

ACP session identity and working directory are stored in opaque Agent Server state. On resume, the
SDK tries `load_session`; on a protocol error it can fall back to a fresh session. That is recovery,
but it can also silently change continuity semantics. The released client cannot request generic
per-conversation ACP data-directory isolation, so ambient-login conversations can reuse provider
HOME/config state. File-backed credentials such as `CODEX_AUTH_JSON` are separately materialized
into a per-conversation directory. A comparative crash-and-resume experiment was not run.

## Harness And Context

Canvas exposes agent presets, model selection, MCP configuration, secrets, skills, runtime service
URLs, confirmation mode, and provider-specific settings. For built-in ACP providers, an empty
command is resolved through pinned registries rather than floating to an arbitrary executable.
That is a sound packaging pattern.

Context assembly, compaction, model memory, and tool execution remain Agent Server or provider-agent
responsibilities. Canvas configures and renders those systems but does not provide a separate
authoritative memory or provenance layer.

The released Codex path is materially narrower end to end than either Codex core or the ACP wrapper:

- plan updates exist in both audited ACP wrappers, but the Python `ACPAgent` event bridge does not
  consume them;
- separately, Canvas hides the Code/Plan mode-switch control for ACP sessions;
- reasoning deltas are buffered into final reasoning content instead of retaining their streaming
  interleave;
- native thread and turn identity is transformed into Agent Server events and is not preserved as
  the primary Canvas correlation key;
- tool progress is collapsed, while native diff, patch-delta, and some error distinctions are lost
  or rendered only as generic tool data;
- `request_permission` always selects the first ACP option rather than asking the operator;
- the SDK's `ask_agent()` path invokes ACP `fork_session`, while neither inspected Codex ACP
  wrapper advertises or implements it.

Canvas's outer OpenHands confirmation policy, the ACP session mode, and the ACP permission callback
are separate controls. Canvas defaults the outer policy to never confirm, the released Codex preset
uses full-access mode, and the Python ACP callback independently selects option index zero without
an operator round trip. For generic filesystem or network permission requests, both audited Codex
wrappers put session-wide approval first. By contrast, Akra directly classifies the official
app-server notification schema and fails a contract test when a method has no handled, deferred,
diagnostic, or ignored disposition. Akra still reduces some native events too coarsely, so direct
topology is a fidelity opportunity, not proof that every field is already projected well.

## Browser UX And Session Oversight

Agent Canvas has no native terminal UI; this section evaluates its browser interaction model.

### Strengths

Canvas's selected-conversation experience is mature. A resizable chat and inspector can show:

- a workspace file tree, read-only file content, and diffs;
- observed shell commands and output through xterm;
- the latest browser screenshot and URL;
- a task list and, in supported cloud contexts, planner state;
- structured cards for OpenHands and ACP tools, reasoning, skills, MCP, and confirmations;
- paged history, streaming reduction, pending-message feedback, attachments, and reconnect state.

The conversation rail supports pinning, grouping, filtering, sorting, and an active-only view. This
is effective multi-session navigation. Backend health degradation and session-to-backend memory
also reduce friction for operators who move between execution environments.

The `/goal` interaction deserves separate credit. Canvas intercepts the command and renders
objective, round, score, missing evidence, and stop/resume state from an Agent Server judge loop.
The goal runtime is not Canvas-owned, but the UI makes a long-running control loop legible.

### Limits

The browser workspace is an inspector, not a browser IDE in the full sense. The xterm instance has
input disabled and only renders recorded command logs. Browser state is a screenshot plus URL and
external-open action. Files and diffs are read-only, the file listing is bounded to 2,000 entries,
and the changes view requests at most the first 100 files.

More importantly for Akra's product direction, Canvas is not a graph canvas or parallel supervisor.
It presents a list of independent conversations and a deep view of one selection. The shipped
subconversation projection assumes one planning agent. It does not show task dependencies,
worktree leases, file/hotspot ownership, collision risk, validation source SHA, PR review state,
integration order, or cleanup.

The list status dot also maps `IDLE` and `WAITING_FOR_CONFIRMATION` to the same green working state.
That makes an approval bottleneck hard to identify across a fleet even though the selected session
can expose confirmation detail.

## Admin, Remote, And Automation

### Remote Backends

Canvas is ahead of Akra in released remote routing. A browser can register local, remote, and cloud
Agent Server endpoints, observe health, and switch between them. The cost is a broad trust model:
each backend record stores its host and plaintext API key in browser `localStorage`. The local
static server can also inject the key into HTML/local storage. Public mode avoids that injection,
but Canvas itself does not provide a multi-user RBAC or durable operator audit layer.

Akra's Admin server is currently loopback-only. A future answer should be a read-only Akra node
registry with server-side or OS-private credentials, mutual-TLS identity, capabilities, version,
workspace scope, and stale-state handling. It should not become a generic provider registry.

### Automation

OpenHands Automation 1.1.4 is the clearest capability to borrow selectively. It has:

- cron schedules with IANA timezones;
- event triggers with source/event matching and JMESPath filters;
- HMAC verification for built-in and custom webhook sources;
- enabled/disabled definitions, soft deletion, and run-now;
- a durable run row linking timestamps, errors, event payload, conversation, sandbox, and bash
  command;
- scheduler polling with database locking, dispatcher timeouts, cancellation, and watchdog recovery;
- encrypted per-automation runtime key/value state.

Canvas exposes definition list/detail, enable/disable, Run Now, partial editing, activity history,
conversation links, and stdout/stderr logs. This is a useful operational trace, but it does not
expose the whole backend contract. Creating an automation launches a translated prompt in a new
agent conversation rather than submitting a deterministic form. The edit modal can change schedule,
model, and prompt fields, while event-trigger source/event/filter fields are read-only. The backend
has run cancellation, but the Canvas service and UI do not call it.

The implementation also supplies design warnings:

- webhook replay or delivery-ID idempotency is not implemented;
- rate limits and request-body limits are delegated to infrastructure;
- ordinary failed runs do not have a general retry/attempt model;
- dispatcher batch size is a pickup bound, not a global execution semaphore;
- concurrency exhaustion can produce a `SKIPPED` terminal state;
- the backend has six run states, but Canvas's TypeScript enum and badge cover only four;
- Canvas omits backend `timeout_at`, `sandbox_id`, and `created_at` fields and does not project retry
  attempts, queue wait, watchdog recovery reason, or an audit actor.

Because the API returns the backend status unchanged and the badge dereferences an absent config,
rendering a `CANCELLED` or `SKIPPED` run has a source-certain `TypeError` path. The backend can
produce both states, so this is a verified contract mismatch and an inferred runtime failure. The
full failure was not reproduced in a live deployment.

Akra should not build a general automation platform. It should accept typed, authenticated,
idempotent code-change events into operator-owned task templates and preserve one trace from trigger
through accepted task, lease, session, validation, PR, delivery outcome, and cleanup.

## Parallel Work And Delivery

Canvas can create many conversations and can request an Agent Server worktree per conversation.
Its Git pull, push, and create-PR buttons insert natural-language instructions into the agent
conversation. This is convenient, but the application does not own a delivery state machine or
prove that the requested operation happened against the intended source.

Akra's shipped parallel-mode contract is stronger in this dimension. It owns accepted task intent,
leases, worktrees, session detail, a frozen source range, GitHub review/check gates, serialized
integration, remote verification, and cleanup. Explicit parent policy can select high-risk
review-skipped paths, but those exceptions have provenance rather than silently imitating reviewed
delivery. Host-run or trusted-CI validation evidence bound to the exact frozen SHA remains a planned
P0 contract; the current free-form validation summary is not that proof.

Akra's immediate weakness is activity visibility. Its parallel reducer deliberately avoids
invalidating supervisor state for tool activity and delta/completion notifications. The operator can
see lifecycle and selected detail but cannot reliably scan what each agent is doing now or how long
it has been quiet. Canvas demonstrates the value of rich selected-session events, while jcode's
earlier audit already identified the same fleet-level gap. This comparison should therefore amend
and elevate the existing parallel-activity slice, not create a competing subsystem.

## Performance

No fair product-performance comparison was completed. Canvas's repository does not publish a
current, reproducible startup, interaction, steady-state memory, or concurrent-session benchmark
that can be compared with Akra. This audit did not run authenticated Codex turns or a three-session
Canvas deployment.

Local release-gate execution in an exported source tree produced these development-process results:

| Command | Result | Wall time | Maximum RSS |
| --- | --- | ---: | ---: |
| `npm run lint` | passed | 46.62 s | 3,362,648 KiB |
| `npm test` | passed: 480 files, 1 skipped; 3,666 tests passed | 52.82 s | 763,632 KiB |
| `npm run build` | passed | 8.76 s | 1,759,796 KiB |
| `npm run build:lib` | passed | 12.83 s | 1,581,092 KiB |

These values describe build and test processes on one machine. They are not Canvas runtime latency
or resource measurements and do not establish an Akra advantage. The build emitted chunk-size
warnings; the largest client chunk was 526.28 kB before compression. `npm pack --dry-run` described
a 14,215,651-byte tarball, 57,069,667 unpacked bytes, and 9,439 entries. These are packaging facts,
not user-experience scores.

The existing jcode P0 performance-evidence contract remains the right Akra work item. Once that
schema is available, a Canvas topology profile can add cold usable UI, resume, submit-to-first-event,
first assistant delta, three-session propagation, and complete-process-tree memory. Until then the
performance winner is `unverified`.

## Quality And Risk

Agent Canvas has substantial automated coverage. The release checkout contains 500 TypeScript test
or spec files by pathname and 19 E2E spec files. Vitest executed 481 files and 3,680 test cases in
this environment: 3,666 passed, 5 were skipped, and 9 were todo. Main CI runs install and the
application build on Ubuntu and Windows; lint, unit tests, library build, and package verification
run in the Ubuntu full-check job. Live E2E is conditional on a manual trigger or an eligible labeled
pull request, and no enforced coverage threshold was found.

The architecture's breadth creates cross-repository contract risk. The Automation status mismatch
is one concrete example. The ACP chain is another: Canvas pins a client/protocol range due to a
known argument-order break, while Agent Server, Python ACP, a provider wrapper, and Codex each evolve
on separate release axes. The next maintained Codex bridge is architecturally closer to Akra but is
not part of Canvas v1.2.1.

### Shipped v1.2.1

Security- and privacy-relevant findings include:

- plaintext backend API keys in browser local storage;
- an unsandboxed local default documented as full host-filesystem access;
- outer OpenHands confirmation disabled and a full-access Codex ACP preset by default;
- automatic first-option permission selection in the Python ACP bridge;
- an ACP child environment that starts from the parent environment, strips only selected variables,
  and adds every non-file secret from the conversation registry;
- potentially shared provider HOME/config/lock state for concurrent same-provider conversations
  using ambient login in the released Canvas path;
- no application-level webhook replay deduplication;
- one anonymous install event can be sent before normal tracking consent, including browser platform,
  user agent, referrer, origin, and embedded status, unless DNT or environment policy disables it.

### Later Path, Not Shipped

The maintained Codex ACP bridge has an opt-in log mode that can record raw app-server frames and
prompts without visible redaction. That is an inferred risk for a later stack, not a Canvas v1.2.1
finding.

These findings do not prove a remotely exploitable vulnerability in a properly isolated deployment.
They do prove that Akra should keep a narrower runtime boundary, explicit child-environment policy,
and server-side credential ownership.

## Akra Comparison

| Dimension | Agent Canvas v1.2.1 | Akra baseline | Verdict | Evidence |
| --- | --- | --- | --- | --- |
| Product center | multi-agent browser control center | Codex-native operator and delivery layer | different | [Canvas](evidence.md#product-packaging-and-boundary), [Akra](evidence.md#akra-baseline-evidence) |
| Selected session | cohesive chat/files/diff/log/browser/task inspector | native inline TUI with focused overlays and Admin detail | Canvas ahead in inspector cohesion | [Canvas](evidence.md#selected-conversation-ux), [Akra](evidence.md#akra-baseline-evidence) |
| Fleet oversight | filterable conversation rail, coarse status | task/agent/worktree/delivery topology, weak current activity | split | [Canvas UX](evidence.md#selected-conversation-ux), [Akra](evidence.md#akra-baseline-evidence) |
| Codex runtime | Agent Server plus released embedded-core ACP bridge | direct official `codex app-server` adapter | Akra structurally ahead; end-to-end fidelity unverified | [ACP](evidence.md#released-codex-acp-path), [Akra](evidence.md#akra-baseline-evidence) |
| Permissions | outer policy off; ACP option zero auto-selected | direct app-server boundary, but only accept/decline; file-change approval and MCP elicitation fail closed without forms | Akra structurally ahead; choice fidelity incomplete | [ACP](evidence.md#released-codex-acp-path), [Akra](evidence.md#akra-baseline-evidence) |
| Isolation | attached workspace defaults local; scratch falls back to a worktree request; no cleanup found | leased worktree per parallel lane | Akra ahead for delivery authority | [Canvas](evidence.md#conversation-creation-reconnect-and-workspaces), [Akra](evidence.md#akra-baseline-evidence) |
| Delivery | Git/PR actions are prompts | shipped frozen range, review/check policy, integration verification, cleanup; exact-SHA validation planned | Akra ahead, proof gate incomplete | [Canvas](evidence.md#selected-conversation-ux), [Akra](evidence.md#akra-baseline-evidence), [planned validation](../jcode/gap-matrix.md#p0-frozen-source-validation-evidence) |
| Remote operation | local/remote/cloud backend registry and health | loopback Admin plus Telegram control plane | Canvas ahead | [Canvas](evidence.md#backend-registry-and-telemetry), [Akra](evidence.md#akra-baseline-evidence) |
| Automation | schedules, webhooks, durable runs, partial Canvas UI | planning tool and parallel tick, no trigger/run ledger | Canvas ahead | [Canvas](evidence.md#automation-114), [Akra](evidence.md#akra-baseline-evidence) |
| Secrets | backend key in local storage; broad ACP child environment | filtered app-server child environment; no remote registry | Akra structurally ahead; remote design pending | [Canvas](evidence.md#backend-registry-and-telemetry), [Akra](evidence.md#akra-baseline-evidence) |
| Performance | no comparable product benchmark found | no complete native performance contract yet | unknown | [limits](evidence.md#audit-limits), [planned contract](../jcode/gap-matrix.md#p0-native-performance-evidence-contract) |
| Release validation | full Ubuntu JS gates, Windows app build, conditional E2E | Rust format/test/clippy and native platform-targeted jobs | different, no single winner | [Canvas](evidence.md#quality-and-validation), [Akra](evidence.md#akra-baseline-evidence) |

## Decisions

### Adopt

- bounded per-agent current activity and last-activity age in the existing TUI/Admin parallel
  projections;
- selected-run deep links across trigger, task, agent session, app-server thread, validation, PR,
  integration, and cleanup;
- schedule, signed event trigger, enable/disable, run-now, cancellation, and durable run history for
  reviewed Codex code-change intake;
- later, read-only remote Akra node health, capability, version, and stale-state visibility.

### Reject

- ACP or a multi-provider runtime as Akra's core;
- a Monaco, xterm-log, or browser-screenshot clone as a competing browser IDE;
- generic Slack, Notion, and arbitrary workflow marketplace scope;
- prompt insertion as authoritative Git or delivery control;
- optional worktree isolation for parallel delivery;
- browser local storage for remote node credentials;
- automatic permission selection.

### Differentiate

- make `event -> accepted task -> leased worktree -> exact-SHA validation -> reviewed PR -> verified
  integration -> cleanup` one durable application trace;
- project the same truth through TUI, Admin, CLI, Telegram, and later remote read models;
- make Admin game motion, packets, blockers, review waits, and progression consequences of durable
  state transitions rather than decorative continuous activity;
- retain direct official app-server schema accountability and measure field fidelity rather than
  adding protocol relays.

## Refresh Triggers

Refresh this analysis when:

- Agent Canvas replaces its pinned Agent Server 1.33.0 or ships the maintained app-server-based
  Codex ACP path;
- Canvas or Automation fixes the six-state run contract, adds replay deduplication, or changes its
  credential model;
- Canvas adds released fleet topology, delivery authority, worktree cleanup, or interactive
  workspace tools;
- Akra ships the activity envelope, truthful diorama, automation intake, or remote node registry;
- a same-machine authenticated benchmark or fault-injection matrix is available.

## Sources

See [evidence.md](evidence.md) for pinned repositories, line-level source links, commands, observed
release-gate output, and unverified experiments.
