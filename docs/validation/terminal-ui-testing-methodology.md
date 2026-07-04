# Terminal UI Testing Methodology

Use this method when native TUI changes affect terminal rendering, history insertion, viewport
state, resize behavior, overlays, prompt editing, or live-tail presentation.

## Current-Stack Default And Compatibility Ownership

Stay on the current Ratatui/Crossterm stack by default. Option A proof hardening is the default
path; do not infer Option B activation from these docs alone. A structural extraction remains
blocked unless the Decision Record explicitly proves the Round 6 trigger evidence.
This contract keeps `invariant × first-class environment × branch family` explicit in repo-facing docs and guards.
The current stack remains the default posture for native runtime proof.
Manual terminal capture stays primitive-sensitive only.
The first-class rendering contract does not replace the broader terminal-baseline rows in
`docs/plan/12-platform-validation-matrix.md`; primitive-sensitive PRs still satisfy any
additional required manual rows there, including the current macOS Terminal.app and iTerm2
baseline rows, until that matrix is revised.


### First-class environment key

- **E1** = Windows Terminal + WSL bash + inline
- **E2** = Windows Terminal + PowerShell + inline
- **E3** = tmux detached PTY + inline
- **E4** = direct Linux terminal + inline

## Compatibility-Tier Ownership Table

| Policy area | Current owner / source | Decision point | First-class default | Fallback / experimental handling | Override mechanism | Downgrade semantics | Proof obligation |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Environment class | approved spec + validation docs | release policy / docs / PR review | E1 Windows Terminal + WSL bash, E2 Windows Terminal + PowerShell, E3 tmux detached PTY, E4 direct Linux terminal | other environments stay explicitly fallback or experimental | none at runtime | non-first-class paths are non-blocking unless they contaminate first-class behavior | first-class invariants are release-blocking; fallback/experimental rows stay representative |
| Default `InlineHistoryRenderMode` | `src/adapter/inbound/tui/app.rs` | app startup / env parse | `HostScrollback` via `InlineHistoryRenderMode::from_env_values(None)` | `ViewportReplay` remains explicit-only | `CODEX_EXEC_LOOP_INLINE_HISTORY_MODE` | replay-only paths stay representative unless release policy promotes them | `HostScrollback` rows are release-blocking; `ViewportReplay` rows stay representative by default |
| Default `HistoryInsertionMode` | `src/adapter/inbound/tui/app/history_insertion.rs` | adapter-local env + terminal detection | `StandardScrollRegion` generally; `NewlineFallback` when `WT_SESSION` is present | non-first-class terminals remain representative-only unless they contaminate first-class behavior | `CODEX_EXEC_LOOP_HISTORY_INSERT_MODE` plus implicit `WT_SESSION` detection | downgrade stays non-blocking unless it changes a first-class claim | both insertion branches need proof wherever they are default or explicitly claimed |
| Terminal primitive behavior ownership | `history_insertion.rs`, `inline_terminal_adapter.rs` | implementation boundary | preserve release-blocking invariants across E1-E4 | representative proof is acceptable for downgraded environments | env override plus explicit manual capture | primitive divergence outside first-class policy must be documented | automated proof always; manual capture only for primitive-sensitive changes |
| Reviewer gate / release semantics | validation docs + PR policy + architecture guards | review / merge | release blocks on first-class invariant failures | fallback/experimental failures do not block unless they contaminate first-class behavior | none | downgrades must be explicit in docs and review notes | first-class rows require pass; fallback/experimental rows require representative evidence |

### Branch-family key

- **B1** = `HostScrollback`
- **B2** = `ViewportReplay`
- **B3** = `StandardScrollRegion`
- **B4** = `NewlineFallback`
- Compatibility-path family summary: `HostScrollback`, `ViewportReplay`, `StandardScrollRegion`, `NewlineFallback`.
- Required Decision Record axes: `bug-class recurrence across compatibility boundaries`, `fallback masking risk`, `future test-growth cost`, `maintainability cost`.

## Responsibility Candidate Summary

