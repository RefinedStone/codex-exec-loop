# Diff context and navigation

This #2949 increment is based on #3010, commit
`a75f634e59aff1f54feb098f76d123c48c67d58e`. #3010 is directly above #2991.
Nothing in this stack has been merged by this task.

## Delivered surface

- Comparison and known aggregate counts stay visible while the file list scrolls.
- File navigation opens one target file. Manual file toggles still allow multiple
  files. This prevents keyboard navigation from rendering every previously visited file.
- Search names its scope: the selected side of the current file's loaded patch.
  Previous/next result and hunk actions use actual old/new file coordinates.
- Pierre selection supports gutter click, Shift extension, and pointer drag.
  The range controls provide a keyboard path. Split/unified and wrapping preserve
  the same snapshot, side, and file lines.
- Selected original context is requested from #3010 in bounded windows. Expansion
  reads another ten surrounding lines from that same snapshot. A later file edit
  reports stale and retains the original text. Expired or unavailable source
  never falls back to the current file.
- Context whitespace markers change presentation only. The stored source remains
  unchanged. Selection refuses missing original content and ranges over 200 lines.
- Large plain diffs render at most 1,000 display rows per page. The existing
  5,000-row syntax-rendering cutoff and 1 MiB source limits remain.
- Git hunk mutations have a separate, explicitly opened action section. Selection,
  navigation, search, and context expansion do not change Git or send a message.

`SnapshotDiffView` exposes `selection` and `onSelectionChange` using
`DiffSelection` (`snapshotId`, `source`, `path`, `originalPath`, `side`,
`startLine`, `endLine`). A controlled selection for another snapshot is refused.
#2827 must bind that selection to its own target and durable attachment contract.
This viewer does not create a second draft store.

## Reproduction and evidence

The native fixture uses real Git and Electron 45.2.3 with a fake Gateway. It
does not call a model. Git numstat independently verifies 100 files and 10,000
changed rows. One file contains 2,000 changed rows. A later fixture verifies a
6,000-line plain diff, a file above 1 MiB, and binary content.

```text
cd desktop
npm run check
npm test -- --run test/diffCoordinates.test.ts test/workingTreeNavigation.test.ts
npm run build
node --experimental-strip-types --disable-warning=ExperimentalWarning --disable-warning=MODULE_TYPELESS_PACKAGE_JSON test/e2e/diff-navigation.mjs
node --experimental-strip-types --disable-warning=ExperimentalWarning --disable-warning=MODULE_TYPELESS_PACKAGE_JSON test/e2e/hunk-actions.mjs
```

Set `AKRA_ELECTRON_EXECUTABLE` when using an existing complete Electron runtime.
Set `AKRA_DIFF_EVIDENCE` to choose the screenshot and timing directory.
The local run used `C:/dev/akra-queue/diff-evidence-st01a21ea6`.

The fixture asserts old `INDEX_ONLY` independently of current `WORKTREE_40`,
exact unchanged line 20, stale retention, click/Shift/drag selection, mode/wrap
retention, keyboard navigation, bounded rendering, and unchanged index content.
It also checks that panel zoom leaves the sidebar width and tab-header height
unchanged. Screenshots are local evidence, not committed product assets.

The 4K captures use a 3840 by 2160 Electron window. The 150% full-window capture
uses Electron zoom and native `capturePage`; it is not proof of changing Windows
OS DPI. The panel-only 150% capture uses the shipped `panel:review` zoom control.
The narrow-window capture exposes an existing top-bar overlap outside this
diff-panel change. The new diff controls wrap without overlapping.

Final native measurements on Windows, Ryzen 9 7900X3D, Electron 45.2.3:

| Measurement | Result | Method |
| --- | --- | --- |
| Warm syntax engine, fresh 2,000-change-row file | 941.557 ms | File-open action to the last changed row being rendered |
| Composer input p95 | 18.772 ms | 20 fills during file loading; includes automation IPC |
| File-navigation response p95 | 13.500 ms | 20 keyboard activations, activation click to two animation frames |
| File-navigation automation p95 | 54.559 ms | The same 20 actions measured outside Electron |

