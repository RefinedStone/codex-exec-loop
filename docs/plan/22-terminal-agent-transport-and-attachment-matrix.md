# Terminal Agent Transport And Attachment Matrix

This document compares the transport and attachment families that could let Akra work with
non-Codex terminal agents.

The goal is not to pick every mechanism.
The goal is to compare them with one rubric before any provider-specific adapter work starts.

## Evaluation Rubric

Every candidate is scored against the same questions:

| Criterion | What it means here |
| --- | --- |
| operator setup | how much manual preparation the operator must do before Akra can attach |
| input relay | how Akra sends prompts, multiline input, and control keystrokes into the agent terminal |
| output relay | how Akra captures stdout, stderr, escape sequences, and prompt boundaries back into the shell |
| interrupt handling | whether stop or cancel can be sent predictably |
| approval handoff | whether approval prompts can be surfaced or safely handed back to the operator |
| reattach and discovery | whether the session can be rediscovered or reattached after Akra restarts |
| recovery | how well the path survives reconnect, restart, or partial terminal drift |
| security | whether credentials and shell scope stay understandable |
| portability | how likely the path is to work across the platforms Akra cares about |
| architecture fit | whether the path maps cleanly to capability seams without inventing a fake shared provider API |

## Candidate Summary

| Candidate | Transport shape | Strength | Main risk | Current posture |
| --- | --- | --- | --- | --- |
| raw local PTY attach | Akra attaches directly to a locally owned PTY | closest to real terminal behavior when Akra owns the process | weak operator-facing discovery and reattach for already-running terminals | research reference, not the first concrete spike |
| tmux session or pane attach | Akra attaches to or observes a user-prepared tmux pane | strongest local attach story for terminal-to-terminal handoff | tmux-specific semantics and fidelity choices must be explicit | primary concrete path |
| managed local wrapper | Akra launches the target CLI inside a managed PTY | highest lifecycle control and easiest prompt injection | can drift from real CLI behavior and grow into a pseudo-provider layer | fallback path |
| SSH or tunnel attach | Akra bridges to a remote terminal endpoint | useful for isolated or remote environments | transport, auth, and recovery complexity grows quickly | deferred |
| proxy or vibeProxy-style mediation | a proxy normalizes terminal I/O between Akra and the agent | stronger replay, normalization, and multiplexing potential | bigger protocol, trust, and security surface | deferred |

## Terminal-To-Terminal Relay Lens

The bridge problem is not just “how to start another CLI.”
It is also “how does one terminal product send and receive data through another terminal surface.”

Each path must answer four relay questions:

1. How does Akra inject prompt text, multiline content, and stop signals?
2. How does Akra observe output without losing prompt boundaries or terminal-state changes?
3. How much of the terminal is mirrored versus delegated back to the operator?
4. What stable anchor exists for recovery: process id, tmux pane id, wrapper handle, SSH target, or
   proxy session id?

## Transport Notes By Candidate

### Raw Local PTY Attach

- Best when Akra itself launched the child process and still owns the PTY handle.
- Poor fit for arbitrary “attach to some other already-open terminal window” workflows because the
  attachment anchor is often missing or platform-specific.
- Useful as a conceptual lower bound for fidelity, but weaker than tmux for operator-driven
  reattachment.

### tmux Session Or Pane Attach

- The most concrete version of “pre-opened local attach” because the pane or session gives Akra a
  stable attach target.
- Input relay can be modeled with tmux-oriented injection or pane stdin control, but the document
  must treat multiline, copy-paste, and control-sequence fidelity as first-class concerns.
- Output relay must distinguish between snapshot-style collection, live piping, and direct control
  semantics; each has different tradeoffs for latency, replay, and recovery.
- Best fit when the operator already accepts tmux as the local control plane.

### Managed Local Wrapper

- Akra owns process launch, environment, PTY allocation, and teardown.
- Prompt submission and interrupt become much easier to standardize.
- The price is realism: wrapper behavior can hide quirks that would still matter in the real
  external CLI.
- Good fallback when attach discovery is too inconsistent or tmux is unavailable.

### SSH Or Tunnel Attach

- Valuable only if local evidence proves the bridge model and a remote use case still justifies the
  added complexity.
- Session identity, terminal resize, reconnect behavior, and credential posture all widen at once.
- Should stay in the matrix so it is deliberately deferred rather than silently forgotten.

### Proxy Or VibeProxy-Style Mediation

- Useful when a proxy could normalize irregular terminal behavior, support replay, or allow
  multiple observers.
- Also the easiest way to create a second system that hides the real provider semantics behind a
  bespoke protocol.
- The proxy layer must justify itself with concrete evidence that tmux or wrapper paths cannot
  reach the needed fidelity or recovery.

## Working Conclusion

- Prefer tmux-oriented local attach as the first concrete transport shape.
- Keep raw local PTY attach as a fidelity reference and fallback concept, not the first operator
  story.
- Keep managed local wrapper ready as the second path when attach semantics prove too brittle.
- Defer SSH or tunnel and proxy or vibeProxy-style mediation until local evidence shows a real gap.
