# tmux Local Attach Gate Verdict

This document records the verdict for `terminal-bridge-local-spike-readiness-gate` after the local
evidence pass in [27-terminal-agent-tmux-local-attach-readiness-evidence.md](27-terminal-agent-tmux-local-attach-readiness-evidence.md).

## Verdict

Pass the gate.

tmux local attach stays the primary path, and the first real bridge slice can now be queued as a
tmux-only implementation.

## Why The Gate Passes

- relay fidelity is credible enough through `pipe-pane`, with explicit cleanup caveats
- multiline prompt injection is credible through `load-buffer` plus `paste-buffer`
- interrupt support is real enough to expose as supported
- approval handling truth is clear: manual handoff only
- pane id or session handle recovery is reviewable and operator-explainable
- failure signatures are explicit enough for `StartupProbe` and operator copy

The gate was never about making tmux look perfect.
It was about proving that relay, recovery, and operator cost could be written down without hand
waving.
That bar is now met.

## Path Decision After Evidence

| Candidate | Verdict | Reason |
| --- | --- | --- |
| tmux local attach | keep as primary | the evidence now covers the required local relay and recovery questions without inventing a fake session API |
| managed wrapper PTY | keep as fallback | still useful when tmux is unavailable or operator-managed attach targets are too awkward |
| SSH or tunnel attach | keep deferred | the local path is now credible, so remote auth and recovery complexity still lacks justification |
| proxy or vibeProxy-style mediation | keep deferred | no concrete fidelity gap was found that forces an extra mediation layer yet |

## Constraints For The First Real Slice

- do not introduce a provider-wide universal bridge API
- implement one tmux local-attach path only
- keep `SessionCatalog` optional; explicit pane handles must remain valid even without a catalog UI
- emit `ConversationStreamEvent::AttachmentObserved` with a tmux-shaped local-attach profile
- keep approval and interrupt truth explicit instead of inheriting Codex defaults

## Capability Mapping For The Next Slice

### `StartupProbe`

- verify `tmux` exists
- verify the target session or pane handle resolves, or truthfully explain why it does not
- surface missing-server and missing-target failures before the operator enters the shell flow

### `InteractiveTurnRuntime`

- submit prompt text through a literal paste path
- observe output through `pipe-pane`, plus transcript recovery through `capture-pane`
- expose interrupt as `RuntimeNative`
- expose approval as `ManualHandoff`
- detect completion through transcript or terminal-state heuristics, not fake provider events

### `SessionCatalog`

- keep it optional
- if present, treat tmux discovery as handle-based reattach rather than as a provider session model
- if absent, allow explicit attach-only flows to proceed

### `TerminalBridgeAttachment`

- model the tmux path as `LocalAttach`
- model the recovery anchor as `SessionHandle`
- keep the stored anchor explicit enough for reattach and failure copy

## Out Of Scope For The Next Slice

- managed wrapper implementation
- SSH or tunnel implementation
- proxy or vibeProxy-style mediation
- fullscreen terminal UI fidelity work
- any broad rename that only swaps `Codex` for generic nouns

## Queue Consequence

`terminal-bridge-local-spike-readiness-gate` should now be treated as done.

`terminal-bridge-primary-implementation-slice` should move from blocked to ready, with scope
limited to one tmux local-attach adapter path that consumes the already-landed capability seams.
