import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const repositoryRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const rolloutDirectory = path.join(
  repositoryRoot,
  "docs",
  "validation",
  "artifacts",
  "post-merge-validation-rollout-2026-08-10",
);
const approvalDirectory = path.join(
  repositoryRoot,
  "docs",
  "validation",
  "artifacts",
  "admin-pr-validation-approval-package-2026-08-11",
);

const readJson = (filePath) => JSON.parse(readFileSync(filePath, "utf8"));
const rollout = readJson(path.join(rolloutDirectory, "evidence.json"));
const approval = readJson(path.join(approvalDirectory, "approval-package.json"));
const browser = readJson(path.join(approvalDirectory, "browser-metrics.json"));

const nearestRank = (values, percentile) => {
  const ordered = [...values].sort((left, right) => left - right);
  assert.ok(ordered.length > 0, "nearest-rank input must not be empty");
  return ordered[Math.max(0, Math.ceil(percentile * ordered.length) - 1)];
};

const percentFaster = (fastSeconds, baselineSeconds) =>
  Number((((baselineSeconds - fastSeconds) * 100) / baselineSeconds).toFixed(2));

test("approval package keeps actual and projected Fast Gate cohorts separate", () => {
  const projectedRows = rollout.sample.rows.filter(
    (row) => row.preMerge.fastGate.source === "projected_from_existing_jobs",
  );
  const actualRows = rollout.sample.rows.filter(
    (row) => row.preMerge.fastGate.source === "actual",
  );
  const projected = approval.gateComparisons.find(
    (entry) => entry.cohort === "projected_only_matching_pull_requests",
  );
  const actual = approval.gateComparisons.find(
    (entry) => entry.cohort === "actual_stable_fast_gate_matching_pull_request",
  );

  assert.equal(projectedRows.length, 9);
  assert.equal(actualRows.length, 1);
  assert.equal(projected.sampleCount, projectedRows.length);
  assert.equal(actual.sampleCount, actualRows.length);

  const projectedFast = projectedRows.map((row) => row.preMerge.fastGate.seconds);
  const projectedCi = projectedRows.map((row) => row.preMerge.ciGate.seconds);
  assert.equal(projected.fastGate.p50Seconds, nearestRank(projectedFast, 0.5));
  assert.equal(projected.fastGate.p95Seconds, nearestRank(projectedFast, 0.95));
  assert.equal(projected.ciGate.p50Seconds, nearestRank(projectedCi, 0.5));
  assert.equal(projected.ciGate.p95Seconds, nearestRank(projectedCi, 0.95));
  assert.equal(
    projected.delta.p50SecondsFaster,
    projected.ciGate.p50Seconds - projected.fastGate.p50Seconds,
  );
  assert.equal(
    projected.delta.p95SecondsFaster,
    projected.ciGate.p95Seconds - projected.fastGate.p95Seconds,
  );
  assert.equal(
    projected.delta.p50PercentFaster,
    percentFaster(projected.fastGate.p50Seconds, projected.ciGate.p50Seconds),
  );
  assert.equal(
    projected.delta.p95PercentFaster,
    percentFaster(projected.fastGate.p95Seconds, projected.ciGate.p95Seconds),
  );

  const actualRow = actualRows[0];
  assert.equal(actual.pullRequestNumber, actualRow.number);
  assert.equal(actual.fastGate.p50Seconds, actualRow.preMerge.fastGate.seconds);
  assert.equal(actual.ciGate.p50Seconds, actualRow.preMerge.ciGate.seconds);
  assert.equal(
    actual.delta.p50PercentFaster,
    percentFaster(actual.fastGate.p50Seconds, actual.ciGate.p50Seconds),
  );
  assert.equal(approval.sourceEvidence.historicalMixedFastGate.label, "mixed_actual_and_projected");
});

