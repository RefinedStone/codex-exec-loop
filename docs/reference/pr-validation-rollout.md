# PR Validation Rollout and Rollback Runbook

[한국어](../ko/reference/pr-validation-rollout.md)

This runbook is the shipped operator contract for moving post-merge validation from passive
observation to automatic Planning Queue remediation. It never authorizes a GitHub Ruleset change.

## Runtime Modes

The repository-local `akra.prValidationMode` setting accepts only these values:

| Mode | Provider observation | New Queue admission | Durable history |
| --- | --- | --- | --- |
| `off` | stopped | blocked | retained |
| `observe` | enabled | blocked | retained and updated |
| `remediate` | enabled | enabled for typed actionable findings only | retained and updated |

An unset value defaults to `observe`. Set it from the canonical repository with:

```text
git config --local akra.prValidationMode observe
git config --local akra.prValidationMode remediate
git config --local akra.prValidationMode off
```

The Admin Validation Rail projects the effective mode as `OFF`, `SHADOW`, or `REMEDIATION`, and
states separately whether Queue admission is blocked. The setting does not change branch
protection or GitHub required checks.

## CI Contract

- `Fast Gate` aggregates CI scope, Rust lint/architecture when selected, Node/Admin when selected,
  and explicit smoke scope. It intentionally leaves the long Rust test and portable-platform jobs
  outside merge admission.
- `CI Gate` continues to aggregate every job selected for a pull request and remains the required
  Ruleset context until a separate user-approved operation changes it.
- `Post-Merge Gate` aggregates the complete selected push validation on the exact integrated
  evidence SHA.

The long jobs start in parallel with Fast Gate rather than after merge. A successful Fast Gate can
therefore shorten admission latency without delaying post-merge evidence collection.

## Evidence Collection

Run the deterministic contract suite, then collect the live GitHub sample:

```text
cargo test --locked pr_validation -- --nocapture
cargo test --locked admin_debug_harness_all_validation_scenarios_keep_board_and_scene_semantics_aligned -- --nocapture
node --test scripts/pr-validation-rollout-evidence.test.mjs
node scripts/pr-validation-rollout-evidence.mjs collect \
  --repository RefinedStone/codex-exec-loop \
  --base prerelease \
  --sample-size 10 \
  --contracts docs/validation/artifacts/post-merge-validation-rollout-2026-08-10/contract-evidence.json \
  --json-out docs/validation/artifacts/post-merge-validation-rollout-2026-08-10/evidence.json \
  --markdown-out docs/validation/artifacts/post-merge-validation-rollout-2026-08-10/README.md
```

The collector is read-only with respect to GitHub. It fails closed when contract evidence is
missing, fewer than ten PRs or less than 24 hours are represented, an Actions target SHA differs
from the merge evidence SHA, the authenticated core quota reaches 50%, a canary is unavailable, or
any deterministic failure count is non-zero. Historical Fast Gate timing is explicitly labeled as
projected until the stable job has actual runs.

## Canary Policy

- Production `prerelease` permits only a successful canary: an ordinary reviewed merge whose exact
  evidence SHA reaches a successful Post-Merge Gate.
- Failure canaries run in the application-owned/Admin debug harness or a disposable test
  repository. Never merge an intentionally broken commit into production `prerelease`.
- `remediate` may be selected only when the checked-in evidence says `ready_for_remediate`.
- A Ruleset context change still requires explicit user approval after actual Fast Gate timing is
  available.

## Rollback

1. Run `git config --local akra.prValidationMode observe`. This immediately blocks new remediation
   admission while preserving observation, records, findings, and existing Queue task history.
2. If provider traffic itself must stop, use `off`. Do not delete validation records or SQLite
   rows; resume with `observe` after the incident.
3. Existing remediation tasks remain ordinary Planning Queue work. Cancel or acknowledge them
   through the typed application workflow instead of deleting correlation history.
4. If a Ruleset had been changed under a separate approval, restore required `CI Gate` first, then
   investigate Fast Gate. Do not add a bypass actor or disable protection.
5. Preserve each record's versioned expected context until active records settle or are explicitly
   acknowledged.

## Ruleset Approval Package

Before requesting approval, attach:

- actual and projected Fast Gate p50/p95;
- current CI Gate p50/p95;
- Post-Merge Gate failure rate and successful production canary URL;
- false-actionable, duplicate-remediation, stale, lease-takeover, phase-consistency, and SHA
  mismatch counts;
- GitHub API quota usage;
- the rollback sequence above.

The checked-in evidence for this rollout is
[`post-merge-validation-rollout-2026-08-10`](../validation/artifacts/post-merge-validation-rollout-2026-08-10/README.md).

The read-only human approval packet is
[`admin-pr-validation-approval-package-2026-08-11`](../validation/artifacts/admin-pr-validation-approval-package-2026-08-11/README.md).
Its GitHub API snapshot found active repository Ruleset `Protect prerelease delivery` on
`refs/heads/prerelease`, required context `CI Gate`, and zero bypass actors. The packet contains no
Ruleset command and performed no protection write. It separates the nine projected-only matching
PRs from the one actual stable Fast Gate run before comparing each cohort with its matching CI Gate
timing.
