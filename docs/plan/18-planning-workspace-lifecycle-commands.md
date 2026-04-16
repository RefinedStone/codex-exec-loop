# Planning Workspace Lifecycle Commands

This document defines the target command surface for planning workspace lifecycle management outside and inside the interactive shell.

## Goal

The product should let an operator bootstrap, inspect, and safely reset planning state without having to enter the TUI first.

The same lifecycle concepts should also be available inside the shell so the operator does not need two different mental models.

## Command Set

### External Commands

- `akra doctor`
- `akra init`
- `akra reset`

### In-Shell Counterparts

- `:doctor`
- `:init`
- `:reset`

## Shared Design Rules

- External and in-shell commands should use the same planning validation and bootstrap rules.
- The external commands are non-interactive by default and should be safe to call from a normal shell prompt before launching `akra`.
- In-shell commands may use richer confirmation, staged review, or overlay affordances when that improves safety.
- Reset behavior must be explicit about what will be rewritten and what will remain intact.

## `akra doctor`

## Operator Goal

Let an operator validate planning health from the workspace root before launching the TUI.

## Command Contract

- default target is the current working directory
- command is read-only
- command should inspect whether planning is absent, incomplete, invalid, ready without task, or ready with task
- command should surface queue-idle policy, top queue summary, proposal summary, and the first validation failure when relevant
- command should exit successfully only when the workspace is either valid or intentionally absent

## Recommended Output Shape

- workspace path
- planning state
- queue summary
- proposal summary when present
- first blocking validation issue or confirmation that the workspace is healthy

## Exit Semantics

- exit `0` when the workspace is healthy or when planning is not initialized
- exit `1` when planning files are invalid, incomplete, or otherwise blocked

## In-Shell `:doctor`

- opens a planning health view or reuses an existing planning diagnostics surface
- uses the same validation engine as external `akra doctor`
- does not silently mutate planning files

## `akra init`

## Operator Goal

Create the default planning scaffold from a normal shell prompt without first entering the TUI.

## Command Contract

- default mode is the current simple scaffold
- command writes the active planning files directly into `.codex-exec-loop/planning/`
- command fails if active planning files already exist
- future mode selection can be added, but the first implementation should optimize for the simple-path bootstrap

## Files Created

- `directions.toml`
- `task-ledger.json`
- `task-ledger.schema.json`
- `result-output.md`
- default queue-idle prompt when the chosen scaffold requires it
- derived queue snapshot if the implementation writes one at bootstrap time

## In-Shell `:init`

- acts as the fast entrypoint into the existing planning bootstrap flow
- should preselect the bootstrap path rather than dropping the operator into an unrelated generic overlay
- should preserve staged review and promotion semantics instead of silently bypassing them

## `akra reset`

## Operator Goal

Safely reset queue state, directions state, or the whole planning workspace back to a known default.

## Reset Targets

- `queue`
  resets `task-ledger.json` and `queue.snapshot.json` while keeping directions and supporting docs intact
- `directions`
  resets `directions.toml`, direction detail docs, and queue-idle prompt artifacts back to the default scaffold
- `all`
  resets both directions-side artifacts and queue-side artifacts back to the default planning scaffold

## Safety Rules

- `reset queue` is safe to run independently
- `reset directions` must refuse to proceed when the existing task ledger still contains live non-done tasks unless the operator also chooses a full reset
- `reset all` rewrites the full active planning scaffold and is the destructive reset path
- every reset path should state exactly which artifacts will be rewritten before it runs

## In-Shell `:reset`

- supports `queue`, `directions`, and `all`
- requires explicit confirmation before destructive resets
- should stage or preview the reset when that provides more safety than immediate mutation

## Boundary And Ownership

- bootstrap logic should stay behind the planning bootstrap service or an equivalent application-layer boundary
- validation logic should stay behind the planning validation and prompt services
- reset behavior should not bypass reconciliation or accepted-state expectations in a way that creates a second planning model
- external CLI commands and in-shell commands should call shared application services rather than duplicating rules

## Acceptance Criteria

- an operator can run `akra doctor` before opening the TUI and immediately understand whether planning is healthy
- an operator can run `akra init` in a fresh workspace and get the same default scaffold the product expects
- an operator can run `akra reset queue`, `akra reset directions`, or `akra reset all` with predictable safety semantics
- the in-shell `:doctor`, `:init`, and `:reset` flows feel like richer versions of the same lifecycle operations, not unrelated features
