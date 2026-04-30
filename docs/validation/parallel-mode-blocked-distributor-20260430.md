# Parallel Mode Blocked Distributor Validation

Date: 2026-04-30

This document is the integration-side seed for a live production blocked-distributor validation.

The queued source branch intentionally adds the same path with different content so `akra parallel-tick` must surface a blocked distributor queue head instead of silently integrating it.

Observed run:

- Baseline: `0bc721bd39c28155043cc710ee85996642804610`
- Conflict seed commit: `ebc4418`
- Source branch: `akra-agent/slot-1/production-blocked-distributor-e2e`
- Source commit: `9d346be1cdb1831df6b7c0c507514b292c3cc682`
- Pull request: `#558`
- Blocked state: queue head `blocked`; slot lease remained `running`; conflict file was `docs/validation/parallel-mode-blocked-distributor-20260430.md`.
- Result: `akra parallel-tick` pushed the source branch, opened the PR, verified readiness, attempted integration, aborted the failed cherry-pick, persisted the conflict file list, and left the queue head blocked for operator recovery.
