# Last-turn source acceptance (#2948)

This scoped successor starts at #3043,
`1f298e46719f806a3e135cb2f5859067552e9e99`.
It uses received turn data only. It does not resolve historical content from
HEAD, index, working files, refs, or a remote repository.

## Data and contract

`transcript.ts` already retains `turn/diff/updated` and applied `fileChange`
items. `turnDiffSources` now keeps each original payload beside its display
model. It preserves the existing choice: an aggregate turn patch takes
precedence; absent or unrecognized aggregates use completed file-change items.

`captureTurnDiff` returns an immutable `last_turn` snapshot. Its SHA-256
identity binds the host endpoint, thread, turn, cwd, source item/aggregate,
old/new paths, content kind, and original payload. Scope property order cannot
change identity, and a caller cannot relabel a context response as a Git source.

- Complete add/delete item content retains CRLF and final-newline absence.
- Patch/update content contributes only supplied, count-checked hunk lines.
- Partial patches remain partial. No missing unchanged lines are invented.
- An incomplete hunk contributes no context; earlier verified hunks remain usable.
- A request containing any missing line returns
  `unavailable: context_not_provided`, without a successful partial excerpt.
- Source input is capped at 1 MiB. Context responses are capped at 200 lines and
  64 KiB. Binary input, invalid overlapping coordinates, wrong snapshot IDs,
  and range-limit violations are refused.

The turn's existing changes card exposes old/new range reads and their
availability. Captures are lazy per opened file. Late captures cannot replace
the current component's source. The normal working-tree reader, attachment
storage, manifest receipts, and submission paths are unchanged.

## Reproducible evidence

```text
cd desktop
npm run check
npm test -- --run test/turnDiffSnapshot.test.ts test/diff.test.ts test/transcript.test.ts
npm run build
node --experimental-strip-types --disable-warning=ExperimentalWarning --disable-warning=MODULE_TYPELESS_PACKAGE_JSON test/e2e/last-turn-source.mjs
```

For a separately installed Electron 45.2.3, set `AKRA_ELECTRON_EXECUTABLE`.
Set `AKRA_TURN_EVIDENCE` to choose the capture directory. This run uses
`C:/dev/akra-queue/last-turn-evidence-st01a21ea6`.

The unchanged feature passed its 35 targeted tests, typecheck, build, and native
runner shard `27/50` on parent `bf531b409`. The planner's Windows Admin checks
passed (1,562 tests, 8 existing skips), but its final receipt rejected generated
input drift. The refreshed parent `1f298e467` adds only `.gitattributes` and the
admin promotion fix for that drift. Per the integration owner's instruction,
unaffected completed QA was retained rather than repeated after this base change.

The native fixture uses the real Electron client, transcript reducer, source
reader, and a disposable Git repository. It replays the existing recorded
`thread/resume` schema with completed file-change data, then separately replays
an actual `turn/diff/updated` notification. No source reader is mocked.
The aggregate restart case replays the same supplied notification. It does not
claim new aggregate persistence; history without that notification uses the
existing completed-item path.

For both source paths it reads exact old lines 10–11 and new lines 20–21,
changes the working file to unrelated text, and reads the same prior excerpts
again. It closes and reopens Desktop and checks the same snapshot identity and
excerpts. A request including unprovided line 19 returns unavailable and removes
the previous successful excerpt. The index remains unchanged, and the Gateway
records zero `turn/start` or `turn/steer` requests.

The unit fixture also changes HEAD and index after capture. Mutation checks
proved the fixtures fail when missing lines are falsely called available
(eight failures), or new-side requests read old-side data (one failure).
Those mutations are restored. Identity-order and caller-metadata regressions
were reproduced before their corrections.

This is the issue's recorded last-turn source acceptance. Branch/commit/PR
revision-pair expansion and new remote reads remain outside this increment.
It does not add a second manifest or draft store.