| Surface | Keep owning | Candidate extraction / clarification |
| --- | --- | --- |
| `NativeTuiApp` | authoritative conversation/session/planning/runtime state, operator mode state, env-derived mode values | must not grow new terminal-primitive orchestration beyond current state/config ownership |
| Thin terminal layer | terminal lifecycle, scrollback writes, viewport sync, clear/reset, cursor-sensitive effects | may be named more explicitly only if Option B later activates |
| Render/layout boundary | typed render surfaces, append-only stream continuity, titleless live-tail behavior, panel chrome exclusion from host scrollback | must stay distinct from terminal primitive emission and from application/core state authority |
| Shared render transaction model | reconcile history delta, geometry state, back-buffer trust, redraw decision, terminal-side flush ordering | remains a conditional extraction candidate only when the Decision Record proves Round 6 trigger evidence |


## Test Layers

Choose the lowest layer that can expose the bug, but prefer temporal evidence when the failure
depends on redraw order. Use this priority for TUI flow regressions:

1. direct frame recorder: store every rendered buffer, host scrollback, and relevant app-side stream
   state after each draw transaction; assert the rows that must survive in each frame
2. Ratatui `TestBackend`: inspect deterministic in-memory screen and scrollback buffers
3. `insta` snapshot: pin stable full-frame presentation once the flow is already covered
4. vt100 parser: validate real ANSI/cursor/clear behavior when terminal escape handling is the risk

### 1. Pure Projection Tests

Use for line builders, status copy, overlays, prompt composition, and transcript projection.

- no real terminal backend
- deterministic input structs and rendered `Line` output
- assertions for presence, absence, order, truncation, and visible key copy
- snapshots only when layout density is the contract

### 2. Reducer And Runtime State Tests

Use for shell input, command dispatch, streaming state, startup/session lifecycle, and the boundary
between committed transcript and live turn state.

Required assertions:

- completed messages move into committed history state
- streaming deltas remain live until turn completion
- clear and thread switch empty pending/deferred history queues
- command-safe buffering does not mutate transcript during unsafe streaming windows
- resize events request redraw without directly mutating conversation state

### 3. Terminal Primitive Tests

Use for code that writes escape sequences, calls `insert_before`, manipulates scrollback, clears the
screen, or invalidates frame buffers.

Required fixtures:

- fake or test backend that exposes screen contents
- vt100-compatible backend when escape sequences matter
- helpers to render buffer contents into plain strings
- helpers to inspect scrollback separately from the active viewport when supported

Required cases:

- insert one committed history block above the viewport
- insert wrapped lines and clear continuation rows
- insert wide characters and verify stale cells are cleared
- clear visible screen plus scrollback and redraw a clean header
- reset pending history and prove stale lines cannot flush after reset

### 4. Frame And Viewport Transaction Tests

Use for frontend draw loop, viewport mode selection, and redraw-order bugs. When a bug mentions
lost rows, duplicated rows, disappearing history, live-tail drift, prompt movement, scrollback
insertion, frame invalidation, or event-stream retention, add a direct frame-recorder-style test
that captures every draw transaction in the sequence before using snapshots as broad coverage.

Frame recorder assertions should include:

- screen text for the current live viewport
- host scrollback text without live panel chrome
- combined terminal history when the user-visible scrollback contract matters
- app-side event stream or transcript state when runtime state must outlive redraws
- before and after frames named for the user flow that triggered the regression

Required cases:

- `HostScrollback` writes new committed history to host scrollback
- `ViewportReplay` does not write committed history to host scrollback
- `ViewportReplay` stays explicit-only and keeps inline viewport positioning
- shrink/restore frame sequences leave no duplicate live tail, stale rows, or misplaced prompt
- draw-time `Terminal::draw` autoresize cannot append the live tail into host scrollback
- overlay open/close resets live-tail redraw cache
- hidden tail skips redundant frames but redraws on width and height changes
- frame invalidation forces a full repaint after terminal-side scrolling

### 5. Event And Scheduler Tests

Use when changing crossterm event mapping, redraw requests, background ticks, or live activity
pulsing.

Required cases:

