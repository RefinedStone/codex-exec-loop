# Repository rename acceptance preflight (#2591)

This is preparation only. No repository rename, ruleset change, branch mutation, release publication, local remote rewrite or CI cancellation is performed by the audit.

## Observed state

Read-only capture against RefinedStone/codex-exec-loop, repository ID 1201646585, on 2026-11-28 KST:
- Default branch prerelease; public visibility.
- 10 branches, 2 open pull requests, 12 releases, 1 ruleset.
- 4 queued and 6 in-progress workflow runs; not a quiescent window.
- Sampled issue identities #2414, #2555, #2584 and #2591 are readable.
- The proposed new URL returned HTTP 404 in the preceding read-only probe. This alone is not proof that the new name is available.

Counts are observations, not invariant values to reuse at cutover. #2588 and #2589 must be accepted first.

## Cutover checklist

1. Review the active references: root package.json, npm/package.json, npm/scripts/stage-npm-packages.mjs, docs/reference/naming.md and other current documentation links. Search workflow uses, links, release URLs and manifests for the old slug. Keep historical references identifiable; do not replace live download endpoints before the new name exists.
2. Inventory local worktrees/remotes, automation, webhooks, GitHub App integrations, Pages (if configured), external repositories and npm consumers. Confirm owner access and name availability. Do not assume redirect coverage for external integrations.
3. Finish or coordinate outstanding writes and CI. Capture a fresh before snapshot in a quiescent window:
   node scripts/repository-rename-audit.mjs capture before <new-before-file.json>
4. After an explicitly authorized rename by the owner, capture after before performing unrelated ref or release changes:
   node scripts/repository-rename-audit.mjs capture after <new-after-file.json>
5. Compare:
   node scripts/repository-rename-audit.mjs compare <before-file.json> <after-file.json>
   Both snapshots must be quiescent, and the after snapshot must have a later capture timestamp. Investigate discrepancies; do not infer preservation if comparison fails. Verify external consumers and only then update their URLs, test clean clones/installation, and preserve rollback evidence.

The capture checks numeric repository identity, default branch and visibility, exact heads/tags, open PR IDs/base/head, sampled issue IDs/state, release and asset IDs/metadata, ruleset contracts, and old/new API and HTTPS Git alias equivalence after rename. GitHub CLI/API access is required. The comparison is not an authorization mechanism, does not prove all closed issue/PR history, does not download release asset contents, and does not check SSH remotes or externally managed settings.

Unit tests: node --test scripts/repository-rename-audit.test.mjs (2 pass). Real before capture: pass. Actual after capture and cutover remain unperformed.
