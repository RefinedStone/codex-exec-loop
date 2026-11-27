# Local validation integration evidence — 2026-11-27

Related issue: [#2906](https://github.com/RefinedStone/codex-exec-loop/issues/2906).
Implementation base: `d619440675581b8ea3f9b08dd948e2899ef6309e` (#2905 runner, including #2904).

## Verified in this change

- 34 shared planner policy tests passed.
- 19 collector tests passed with real temporary Git repositories and controlled metadata fixtures.
  They cover staged/unstaged/untracked paths, rename/delete, same-size restored-mtime edits, index
  identity, registered ignored inputs and directory prefixes, config/environment hashing, exact
  source commits, unavailable base fallback, missing head failure, concurrent edits, immutable
  archive cleanup, symlink/submodule rejection, and CI clean-checkout enforcement. Actual registry
  overlap with the admin npm install directory is covered: new install output preserves identity,
  while source/contract overlaps, arbitrary exclusions and tracked output edits remain protected.
- 13 local runner tests passed. They cover complete package argv for Windows/Linux/macOS
  configurations, registered contracts, existing full/Node/docs gates, unsupported conditions,
  source/config/registry/graph stale-plan rejection before command zero, execution-root selection,
  target/incremental preservation, failure/cancellation propagation and source changes during
  validation. The implementation reuses the existing process-tree supervisor; no new descendant
  management policy is introduced here.
- A real Node subprocess ran the existing performance wrapper around a Git diff check and wrote
  the existing JSONL measurement format. Saved-plan JSON and explain CLIs ran without Cargo.
- Both Bash Changed entrypoints ran against an argv recorder. Spaces and shell metacharacters
  remained literal arguments; target/incremental values were preserved. Extra full-gate arguments
  were rejected. `bash -n` accepted the changed Bash files.

Commands:

```text
node --test scripts/agent-plan.test.mjs
node --test scripts/ci-plan-inputs.test.mjs scripts/ci-impact-plan.test.mjs
bash -n scripts/check_native_pr.sh scripts/check_windows_portable.sh scripts/check_node_surfaces.sh
```

## Environment and limits

The Windows tunnel session terminated during implementation. The final integration was prepared
and tested in a Linux Node 24.22.3 / Git 2.56.1 environment. Cargo and PowerShell were unavailable
there. Cargo metadata was represented by explicit fixtures; no real Rust suite or Windows
PowerShell execution is claimed by these results. PowerShell wiring was reviewed statically.

The full repository local gate was not run in this environment. The parent integration retains
that limitation and requires CI Gate for the final composed changes. No new CI job or model turn
was started for these tests. This evidence establishes selection/collection/execution policy;
it does not claim a new Rust build-time improvement or a complete CI success.
