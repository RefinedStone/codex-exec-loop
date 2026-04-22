# Terminal Agent Bridge Experiment Matrix

This document turns the bridge research into concrete experiments.

The purpose is evidence, not premature implementation.
Local attach and managed wrapper get executable experiments first.
SSH or tunnel and proxy or vibeProxy-style mediation stay in deferred-feasibility mode.

## Experiment Rules

- run local experiments before any remote or proxy work
- record what Akra can really observe and control, not what the transport theoretically allows
- treat terminal-to-terminal data relay as a first-class concern
- capture both success and failure signatures so deferred paths have explicit reasons

## Primary Experiments

| Experiment | Candidate | Goal | Evidence to collect | Pass signal |
| --- | --- | --- | --- | --- |
| local attach via tmux | pre-opened local attach | prove Akra can attach to a stable pane or session and keep one terminal cockpit story | attach target discovery, prompt injection path, live output capture, stop behavior, restart behavior | pane-oriented attach works with explainable operator setup and no fake session model |
| managed wrapper PTY | local wrapper | prove Akra can launch and control a target CLI when attach discovery is unavailable | launch flow, stream capture fidelity, interrupt support, prompt boundary handling, teardown and recovery | wrapper path is more controlled but clearly documented as less faithful than tmux attach |

## Deferred Feasibility Questions

| Candidate | Question that must be answered before a spike | Required evidence |
| --- | --- | --- |
| SSH or tunnel attach | does the remote use case justify wider auth and recovery complexity after local attach is already credible? | concrete remote operator story plus recovery notes that local attach cannot satisfy |
| proxy or vibeProxy-style mediation | what specific fidelity, replay, or observer requirement cannot be met by tmux or wrapper paths? | one explicit gap, a smaller failed local attempt, and a security posture note |

## Scenario Checklist

Every primary experiment should walk the same scenarios:

1. Detect prerequisites before entry.
2. Attach or launch successfully.
3. Send a one-line prompt.
4. Send multiline input without mangling it.
5. Observe streaming output with enough fidelity to mirror the operator story.
6. Request interrupt or stop and record the true provider behavior.
7. Handle an approval or confirmation prompt without hiding responsibility.
8. Restart Akra and test whether reattach is possible, partial, or unsupported.

## tmux-Focused Checks

- confirm how Akra addresses the pane or session
- confirm how prompt text is injected and whether multiline payloads stay faithful
- confirm whether output is captured through snapshot, pipe, or direct control semantics
- confirm what stable recovery anchor Akra can store after restart
- record where tmux semantics leak into operator setup so the path stays explicit rather than
  magical

## Managed Wrapper Checks

- confirm the exact launch contract and environment preparation
- confirm whether the wrapper can preserve the same approval and interrupt semantics as the real CLI
- record where wrapper control improves recovery and where it distorts real behavior
- keep a clear note on what is wrapper-only convenience and not portable provider truth

## Expected Outcome

- tmux-oriented local attach either stays the primary path with evidence or loses that status for a
  written reason
- managed wrapper either stays the fallback path with known realism costs or is rejected with
  evidence
- SSH or tunnel and proxy or vibeProxy-style mediation remain deferred unless the local experiments
  reveal a concrete unsolved gap