- resize maps to a draw request
- focus gain maps to draw and can refresh palette/theme state
- focus lost does not force a frame unless product behavior needs it
- multiple immediate frame requests coalesce into one draw notification
- delayed and immediate frame requests choose the earliest safe draw
- paused/resumed input sources do not steal events from nested terminal programs

### 6. User-Visible Snapshot Tests

Use snapshots for stable surfaces that are hard to validate with a few assertions:

- ready shell
- streaming shell
- viewport replay shell
- queue overlay
- planning editor
- diagnostics/session/help inspection
- narrow-height and narrow-width variants

Snapshot policy:

- keep dimensions explicit in test names or helper calls
- normalize OS-specific paths and terminal capabilities
- avoid snapshots for copy that changes often unless the copy is the contract
- add one targeted assertion near a snapshot for the bug class it protects

## Required Regression Matrix

Every TUI rendering PR should state which rows it touches.

| Area | Required automated proof |
| --- | --- |
| Host scrollback history | pending suffix insert, shifted window insert, no duplicate replay |
| Viewport replay | explicit-only fallback, no host scrollback insert, visible recent transcript, inline viewport contract |
| Resize | shrink/restore frame sequence with no stale rows or duplicated live tail |
| Clear/reset | pending history dropped, viewport reset, fresh header redraw |
| Thread/session switch | old transcript and deferred history cannot leak into new thread |
| Streaming turn | active cell or live delta stays live, final output becomes committed history |
| Overlay | opening overlay clears stale live-tail rows and closing redraws normal tail |
| Parallel event stream | frame recorder proves initial status rows survive later runtime-event redraws without panel chrome in host scrollback; split scrollback/live-tail streams render as a titleless live tail |
| Terminal fallback | standard and fallback insertion modes each update viewport state correctly |

## Architectural Guardrails

- Stream surfaces that can span host scrollback and the live viewport must preserve row continuity:
  no panel title may be inserted between durable scrollback rows and live rows.
- Inline inspection code must use the typed render surface API: `InlineTitledPanel` for ordinary
  titled panels, `InlineScrolledPanel` for ordinary scrolled panels, and `InlineAppendOnlyStream`
  for append-only stream rows.
- Parallel event stream rendering must use the dedicated stream renderer and
  `InlineAppendOnlyStream`, not a generic titled scrolled section with new ad hoc copy.
- A TUI PR that changes stream row retention, scroll offset, title visibility, host scrollback, or
  live-tail chrome must include `tui_testkit::InlineFrameRecorder` coverage for the exact failing
  redraw sequence.
- The architecture tests intentionally check this methodology, the design contract, the shared
  frame recorder, and the named parallel stream regression tests. Update the design first if the
  contract itself changes.

## Manual Capture Contract

Manual capture is required **only** for primitive-sensitive changes: scrollback insertion or host scrollback behavior,
viewport mode behavior, clear or restore behavior, resize-dependent redraw behavior, cursor restoration,
or emitted escape-sequence behavior.

### Required artifact fields

For primitive-sensitive review, the artifact set must distinguish between:
- a **matrix-row capture** counted by `scripts/summarize_native_validation.sh`
- a **supplemental representative capture** that documents branch-family or environment-specific primitive behavior

Current capture helpers emit the shared baseline fields only:
- date
- commit SHA
- OS / distro
- terminal program
- shell
- frontend
- `TERM` when available
- check profile
- generic checklist labels
- result
- notes

When primitive-sensitive review needs more detail than the helpers emit, append manual metadata below the helper output instead of omitting it.
If a field cannot be recovered after capture, record `not recorded` explicitly.
For `HistoryInsertionMode`, reviewers should prefer a concrete value; use `not recorded` only when the artifact is purely supplemental and the named automated proof covers the insertion branch.

