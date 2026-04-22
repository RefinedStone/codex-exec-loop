# tmux Local Attach Readiness Evidence

This document closes the evidence gap named by `terminal-bridge-local-spike-readiness-gate`.

It records one local tmux pass on April 23, 2026 without adding any Akra provider adapter yet.
The point is to decide whether a bounded tmux implementation slice is justified, not to pretend the
bridge is already shipped.

## Environment

- host check: `tmux 3.4`
- test socket: `akra-readiness-20260423`
- detached session: `akra-evidence`
- pane shell: `bash --noprofile --norc`
- captured artifacts: `artifacts/terminal-bridge-readiness-2026-04-23/`

## Evidence Summary

| Question | Result | Evidence | Implementation note |
| --- | --- | --- | --- |
| Can Akra discover an attach target without a provider session API? | yes | [01-pane-discovery.txt](../../artifacts/terminal-bridge-readiness-2026-04-23/01-pane-discovery.txt) shows session, window, pane id, tty, pid, and current command for a detached tmux pane | tmux can support attach-only or handle-based reattach without inventing a provider-backed session catalog |
| Can Akra inject one-line and multiline prompt payloads faithfully? | yes, with caveat | [03-sendkeys-multiline.txt](../../artifacts/terminal-bridge-readiness-2026-04-23/03-sendkeys-multiline.txt) and [04-paste-multiline.txt](../../artifacts/terminal-bridge-readiness-2026-04-23/04-paste-multiline.txt) both replayed multiline here-doc payloads correctly | prefer `load-buffer` plus `paste-buffer` for multiline or large payloads; keep `send-keys -l` for short literal input |
| Can Akra relay output incrementally enough to keep a live shell story? | yes, with transcript semantics | [06-stream-at-0.5s.log](../../artifacts/terminal-bridge-readiness-2026-04-23/06-stream-at-0.5s.log) already contains `tick-0` and `tick-1`, while [07-stream-at-1.8s.log](../../artifacts/terminal-bridge-readiness-2026-04-23/07-stream-at-1.8s.log) contains the full run | `pipe-pane` is credible for live relay, but Akra must sanitize terminal control bytes and accept terminal transcript semantics instead of provider event frames |
| Is interrupt truth strong enough to expose as a capability? | yes | [09-interrupt.txt](../../artifacts/terminal-bridge-readiness-2026-04-23/09-interrupt.txt) shows `^C` and shell status `130` after `sleep 30` | treat interrupt as runtime-native terminal control, not provider-native acknowledgement; it only stays true when the pane process respects tty SIGINT |
| Can approval be handled without lying about the control surface? | only as manual handoff | [10-approval-prompt.txt](../../artifacts/terminal-bridge-readiness-2026-04-23/10-approval-prompt.txt) and [11-approval-answer.txt](../../artifacts/terminal-bridge-readiness-2026-04-23/11-approval-answer.txt) show raw prompt detection plus raw `y` input | approval must stay `ManualHandoff`; tmux gives Akra keystroke injection, not structured approve or deny semantics |
| Is there a stable recovery anchor for Akra restart? | yes | [12-recovery-anchor.txt](../../artifacts/terminal-bridge-readiness-2026-04-23/12-recovery-anchor.txt) records pane id `%2`, and [13-recovery-by-pane-id.txt](../../artifacts/terminal-bridge-readiness-2026-04-23/13-recovery-by-pane-id.txt) re-captures the transcript through that stored handle | `TerminalBridgeAttachment` can credibly map tmux local attach to `LocalAttach + SessionHandle` |
| Are setup and failure recovery explicit enough for an operator runbook? | yes | [14-missing-target.txt](../../artifacts/terminal-bridge-readiness-2026-04-23/14-missing-target.txt), [15-missing-capture.txt](../../artifacts/terminal-bridge-readiness-2026-04-23/15-missing-capture.txt), and [16-no-server.txt](../../artifacts/terminal-bridge-readiness-2026-04-23/16-no-server.txt) capture the primary failure signatures | startup and attach copy can state missing target, missing server, and rediscovery steps truthfully before a turn starts |

## What The Artifacts Proved

### Attach Target Discovery

