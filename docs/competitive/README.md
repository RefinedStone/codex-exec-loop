# Competitive Research

This directory is the durable evidence base for products that compete with, wrap, or materially
shape Akra. Each analyzed product owns one directory. The goal is not a feature checklist. The goal
is to decide what makes Akra worth choosing as a Codex-first operator runtime and to turn verified
gaps into testable product work.

## Product Position

Akra should not become another broad model harness. Its defensible position is:

> Akra is the Codex-first operating and delivery layer that turns official `codex app-server`
> sessions into durable, inspectable, review-delivered work.

That position has three required pillars:

1. **Protocol-native interaction**: expose useful Codex capabilities quickly and faithfully through
   a high-density native TUI.
2. **Long-horizon delivery**: persist planning authority, isolate parallel work, deliver through
   commits and pull requests by default, and incorporate valid review feedback.
3. **Operator control**: project the same application truth through TUI, Admin, CLI, Telegram, and
   automation instead of building disconnected frontends.

The reviewed path is the product default, not an absolute current invariant. When a parent process
supplies the explicit high-risk autonomous-delivery opt-in, the runtime distributor may bypass
review/check gates while retaining a PR, and eligible PR modes may skip PR automation entirely.
Competitive conclusions treat those paths as exceptions and require them to be projected with
review-skipped policy provenance, not counted as reviewed delivery.

Competitive work should sharpen or disprove this position. It must not silently turn provider
breadth, tool count, or UI surface count into the product goal.

## Coverage

| Product | Snapshot | Status | Primary threat |
| --- | --- | --- | --- |
| [jcode](jcode/analysis.md) | v0.43.0, `649276753ae11948759192c067dfc4c90fafd47f` | current as of 2026-07-12 | native TUI performance, multi-session runtime, memory, swarm |

Next research order:

1. OpenHands Agent Canvas: visual parallel supervision and Codex/ACP harnessing.
2. OpenCode: TUI, desktop, IDE, client/server API, extensibility, and child-session navigation.
3. Upstream Codex: mandatory baseline for capabilities Akra should project rather than rebuild.
4. Claude Code, Amp, Pi, Aider, Cline/Roo, Cursor, and Windsurf: closed and open harness patterns
   after the three direct architectural comparisons above.

Do not create an empty product directory. Add it only when its snapshot and first evidence ledger
are ready to review.

## Required Files

Every product directory must contain:

- `analysis.md`: product thesis, architecture, UX, performance, safety, strengths, and weaknesses.
- `evidence.md`: immutable snapshot, source ledger, local inspection commands, and evidence limits.
- `gap-matrix.md`: Akra-relative findings, adopt/reject/differentiate decisions, and testable work.

Use [_template.md](_template.md) as the minimum contract. More files are allowed only when the
source volume or a separately reviewable experiment requires them.

## Evidence Classes

Every consequential statement should be classifiable as one of these:

| Class | Meaning | Allowed conclusion |
| --- | --- | --- |
| `verified` | inspected in source, a release artifact, or locally reproduced output | may support a product or implementation decision |
| `documented` | stated by the product's official documentation but not reproduced | may support a hypothesis, not a performance claim |
| `proposed` | explicitly marked design, proposed, experimental, beta, or incomplete | may inform direction; must not be scored as shipped |
| `inferred` | reasoned from multiple verified facts | must state the inference and its uncertainty |
| `unverified` | marketing claim, missing raw data, inaccessible source, or ambiguous behavior | may be recorded only with the limitation visible |

Screenshots and demos prove presentation, not correctness, latency, durability, or release support.
Code presence proves an implementation exists, not that the public product enables it by default.

## Comparison Dimensions

Each analysis must cover the dimensions that apply:

- product audience and reason to choose it
- runtime authority and client/server topology
- provider, model, tool, MCP, skill, and plugin boundaries
- session continuity, context compaction, memory, and provenance
- TUI information architecture, input ergonomics, rendering, and terminal compatibility
- desktop, web, Admin, remote, mobile, and automation surfaces
- parallel execution, isolation, coordination, completion, and integration
- permissions, sandboxing, secrets, unattended work, and auditability
- startup, input, streaming, rendering, memory, and build performance
- tests, release validation, maintainability, and architecture debt

Performance comparisons require process-tree accounting, exact versions, authentication state,
warm/cold state, terminal geometry, run count, raw samples, and an environment stamp. A mean or
best-case number without those fields is not comparable evidence.

## Akra Baseline

An analysis must pin the Akra commit it compares against and read current source, not only product
docs. At minimum inspect:

- `src/core/`
- `src/application/service/`
- `src/application/port/`
- `src/adapter/inbound/tui/`
- `src/adapter/inbound/admin_api/`
- `src/adapter/outbound/app_server/`
- `src/application/service/parallel_mode/`
- `docs/supersession/current-contract.md`

Conflicts between Akra documentation and code are findings, not details to smooth over.

## Decision Rules

- **Adopt** only when the pattern improves Akra's chosen product position and maps to an owned
  boundary.
- **Reject** when copying it would duplicate `codex app-server`, weaken safety, or broaden the
  product without improving the operator loop.
- **Differentiate** when Akra already has a stronger primitive, such as worktree-isolated delivery
  instead of same-checkout agent coordination.
- Every accepted gap needs a proof target: test, benchmark artifact, terminal capture, API response,
  persisted state transition, or merged delivery path.
- Keep speculative backlog outside this directory. A work item belongs here only as a consequence
  of an evidence-backed comparison.

## Refresh Policy

Refresh an app analysis when any of these occurs:

- a new major release or architecture migration
- a release that changes the compared TUI, runtime, parallel, memory, or control-plane behavior
- the snapshot is older than 90 days and is still used to justify active work
- an Akra implementation slice closes or invalidates a recorded gap

Preserve old conclusions through git history. Update the snapshot and evidence ledger in place so
the current directory remains the decision source.
