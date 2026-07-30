# PR 2061 Typed Terminal Delivery Evidence

This supplemental artifact set records primitive-sensitive validation for implementation commit
`ac409ed7d1fa97cde46553aff271e5ae3b162742`. It exercises the shipped
`HostScrollback` + automatic `NewlineFallback` parallel transaction against an actual tmux 3.4
detached PTY and production `akra` runtime composition.
The later PR-head delta contains this documentation and no production-source change.

The artifact is `supplemental-unmatched`. It is a candidate-specific first-class E3 observation,
but it does not change dated validation-summary totals or impersonate candidate-specific E1, E2, or
E4 captures.

## Result

- Two distinct pool-state events were admitted through the real parallel refresh path.
- Each event appeared exactly once at 120×30, 48×18, restore, board close, board reopen, and a
  second shrink/restore cycle.
- A third shrink/restore retained both events once.
- Focused cursor placement tracked the physical live viewport origin: `(0,14)` at 120×30 and
  `(0,2)` at 48×18.
- Host-history-only inspection retained each full event once and found zero `Parallel Operations`,
  `Agent Lanes`, `Selected Lane`, `Accepted Queue`, or `Command Hints` rows.
- `Ctrl+q` closed the candidate through its normal shutdown path.

The tested boundary combines two protections: a newly durable host batch writes physical reflow
guards in the same typed transaction, and a focused frame anchors its hidden cursor at the live
viewport origin. Keeping only the cursor at the terminal bottom preserved event counts but allowed
four board rows into tmux history; keeping it at the terminal origin allowed broader chrome reflow.
Those exploratory shapes are failed controls, not accepted candidate evidence.

## Evidence Matrix

| Row | Candidate-specific manual result | Supporting evidence | Release-evidence treatment |
| --- | --- | --- | --- |
| E1 Windows Terminal + WSL bash | not executed | PR 1926 independently passed the unchanged `NewlineFallback` primitive, resize, reset, and session-switch flow on an older candidate | historical support only; not relabeled |
| E2 Windows Terminal + PowerShell | not executed | PR 1926 independently passed the unchanged `NewlineFallback` primitive, resize, reset, and session-switch flow on an older candidate | historical support only; not relabeled |
| E3 tmux detached PTY | pass | this artifact, exact implementation commit and binary | candidate-specific first-class observation |
| E4 direct Linux terminal | not executed | automated vt100 proof covers the common fallback transaction; PR 1926 direct-terminal evidence used the now-diagnostic standard path | explicit candidate-specific evidence gap |

This is the explicit downgrade required by the terminal methodology: only E3 has
candidate-specific real-terminal evidence in this workspace. E1, E2, and E4 remain `not executed`
for this candidate, not implicit passes. The downgrade limits the evidence claim; it does not hide
a known automatic-path failure.

## Files

- [environment-stamp.txt](environment-stamp.txt) records candidate and environment provenance.
- [scenario-results.txt](scenario-results.txt) records the manual sequence and named automated
  addenda.
- [candidate-integrity.txt](candidate-integrity.txt) records source and binary identity.

Historical comparison: [PR 1926 physical-resize evidence](../pr-1926-physical-resize/README.md).

## Scope And Safety

- The proof repository, `AKRA_HOME`, pool worktrees, and empty proof commits were isolated under a
  disposable temporary root.
- No accepted planning task was created or delivered.
- Raw pane output is omitted because the normalized assertions are sufficient and local paths are
  not useful review evidence.
- No credentials, tokens, session IDs, private prompts, or provider output are included.
- No claim follows for Admin, CLI, Telegram, persistence, or released-package behavior.