- tmux already exposes the minimum handle Akra needs: pane id, tty, pid, and human-readable
  session/window naming.
- This is enough for a handle-based reattach tier even if the bridge never exposes a broad
  recent-session browser.
- The local attach path therefore does not need a fake provider session model to start.

### Prompt Injection Path

- `send-keys -l` successfully replayed a multiline here-doc into the pane.
- `load-buffer` plus `paste-buffer` also replayed the same style of payload, including spaces and
  embedded JSON punctuation.
- The safer default for a real adapter is `load-buffer` plus `paste-buffer` plus `send-keys C-m`
  because it keeps one explicit payload buffer and avoids relying on per-character shell-editing
  behavior for larger prompts.

### Incremental Output Relay Fidelity

- `pipe-pane` produced a live log before the command finished.
- The early snapshot already contained partial output, which is the key requirement for a live
  shell transcript.
- The relay is not semantically clean text. The log includes bracketed-paste control bytes, so a
  real runtime must normalize or filter terminal control sequences before they surface in Akra.
- `capture-pane` remains useful for transcript recovery and snapshot refresh, but it is polling,
  not live streaming.

### Interrupt Truth

- Sending `C-c` into the pane interrupted a long-running command and returned shell status `130`.
- That is strong enough to mark interrupt as supported for the tmux path.
- The truth is still terminal-mediated. Akra would be sending a tty control sequence, not asking a
  provider API for a durable cancel acknowledgement.

### Approval Handling Truth

- The approval prompt is visible in the pane transcript.
- Akra can answer it only by sending more raw input into the same pane.
- Nothing in this transport creates a structured approval review object on its own.
- The tmux path should therefore report approval as manual handoff from the start.

### Recovery Anchor And Restart Path

- The pane id stayed stable long enough to re-capture the pane from a separate tmux command.
- That is the relevant restart case for Akra: the Akra process can die and come back while the tmux
  server and pane keep running.
- This evidence does not claim survival through tmux server loss. If the tmux server is gone, the
  failure should stay explicit instead of pretending recovery still exists.

### Operator Setup Cost And Failure Recovery

Minimum operator story:

1. Ensure `tmux` exists and a server can be started.
2. Create or pick a target session or pane.
3. Discover the target with `list-panes` or pass an explicit handle.
4. Enable `pipe-pane` if live relay is desired.
5. Submit prompt text through a literal paste path.

Primary failure signatures:

- wrong pane or window handle: `can't find window`
- missing tmux server or dead socket: `server exited unexpectedly`
- dead pane after the target process exits: rediscovery or operator recreation is required

The setup cost is real but explainable. It is closer to an operator runbook than a hidden transport
layer, which is acceptable for this research-selected primary path.

## Scenario Coverage Against The Experiment Matrix

| Scenario from `24` | Result | Note |
| --- | --- | --- |
| detect prerequisites before entry | pass | tmux presence and server or target existence are directly checkable |
| attach or launch successfully | pass | detached session and panes were created and addressed explicitly |
| send a one-line prompt | pass | same transport used for multiline also covers one-line injection |
| send multiline input without mangling it | pass | both send-keys and buffer paste preserved the payload |
| observe streaming output | pass with caveat | `pipe-pane` is live enough, but transcript cleanup is required |
| request interrupt or stop | pass | `C-c` worked through pane tty control |
| handle approval or confirmation prompt | pass with caveat | possible only as raw-input handoff |
| restart Akra and reattach | pass for handle-based reattach | pane id re-capture worked while tmux server stayed alive |

## Remaining Gaps Before Or During Implementation

- completion detection is still transcript-derived, not provider-native
- terminal control-sequence filtering must be part of the real runtime slice
- no evidence was collected yet for fullscreen TUIs, resize propagation, or remote tmux control
- this pass did not try to prove a provider-wide session browser; it only proved local-handle
  discovery and reattach

## Outcome

The tmux local attach path now has reviewable evidence for discovery, prompt submit, incremental
relay, interrupt truth, approval truth, recovery anchor, and operator setup cost.

That is enough to close the readiness gate and move the next queue item into a bounded tmux-only
implementation slice.
