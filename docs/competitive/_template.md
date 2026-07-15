# Product Analysis Template

[한국어](../ko/competitive/_template.md)

Replace every placeholder before review. Delete non-applicable sections only with a short reason.

## Snapshot

| Field | Value |
| --- | --- |
| Product | `<name>` |
| Official repository/site | `<url>` |
| Release/tag | `<release>` |
| Source commit | `<full sha or closed-source limitation>` |
| Release date | `<ISO date>` |
| Audit date | `<ISO date>` |
| Akra baseline | `<full prerelease sha>` |
| Auditor environment | `<OS, architecture, terminal when relevant>` |

State whether the checkout is public, locally available, and clean. For closed-source products,
list the exact binaries, versions, and public docs used instead.

## Evidence Discipline

Define which findings are `verified`, `documented`, `proposed`, `inferred`, or `unverified` under
the parent [evidence rules](README.md). Link the product's `evidence.md` ledger.

## Executive Verdict

In five to ten sentences:

- identify the product's real reason to choose it
- name the two or three strongest threats to Akra
- name the structural weakness Akra should exploit
- state what Akra must adopt, reject, and differentiate

## Product And Audience

- primary operator and workflow
- product surfaces
- distribution and supported platforms
- pricing/auth/provider assumptions when material

## Architecture

- runtime authority
- process and client/server topology
- persistence and protocol
- module/package boundaries
- reconnect, resume, and failure recovery

Separate current implementation from proposed architecture.

## Harness And Context

- provider/model routing
- tools, MCP, skills, commands, plugins, and hooks
- context assembly, compaction, session search, and memory
- permission and unattended execution rules

## TUI And UX

- first-run and submit loop
- transcript and tool activity presentation
- session navigation and parallel-work visibility
- prompt editing, command palette, approvals, diffs, and recovery
- narrow terminal, scrollback, resize, input, and rendering behavior

## Admin And Remote Surfaces

- web/desktop/admin/mobile status
- whether they share application truth or duplicate runtime logic
- operational metrics, audit history, and remote actions

## Parallel Work

- isolation unit
- assignment and coordination
- conflicts and file ownership
- completion proof, integration, review, and cleanup

## Performance

For each published or reproduced number include:

- exact binary version and commit
- machine and terminal environment
- warm/cold and authenticated/unauthenticated state
- process-tree boundary
- run count and raw samples
- measurement definition

Mark unsupported claims `unverified`. Do not normalize incompatible metrics into one score.

## Quality And Risk

- test and validation shape
- release gates
- architecture and size debt
- security and credential surface
- documented design that is not demonstrably shipped

## Akra Comparison

Use evidence-backed rows rather than a feature dump:

| Dimension | Product | Akra | Verdict | Evidence |
| --- | --- | --- | --- | --- |
| `<dimension>` | `<observed state>` | `<current state>` | `ahead / behind / different / unknown` | `<links>` |

## Decisions

### Adopt

Patterns that fit Akra's Codex-first boundaries, with the owning Akra modules.

### Reject

Patterns that duplicate runtime authority, weaken safety, or add surface without product leverage.

### Differentiate

Capabilities where Akra should deliberately offer a stronger or narrower alternative.

## Implementation Slices

Each accepted slice must include:

| Priority | Slice | Owned boundary | User outcome | Required proof |
| --- | --- | --- | --- | --- |
| `P0` | `<name>` | `<files/modules>` | `<observable result>` | `<tests/benchmark/capture/state>` |

Do not call a slice complete from a unit test that covers only one layer of a cross-surface claim.

## Refresh Triggers

List product releases, Akra changes, and missing experiments that would invalidate the analysis.

## Sources

Keep the detailed immutable source ledger and inspection commands in `evidence.md`.
