# Admin PR Validation Operations Evidence

This artifact records deterministic browser validation for the PR validation operations slice.
The server ran with the application-owned `--debug-harness`; it did not mutate GitHub, production
SQLite state, or the ordinary Planning Queue.

## Environment

- date: `2026-08-10` (Asia/Seoul)
- branch: `codex/admin-pr-validation-operations`
- surface: authenticated Akra Admin dashboard in the Codex in-app browser
- viewport: `1280x720` for the focused interaction captures
- transport exercised: dashboard JSON, typed validation mutation endpoints, SSE invalidation,
  DOM reconciliation, and `AkraAdminGame.inspectScene()`
- browser console: zero warnings and zero errors after the completed flow

## Ten-Scenario Semantic Audit

For every scenario, the dashboard API record key/revision/phase was compared with the rendered
Validation Rail row and the Pixi inspection projection. Passive validation states also asserted
zero active remediation actors and no worker lease.

| # | Scenario | Stages | Initial semantic phase | Revision | DOM/Pixi | Passive actor invariant |
| --- | --- | ---: | --- | ---: | --- | --- |
| 1 | Post-Merge success | 4 | Integrated | 4 | pass | pass |
| 2 | check failure -> Queue -> revalidation | 8 | Integrated | 5 | pass | pass |
| 3 | optional skipped | 3 | Integrated | 6 | pass | pass |
| 4 | required missing/skipped | 3 | Integrated | 7 | pass | pass |
| 5 | rate limit recovery | 5 | Integrated | 8 | pass | pass |
| 6 | provider outage + RetryNow | 5 | Integrated | 9 | pass | pass |
| 7 | closed-unmerged + distributor attestation | 5 | Blocked | 13 | pass | pass |
| 8 | restart during Verifying | 5 | Integrated | 14 | pass | pass |
| 9 | two-process claim race | 4 | Integrated | 15 | pass | pass |
| 10 | duplicate event + late review | 5 | Integrated | 16 | pass | pass |

## End-to-End Recovery Flow

Scenario 2 was advanced through all eight stages. The observed semantic phases were:

```text
Verifying
-> Verifying
-> RemediationQueued
-> RemediationQueued
-> RemediationRunning
-> Verifying
-> Verified
```

Assertions:

- the failure packet moved toward Queue before any worker appeared;
- Queue admission alone kept actor count at zero and worker lease false;
- `RemediationRunning` showed exactly one actor, one active worker lease, and a
  `queue_to_worker` packet;
- revalidation removed the worker and returned the scene to passive verification;
- completion reported `Verified` in API, DOM, and Pixi inspection;
- the Actions column reported required-check attempt `2`, not scheduler poll attempt `8`;
- the dynamic harness counter reported `1/8` after selecting the eight-stage scenario.

## Typed Command Audit

The browser exercised Queue Remediation, Pause, Resume, and provider RetryNow through their typed
application commands.

- applied command feedback survived the mandatory fresh dashboard/detail reconciliation;
- stale/disabled reasons were projected through command availability rather than guessed in JS;
- rate-limit blocked RetryNow with the `rate-limit reset` reason;
- provider outage enabled RetryNow and advanced to recovery;
- pause/resume changed revision and command availability without deleting the record;
- command status exposed the machine-readable `data-validation-command-outcome="applied"` state.

## Layout and Accessibility Audit

- the detail drawer traps its own long content in an internal scroll region instead of spilling
  beyond the viewport;
- the scenario title wraps rather than truncating its meaning;
- Validation Rail rows remain keyboard buttons with expanded state and dialog association;
- phase meaning is repeated in text/icon copy and does not rely on color alone;
- the game actor is only a lease projection; passive CI does not fabricate a worker.

## Captures

- [`01-remediation-worker-wide.png`](01-remediation-worker-wide.png): one real harness worker lease
  at `RemediationRunning`.
- [`02-validation-detail-running.png`](02-validation-detail-running.png): the running validation
  detail drawer with bounded internal scrolling.
- [`03-command-feedback.png`](03-command-feedback.png): correlation details and persistent typed
  command feedback.
- [`04-verified-wide.png`](04-verified-wide.png): the completed `Verified` rail and passive QA/CI
  station.

The `responsive/` subdirectory is produced by `scripts/check_admin_graphic_visual.sh` with
`ADMIN_GRAPHIC_DEBUG_HARNESS=1` and contains the mobile, compact, wide, Full HD, and QHD visual
contract captures.

The final responsive gate passed at `390x844`, `1280x800`, the standard wide viewport, Full HD,
and QHD. The mobile assertion verifies a hidden table header, a two-column Validation Rail card,
no global horizontal overflow, and at least 38px controls. The Impeccable detector was run once on
the completed UI slice: its side-tab and layout-width transition warnings were removed. The one
remaining advisory concerns the dashboard's deliberate blueprint grid, which is retained because
this surface is the actual game/map canvas context named by the detector's exception.

## Reproduction

```powershell
$env:ADMIN_GRAPHIC_CAPTURE = 'always'
$env:ADMIN_GRAPHIC_DEBUG_HARNESS = '1'
$env:ADMIN_GRAPHIC_OUTPUT_DIR = 'docs/validation/artifacts/admin-pr-validation-operations-2026-08-10/responsive'
bash scripts/check_admin_graphic_visual.sh
```

The deterministic HTTP scenario suite is also covered by
`admin_debug_harness_all_validation_scenarios_keep_board_and_scene_semantics_aligned`.