test("approval safety, freshness, and quota facts match checked-in evidence", () => {
  assert.equal(approval.disposition.requestedRulesetMutation, false);
  assert.equal(approval.disposition.automaticRulesetMutationAllowed, false);
  assert.equal(approval.rulesetSnapshot.readOnly, true);
  assert.deepEqual(approval.rulesetSnapshot.requiredStatusChecks, ["CI Gate"]);
  assert.equal(approval.rulesetSnapshot.bypassActorCount, 0);

  assert.equal(approval.postMerge.sampleCount, rollout.postMerge.sampleCount);
  assert.equal(approval.postMerge.failureCount, rollout.postMerge.failureCount);
  assert.equal(approval.postMerge.failureRatePercent, rollout.postMerge.failureRatePercent);
  assert.equal(approval.safetyCounters.falseActionable, rollout.criteria.falseActionable.observed);
  assert.equal(
    approval.safetyCounters.duplicateRemediation,
    rollout.criteria.duplicateRemediation.observed,
  );
  assert.equal(
    approval.safetyCounters.expiredLeaseTakeoverFailure,
    rollout.criteria.expiredLeaseTakeoverFailure.observed,
  );
  assert.equal(
    approval.safetyCounters.adminCliTuiPhaseMismatch,
    rollout.criteria.adminCliTuiPhaseMismatch.observed,
  );
  assert.equal(
    approval.safetyCounters.evidenceShaMismatch,
    rollout.criteria.evidenceShaMismatch.observed,
  );
  assert.equal(approval.githubApiQuota.evidenceCollection.used, rollout.quota.used);
  assert.equal(approval.githubApiQuota.evidenceCollection.usedPercent, rollout.quota.usedPercent);

  const ageSeconds =
    (Date.parse(approval.freshness.capturedAt) - Date.parse(approval.freshness.evidenceGeneratedAt)) /
    1000;
  assert.equal(approval.freshness.ageSeconds, ageSeconds);
  assert.ok(ageSeconds < approval.freshness.freshnessLimitHours * 60 * 60);
});

test("browser evidence proves bounded stable desktop and narrow surfaces", () => {
  assert.equal(browser.commit, approval.browserEvidence.captureCommit);
  assert.equal(browser.api.latestStatus, "ready");
  assert.equal(browser.observationMillis, approval.browserEvidence.observationMillis);
  assert.equal(browser.api.dashboardBytes, approval.browserEvidence.dashboardBytes);
  assert.equal(browser.api.evidenceDetailBytes, approval.browserEvidence.evidenceDetailBytes);
  assert.equal(browser.wide.nodeDelta, 0);
  assert.equal(browser.wide.scrollHeightDelta, 0);
  assert.equal(browser.wide.before.horizontalOverflowPixels, 0);
  assert.equal(browser.wide.after.horizontalOverflowPixels, 0);
  assert.equal(browser.narrow.beforeDrawer.horizontalOverflowPixels, 0);
  assert.equal(browser.narrow.dom.horizontalOverflowPixels, 0);
  assert.equal(browser.wide.browserWarningOrErrorCount, 0);
  assert.equal(browser.narrow.browserWarningOrErrorCount, 0);
  assert.equal(browser.api.collection.staleLeaseRecoveryCount, 0);
  assert.equal(browser.api.collection.identityConflictCount, 0);
});

test("approval artifact checksums cover the committed human and machine evidence", () => {
  const lines = readFileSync(path.join(approvalDirectory, "SHA256SUMS"), "utf8")
    .trim()
    .split(/\r?\n/);
  assert.equal(lines.length, 5);
  for (const line of lines) {
    const match = line.match(/^([0-9a-f]{64})  ([^/\\]+)$/);
    assert.ok(match, `invalid checksum row: ${line}`);
    const [, expected, fileName] = match;
    const actual = createHash("sha256")
      .update(readFileSync(path.join(approvalDirectory, fileName)))
      .digest("hex");
    assert.equal(actual, expected, `checksum mismatch for ${fileName}`);
  }
});
