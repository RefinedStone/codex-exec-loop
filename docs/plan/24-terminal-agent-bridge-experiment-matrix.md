# Terminal Agent Bridge Experiment Matrix

This document turns the bridge research into concrete future experiments.

The purpose is evidence, not premature implementation.
The next executable experiment family is a Claude-first headless runner behind `PlanningWorkerPort`
while the main interactive runtime stays on `codex app-server`.

## Experiment Rules

- keep the main interactive runtime unchanged while exploring the first non-Codex worker path
- record what Akra can really observe and control, not what the runtime theoretically allows
- treat changed-file reporting, stream completion, and failure truth as first-class concerns
- capture both success and failure signatures so deferred paths have explicit reasons

## Primary Experiments

| Experiment | Candidate | Goal | Evidence to collect | Pass signal |
| --- | --- | --- | --- | --- |
| Claude headless planning worker | managed local launch | prove Akra can satisfy `PlanningWorkerPort` with a Claude-first headless CLI runner without changing the main interactive runtime | launch contract, stream capture, completion or failure detection, changed planning file reporting, environment requirements | hidden planning runs through the new worker while the main conversation path remains on `codex app-server` |
| headless attachment truth | managed local launch | decide how the runner should surface attachment truth without pretending it is a full interactive attach story | `TerminalBridgeAttachmentProfile` mode choice, recovery-anchor choice, operator notices, future extensibility notes | the runner advertises explicit truth such as `ManagedWrapper` or a new managed-headless constructor instead of leaking provider assumptions |

## Deferred Feasibility Questions

| Candidate | Question that must be answered before a spike | Required evidence |
| --- | --- | --- |
| main interactive Claude runtime | what operator or product requirement justifies replacing the shipped Codex main session? | one explicit operator story plus transition and recovery notes |
| SSH or tunnel attach | does the remote use case justify wider auth and recovery complexity after a local headless worker is already credible? | concrete remote operator story plus recovery notes that local launch cannot satisfy |
| proxy or vibeProxy-style mediation | what specific fidelity, replay, or observer requirement cannot be met by local launch or other simpler paths? | one explicit gap, a smaller failed local attempt, and a security posture note |

## Scenario Checklist

Every primary experiment should walk the same scenarios:

1. Detect prerequisites before entry.
2. Launch the worker successfully.
3. Send a one-line prompt.
4. Send multiline input without mangling it.
5. Observe streaming output with enough fidelity to decide completion or failure.
6. Capture changed planning files truthfully.
7. Record what interrupt or cancellation can and cannot do in the worker path.
8. Restart Akra and record whether worker recovery is supported, partial, or intentionally absent.

## Headless Runner Checks

- confirm the exact launch contract and environment preparation
- confirm whether the worker runs as a hidden planning-only surface rather than a new main session
- confirm how completion and failure are detected without relying on a provider session catalog
- confirm how changed planning files are surfaced back through `PlanningWorkerPort`
- record where attachment vocabulary needs a managed headless truth instead of an interactive
  attach claim

## Expected Outcome

- a Claude-first headless runner either becomes the next bounded planning-worker slice or is
  rejected for a written reason
- the main interactive runtime stays on `codex app-server` unless a later decision explicitly
  changes that baseline
- SSH or tunnel and proxy or vibeProxy-style mediation remain deferred unless local headless work
  reveals a concrete unsolved gap
