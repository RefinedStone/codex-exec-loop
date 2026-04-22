# Supersession Docs

This folder is the canonical entry for the current supersession, planning, and directions contract.

Read in this order:

1. [current-contract.md](current-contract.md): shipped operator-facing contract
2. [remaining-work.md](remaining-work.md): unfinished implementation, validation, polish, open questions, and non-goals

## Supporting Deep Dives

- [../design/01-current-product-state.md](../design/01-current-product-state.md): product identity and surface map
- [../design/02-tui-shell-flow.md](../design/02-tui-shell-flow.md): shell interaction flow
- [../design/06-planning-runtime-and-draft-editor.md](../design/06-planning-runtime-and-draft-editor.md): planning/runtime implementation detail
- [../releases/v1.2.9-to-prerelease.md](../releases/v1.2.9-to-prerelease.md): release delta from `v1.2.9`

## Historical References

- [../plan/18-repo-shared-planning-authority-store.md](../plan/18-repo-shared-planning-authority-store.md): authority-store redesign record that guided the implemented cutover
- [../plan/19-supersession-runtime-risk-audit.md](../plan/19-supersession-runtime-risk-audit.md): pre-store failure analysis and risk audit

## Working Rules

- Keep implemented behavior compact in [current-contract.md](current-contract.md).
- Keep only unfinished or lightly validated work detailed in [remaining-work.md](remaining-work.md).
- Do not repeat the same current contract across `README`, `docs/README`, `docs/design`, and `docs/plan`.
- Use `docs/design/` for technical depth, not for a competing current-truth summary.