The fixture verifies that navigation has exactly one expanded target after each
activation. Earlier development runs observed first-display times from 916.560
to 1,168.223 ms. The proposed one-second target is therefore not a consistent
guarantee on this host, even though the final run met it. Raw samples and exact
measurement definitions are in `timings.json`.

The plain-diff fixture keeps exactly 1,000 DOM rows before and after pagination,
then reads actual file line 6,000 from its original snapshot. The oversized-line
case initially reproduced horizontal overflow of the entire panel. Constraining
the file-list grid to `minmax(0, 1fr)` fixes the intrinsic-width cause; the native
test now asserts that only the code view can scroll horizontally.

Review: each new module owns one display, coordinate, or context responsibility.
UI modules remain below 200 pure lines. The native acceptance fixture is 213
pure lines; extract fixture setup before adding further scenarios. Inputs cross
the existing typed IPC boundary, and line kinds are matched exhaustively.
No casts, suppressions, model calls, production logging, duplicate draft store,
or new dependency were added. Public component props are the selection contract.
The partial-patch coordinate test first failed when unknown rows acquired invented
line numbers. The overflow assertion failed on the pre-fix bundle.

The predecessor-based planner run passed all Windows Admin checks, including
112 Desktop test files and 1,541 tests (8 existing skips). Its final input
fingerprint failed after the admin build changed registered input bytes. This
is the separate Windows build-input problem tracked by #3000, not a passing
planner receipt. The generated admin bundle is excluded from this increment.

## Exact remaining dependency table

The issue states below were rechecked live on 2026-11-27. #2825, #2826, #2827,
and #2947 remain open. This stack does not close them.

| Outcome | Available prerequisite | Exact remaining prerequisite / owner |
| --- | --- | --- |
| #2827 old/deleted-side feedback | #3010 returns immutable old/new ranges and preserves #2991 source/repository/worktree identifiers | #2827 must consume that reader for the selected side. `worktree_file` is never an old-side fallback. |
| #2827 durable feedback cards | #2991 implements file-range drafts; this viewer emits semantic selection | #2826/#2827 must store selected bytes, bounded context, source, side, old/new paths, target, draft revision, instruction and ordering together. |
| #2827 preview and one-request transfer | Existing composer/send/steer/queue paths | #2827 must prove two files and both sides reach one request without replacing existing text, changing order, auto-sending, or mutating Git. |
| #2827 stale and target checks | #3010 `current: false` and source/scope rejection | #2825 manifest binding and #2827 enqueue/dispatch preflight must preserve the original selection after file/index/HEAD/rename or host/thread/worktree changes. |
| #2827 restart, failure, unknown result | #2991 draft persistence and existing request/receipt paths | #2826/#2827 must prove exact-batch recovery, no automatic resubmission, no duplicate click submission, and retention of edits made during submission. Queue persistence is not assumed. |
| #2827 return from a card | Controlled `SnapshotDiffView` selection and immutable source reader | #2827 must open the original file/snapshot and handle cache expiry from retained attachment bytes. No silent remapping to current code. |
| #2827 review lifecycle | Source and display contracts above | #2827 must keep delivery acknowledgement separate from user-confirmed review and test re-review/re-request/archive. A turn ending or line disappearing does not resolve feedback. |
| #2827 IME | Native range controls are keyboard operable | The feedback editor must prove Korean IME Enter/Escape behavior and preservation of a cancelled instruction. That editor belongs to #2827. |
| #2947 readable local diffs | #2984/#2986 plus #3010 and this #2949 increment | Historical-turn snapshot capture remains #2948 work. Patch-only historical turns cannot use current local files for missing context. |
| #2947 version-safe feedback handoff | #2825 manifest, #2991 drafts, #3010 source, #2949 selection | Complete the #2827 rows above and the aggregate two-file, both-side, restart and failure scenario. |
| #2947 native acceptance | Reproducible native local fixture and scoped zoom evidence | Actual Windows OS DPI 100%/150% acceptance and the pre-existing narrow top-bar overlap remain unverified/unfixed here. |
| Remote host and branch/commit/PR source | Explicit source/scope boundary | Remote authoritative capture is not implemented. Branch/commit/PR sources remain the issue's declared extension scope. No fetch or checkout fallback was added. |
