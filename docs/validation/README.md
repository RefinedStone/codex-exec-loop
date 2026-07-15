# Validation Records

[한국어 안내](../ko/reference/validation.md)

This directory stores real validation records. The required rows and profiles are in
[Platform Validation Matrix](../plan/12-platform-validation-matrix.md); primitive-sensitive proof
rules are in [Terminal UI Testing Methodology](terminal-ui-testing-methodology.md).

## Evidence Rules

- Record only an observed pass or blocker and include the exact commit.
- Use one file per exercised terminal/frontend row.
- Use `capture_role: counted-row` only for an exact required matrix row.
- Use `capture_role: supplemental-unmatched` for representative branch-family evidence; it does not
  satisfy a missing required row.
- Preserve the emitted profile, checklist, environment fields, result, and notes.
- Do not copy generated coverage totals into this file. Run the summary helper against the records.

Historical parallel-mode production runs from 2026-04-30 proved happy-path delivery, queued-result
delivery, restart recovery, blocked-conflict preservation, and serialized multi-worktree delivery.
Their identifiers remain available in Git history; they are not current terminal matrix rows.

## Capture

```bash
bash scripts/capture_native_validation.sh \
  --frontend inline \
  --check-profile terminal-baseline \
  --terminal "iTerm2 3.5" \
  --result pass \
  --output-dir docs/validation
```

Use `phase1-operator-surface`, `prompt-input-delay-pty`, or an approved supplemental profile when
that is the actual scenario. Windows uses `scripts/capture_native_validation.ps1`.

Primitive-sensitive supplemental evidence may append environment class, effective render/insertion
mode, override causes, and per-scenario results after the helper output. Follow the methodology; the
capture helper alone does not create missing manual evidence.

## Summarize

```bash
bash scripts/summarize_native_validation.sh
bash scripts/summarize_native_validation.sh --fail-on-incomplete
bash scripts/summarize_native_validation.sh --format markdown
bash scripts/summarize_native_validation.sh \
  --check-profile prompt-input-delay-pty --fail-on-incomplete
```

The plain summary reports visibility and warns on gaps. Only `--fail-on-incomplete` acts as an
explicit gate.

## Historical and Review Artifacts

- [artifacts/pr-1926-physical-resize/](artifacts/pr-1926-physical-resize/) contains reviewed
  primitive-sensitive evidence for that exact candidate, not a general release pass.
- [../../artifacts/terminal-bridge-readiness-2026-04-23/](../../artifacts/terminal-bridge-readiness-2026-04-23/)
  is historical feasibility evidence and cannot satisfy a current required row.