Each supplemental primitive-sensitive artifact should record:
- artifact id / file name
- whether it is `counted-row` or `supplemental-unmatched`
- commit SHA
- PR or work item id
- capture date/time
- operator/reviewer initials
- frontend (`inline`)
- environment class (`first-class`, `fallback`, `experimental`)
- terminal program and version
- shell and version
- OS / distro / kernel
- multiplexer state (`none`, `tmux detached PTY`, or other)
- configured `InlineHistoryRenderMode`
- configured `HistoryInsertionMode`
- whether override env vars were used
- check profile / scenario set name
- pass/fail per scenario, or a named automated-proof reference when the artifact is a representative manual addendum
- notes on deviations

### Environment stamp contents

Minimum environment stamp:

- terminal name + version
- shell name + version
- OS / distro
- kernel / platform
- inline frontend
- whether tmux detached PTY is involved
- effective `InlineHistoryRenderMode`
- effective `HistoryInsertionMode`
- relevant env overrides (`CODEX_EXEC_LOOP_INLINE_HISTORY_MODE`,
  `CODEX_EXEC_LOOP_HISTORY_INSERT_MODE`, `WT_SESSION` if relevant)

### Scenario checklist minimums

For a primitive-sensitive change, the artifact must show at least:

1. committed history insertion above live viewport
2. no duplicate replay after redraw
3. shrink then restore with no stale rows / duplicated tail
4. clear/reset path with clean header and viewport recovery
5. thread or session switch with no transcript/history leakage
6. if parallel/live-tail changed: split scrollback/live-tail continuity without panel chrome
   inside host scrollback
7. if fallback insertion changed: fallback-specific proof of viewport state and cursor
   restoration

### Reviewer gate

- A primitive-sensitive PR cannot be approved without manual capture artifacts attached.
- Reviewer must confirm artifact type (`counted-row` vs `supplemental-unmatched`), environment stamp,
  required scenarios, explicit downgrade handling, and updated matrix rows.
- `scripts/capture_native_validation.sh` / `.ps1` do not satisfy the supplemental metadata contract by
  themselves; representative artifacts may need manual augmentation after capture.

### When all four first-class environments are required

Capture all four first-class environments when:

- the change alters shared primitive behavior expected across E1-E4
- the change touches defaulting logic or common adapter code affecting multiple first-class
  classes
- the change changes release-policy claims or downgrade semantics

### When a smaller representative set is sufficient

A smaller representative set is sufficient when:

- the change is primitive-sensitive but isolated to one branch family or one environment-specific
  default
- the changed logic is clearly scoped to a single first-class environment plus one
  representative alternate path
- reviewer agrees unaffected first-class environments are covered by unchanged automated proof
  and unchanged primitive path

Representative minimum in that case:

- every directly affected first-class environment
- plus one contrasting representative path if branch-family behavior differs (`HostScrollback`
  vs `ViewportReplay`, or `StandardScrollRegion` vs `NewlineFallback`)

## Current Automated Entry Points

- `docs/validation/tui-coverage-matrix.md`
- `src/adapter/inbound/tui/app/shell_rendering_tests.rs`
- `src/adapter/inbound/tui/app/shell_rendering_contract_tests.rs`
- `src/adapter/inbound/tui/app/inline_terminal_adapter/tests/`
- `src/adapter/inbound/tui/app/shell_runtime/tests/`
- `src/adapter/inbound/tui/app/snapshots/`
- `tests/native_validation_scripts.rs`

## Validation Commands

```bash
. "$HOME/.cargo/env"
cargo fmt --all -- --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

For TUI visual/presentation work:

```bash
bash scripts/check_tui_layering.sh
```

For broad native/TUI PRs:

```bash
bash scripts/check_native_pr.sh
```

Manual terminal evidence is still required when the change alters escape sequences, viewport mode,
clear behavior, or scrollback behavior. Record manual rows with
`scripts/capture_native_validation.sh` or `scripts/capture_native_validation.ps1`.

## Related Docs

- [README.md](README.md)
- [../plan/10-inline-scrollback-shell.md](../plan/10-inline-scrollback-shell.md)
- [../plan/12-platform-validation-matrix.md](../plan/12-platform-validation-matrix.md)
- [../design/07-tui-layered-architecture-and-aesthetic-contract.md](../design/07-tui-layered-architecture-and-aesthetic-contract.md)
