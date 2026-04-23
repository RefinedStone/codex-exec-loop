# Terminal Agent Transport And Attachment Matrix

This document compares the transport and attachment families that could matter if Akra expands
beyond `codex app-server`.

The goal is not to activate every mechanism.
The goal is to keep one comparison rubric ready for future bounded work.

## Evaluation Rubric

Every candidate is scored against the same questions:

| Criterion | What it means here |
| --- | --- |
| operator setup | how much preparation is needed before Akra can start or attach |
| input relay | how Akra sends prompts, multiline input, and control signals |
| output relay | how Akra captures stdout, stderr, terminal state, and prompt boundaries |
| interrupt handling | whether stop or cancel can be sent predictably |
| approval handoff | whether approval prompts can be surfaced or handed back truthfully |
| reattach and discovery | whether the runtime can be rediscovered after Akra restarts |
| recovery | how well the path survives reconnect, restart, or partial terminal drift |
| security | whether credentials and shell scope stay understandable |
| portability | how likely the path is to work across the platforms Akra cares about |
| architecture fit | whether the path maps cleanly to capability seams without inventing a fake shared provider API |

## Candidate Summary

| Candidate | Transport shape | Strength | Main risk | Current posture |
| --- | --- | --- | --- | --- |
| raw local PTY attach | Akra attaches directly to a locally owned PTY | closest to raw terminal behavior when Akra owns the process | weak discovery and reattach for already-running terminals | reference only |
| tmux session or pane attach | Akra attaches to or observes a user-prepared tmux pane | stable local anchor when the operator already lives in tmux | tmux-specific semantics and operator setup can dominate the design | research reference, not the active product path |
| managed local launch | Akra launches the target CLI in a controlled local process or PTY | strongest fit for a bounded headless worker | can drift from real interactive CLI behavior if the wrapper becomes too magical | most plausible next non-main-session path |
| SSH or tunnel attach | Akra bridges to a remote terminal endpoint | useful for isolated or remote environments | transport, auth, and recovery complexity grows quickly | deferred |
| proxy or vibeProxy-style mediation | a proxy normalizes terminal I/O between Akra and the agent | stronger replay, normalization, and multiplexing potential | bigger protocol, trust, and security surface | deferred |

## Relay Lens

The bridge problem is not just “how to start another CLI.”
It is also “how does one terminal product send and receive data through another terminal or
headless surface.”

Each path must answer four relay questions:

1. How does Akra inject prompt text, multiline content, and stop signals?
2. How does Akra observe output without losing prompt boundaries or terminal-state changes?
3. How much of the runtime is mirrored versus kept hidden as a headless worker?
4. What stable anchor exists for recovery: process id, wrapper handle, pane id, SSH target, or
   proxy session id?

## Transport Notes By Candidate

### Raw Local PTY Attach

- Best when Akra itself launched the child process and still owns the PTY handle.
- Poor fit for arbitrary “attach to some other already-open terminal” workflows because the anchor
  is often missing or platform-specific.
- Useful as a fidelity reference, not as the current queue head.

### tmux Session Or Pane Attach

- Gives Akra a concrete operator-managed attach target when the operator already uses tmux.
- Keeps local-attach semantics explicit, but it also pulls tmux-specific addressing and transcript
  behavior into the design.
- Not the current product path after the runtime reset back to `codex app-server` only.

### Managed Local Launch

- Akra owns process launch, environment, stream capture, and teardown.
- Fits hidden planning or sub-task workers better than replacing the main interactive runtime.
- The price is realism: launch control can hide quirks that still matter in the real CLI.

### SSH Or Tunnel Attach

- Valuable only if a future use case justifies remote auth, reconnect, and portability costs.
- Should stay in the matrix so it is deliberately deferred rather than silently forgotten.

### Proxy Or VibeProxy-Style Mediation

- Useful when a proxy could normalize irregular behavior, support replay, or allow multiple
  observers.
- Also the easiest way to create a second system that hides the real provider semantics behind a
  bespoke protocol.

## Working Conclusion

- Keep the shipped main interactive runtime on `codex app-server`.
- Use this matrix as a comparison baseline for future bounded non-main-session work.
- Managed local launch is the strongest current candidate for a Claude-first headless planning or
  sub-task runner.
- Raw PTY attach, tmux attach, SSH or tunnel, and proxy mediation remain non-active transport
  families until a future slice proves a concrete need.
