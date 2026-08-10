# Admin PR validation evidence store validation — 2026-08-11

## Scope

This artifact validates the durable PR-validation rollout evidence path introduced for the Admin
control center. The test target was the production composition, not the isolated debug harness.
The Admin server read the checked-in rollout artifact, projected it through the application
service, and persisted only the bounded/redacted snapshot in the repository-scoped authority
SQLite store.

## Verified behavior

- The evidence drawer reported collection revision `1`, state `cooldown`, outcome `collected`,
  zero stale-lease recoveries, and zero identity conflicts.
- The production evidence page retained one valid history row and exposed three text-first trend
  comparisons: Actual Fast Gate, CI Gate, and Post-Merge Gate.
- The typed warning surface reported `insufficient_actual_samples` for the single independent
  Actual Fast Gate run.
- The dashboard bootstrap response remained summary-only. Full history, collection ownership,
  trends, and warnings were fetched only after opening the evidence drawer.
- A 15-second live observation kept the drawer at 444 DOM nodes, one history row, and a stable
  1,416-pixel scroll surface. The realtime state remained `live`; no console warnings or errors
  were emitted.
- Background transport has an explicit visibility lifecycle: a hidden document closes its
  `EventSource` and skips dashboard/event fallback polls; visibility recovery performs one fresh
  dashboard/event reconciliation and reconnects the stream.

## Payload measurements

| Surface | UTF-8 payload | Notes |
| --- | ---: | --- |
| Dashboard bootstrap | 9,708 bytes | Latest rollout summary only |
| Evidence detail (`limit=10`) | 9,671 bytes | One durable row, three trends, one typed warning |

The bounded store retains at most 64 snapshots, while the public page contract accepts at most 20
history rows per request. Cursor ordering is `sort_at DESC, artifact_sha DESC`, and stale cursors
fail closed instead of silently returning a shifted page.

## Automated coverage

- two SQLite-backed collectors racing for the same scope produce exactly one lease owner;
- an expired lease is recovered and the old owner's settlement is rejected by CAS;
- duplicate evidence identities do not create rows;
- an identity collision preserves the first snapshot and increments a typed conflict counter;
- retention prunes the oldest rows after the 64-row bound;
- a restarted query service replays durable history and preserves the last valid snapshot when
  the source becomes unavailable;
- a durable-store outage degrades to the existing read-only filesystem projection without taking
  down the Admin dashboard;
- SSE revision regressions and duplicate revisions do not rewind evidence or validation details;
- hidden-tab transport and visible-tab recovery are protected by a static Admin client contract.

## Data boundary

The SQLite adapter never receives raw GitHub/provider JSON. It stores application-validated DTO
JSON plus minimal identity metadata (`repository`, `base`, full evidence SHA, and generated-at), a
content hash, and typed collection state. Provider error strings and local paths are not persisted
or returned to the browser.
