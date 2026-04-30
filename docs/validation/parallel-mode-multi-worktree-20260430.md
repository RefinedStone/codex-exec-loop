# Parallel Mode Multi-Worktree Validation

Date: 2026-04-30

This document records a live production validation of operator-driven multi-worktree delivery.

Observed run:

- Baseline: `04a5e3c5472f5c89542a16c1500a6c082ef169dd`
- Active pool worktrees: `slot-1` and `slot-2` leased concurrently; `slot-3` remained idle.
- Source branch A: `akra-agent/slot-1/production-multi-worktree-a`
- Source commit A: `e5006049323f8e046538472cbaff4d93d91579ea`
- Pull request A: `#560`
- Integration commit A: `8e20fa4a8ca22d62ae419d808e7df821d22f0020`
- Source branch B: `akra-agent/slot-2/production-multi-worktree-b`
- Source commit B: `24482aee13ed765c45e336a4920da19f67fb23f8`
- Pull request B: `#561`
- Integration commit B: `bff732bd0c5322f47fd88c370c7c348cf6116e5e`
- Result: the first `akra parallel-tick` processed only queue head A and returned `slot-1` to idle while B stayed queued in `slot-2`; the second tick processed B and returned `slot-2` to idle; a final idle tick reconciled all three pool worktrees to the current `prerelease` head.

The unrelated feature worktree `feature-native-planning-prompt-assembly` stayed present throughout the run and was not modified.
