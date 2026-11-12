# Hide

## Snapshot

| Field | Value |
| --- | --- |
| Official repository | <https://github.com/with-hide/hide> |
| Site | <https://withhide.dev> |
| Release | [v1.2.7](https://github.com/with-hide/hide/releases/tag/v1.2.7) |
| Source commit | [`171719601dd6874f0113751335a9362d62405e3f`](https://github.com/with-hide/hide/tree/171719601dd6874f0113751335a9362d62405e3f) (`main`, 2026-11-12) |
| License | FSL-1.1-ALv2 |
| Audit date | 2026-11-12 (Asia/Seoul) |
| Previous Akra pin | none (first audit) |
| Akra baseline | `a6cc327e` (`prerelease`) |

Evidence is source and public-document inspection of the pinned commit, the site, and one 13-second
screen recording of the mission map that the Akra operator supplied. The application was not
installed or run.

## Verdict

Hide is a desktop "agentic development environment" that runs the vendor agent CLIs a person already
signed in to (Claude Code, Codex, Antigravity, OpenCode, Grok, Pi, omp) in real terminal panes, and
adds a core that decides which agent needs a person and why. Its threat is not the terminal fleet,
which Orca and Herdr also have. It is the attention model on top: one list grouped by whose move is
next, one line that says why an agent stopped, per-node finish lines, and a human gate.

Akra's edge stays the engine contract. Hide reads agent state from three ranked sources (the agent's
hook, its session file, then the terminal screen) and needs screen rules, composer reads, and
doorbell retries to deliver input safely. Akra reads typed `codex app-server` events and publishes a
machine-readable kernel spec with reasons, next steps, and idempotent commands.

## Strengths

- **Five-axis status, decided once.** Demand, activity, completion, read, and ownership are separate
  axes (`docs/status-model.md`, `hide-core/src/agent_state/axes.rs`). The core derives four groups
  (Needs You, Unread, Working, Seen) and nine row marks, and no screen derives its own. A demand
  leaves Needs You when it is answered, not when it is looked at. Badges and notifications count
  only unread asks.
- **Finish lines per mission node.** A node carries its own criteria (at most three), the plan's check
  command, the pull request's CI, reviews, verifications, defects, and one human gate
  (`hide-core/src/mission_criteria.rs`). A line is recorded by its owner only; the core refuses a
  node's done while a line is unmet.
- **Human gate.** A plan names one place the operator looks before a node is done. Only the
  operator's answer passes it; no gate is passed by waiting.
- **A parent watches its child.** A child that stops on a question is told to its parent within a
  minute; 20 quiet minutes raise a warning (`docs/ARCHITECTURE.md`, "Local agent delivery and
  inactivity watches").
- **Strays.** The Project tool lists worktrees where work ended unfinished: uncommitted changes,
  unpushed commits, or a pull request whose CI failed, where no agent works or waits
  (`docs/UI_BEHAVIOR.md`, "The Project tool"; `hide-core/src/runtime/strays.rs`). Each row offers
  "finish it" and "leave it".
- **Pull requests ordered by who acts next,** with one click to hand a pull request to an agent.
- **Project Memory.** Decisions become cards that the next agent starts with, across vendors.
- **Agent-driven panes.** `hide browser open|click` and `hide file open` are CLI commands behind one
  installed skill, so an agent checks its own page before it asks a person.
- **Reach.** Signed macOS build, Windows and Linux packages, self-update, SSH devices in one window,
  and a phone client over an end-to-end encrypted relay.

## Limits

- **Dense and concept-heavy.** The same work appears in the task list, the terminal pane, and the
  mission map; the operator learns missions, nodes, Conductors, letters, Views, and finish lines.
- **The mission map costs more than it returns at small scale.** In the recording the operator pans
  and zooms a seven-node map; a list reads faster below roughly six nodes.
- **Raw terminal as the conversation surface.** Tables and diffs are terminal text.
- **State inference.** Hook, session-file, and screen readings can disagree; a large part of
  `docs/status-model.md` and `docs/delivery.md` exists to rank and reconcile them.
- **Size.** About 953k Rust lines and 198k TypeScript lines including tests; `docs/ARCHITECTURE.md`
  alone is 1.2 MB.
- **Private design authority.** The core design of missions, plans, task identity, and Memory lives
  in a private repository; the public guides keep only pointers.

## Akra Decisions

### Adopt

- A kernel-owned attention item list with a closed cause list, where the unit is an item (thread,
  task, pull request, or repository) and not a thread.
- Demand and read as separate axes; an item leaves the list when it is answered.
- One reason line per row from the cause code, without a model call.
- A completion-criteria card that shows only machine-recorded lines first.
- A strays list, pull requests ordered by who acts next, and a usage-limit readout.

The proposed design, its order, and the Akra code facts behind it are in
[plan 23](../../plan/23-akra-attention-items.md).

### Reject

- Copying Hide source. FSL-1.1-ALv2 forbids use in a competing product; Akra takes ideas only.
- The IDE scope (editor, Explorer, terminal multiplexer), the phone relay, SSH devices, and
  per-vendor account profiles.
- The mission map while Akra runs a three-slot pool.
- Reading agent state from a terminal screen.

### Differentiate

- Typed app-server events instead of hook, session-file, and screen inference.
- A kernel spec a client can execute: `reasons`, ordered `next` steps, `idempotency_key`, `describe`.
- Reviewed delivery with a frozen base, exact reviewed range, and post-merge PR validation.

## Sources and Evidence Gaps

- Source at the pin: `README.md`, `docs/status-model.md`, `docs/ARCHITECTURE.md`,
  `docs/UI_BEHAVIOR.md` (Project tool section only), `docs/README.md`,
  `hide-core/src/mission_criteria.rs`, `contracts/snapshot-wire-enums.json`.
- Not verified: runtime behavior, performance claims, the phone relay, and anything owned by the
  private `with-hide/hide-private` documents.
- Line counts come from `wc -l` over the checkout without `vendor/`, and include tests.
