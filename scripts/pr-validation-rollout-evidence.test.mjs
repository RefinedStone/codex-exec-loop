import assert from "node:assert/strict";
import test from "node:test";

import {
  buildRolloutEvidence,
  nearestRankPercentile,
  renderRolloutMarkdown,
} from "./pr-validation-rollout-evidence.mjs";

const completedJob = (name, seconds, conclusion = "success") => ({
  name,
  conclusion,
  completedAt: new Date(Date.parse("2026-08-10T00:00:00Z") + seconds * 1000).toISOString(),
  url: `https://example.test/jobs/${name}`,
});

const completedRun = (id, event, headSha, seconds) => ({
  id,
  event,
  headSha,
  conclusion: "success",
  createdAt: "2026-08-10T00:00:00Z",
  updatedAt: new Date(Date.parse("2026-08-10T00:00:00Z") + seconds * 1000).toISOString(),
  url: `https://example.test/runs/${id}`,
});

function passingInput() {
  const pulls = Array.from({ length: 10 }, (_, index) => {
    const number = 100 + index;
    const mergeSha = `${number}`.padStart(40, "a").slice(-40);
    const mergedAt = new Date(Date.parse("2026-08-10T12:00:00Z") - index * 4 * 3_600_000).toISOString();
    return {
      number,
      title: `PR ${number}`,
      url: `https://example.test/pull/${number}`,
      mergedAt,
      headSha: `${number}`.padStart(40, "b").slice(-40),
      mergeSha,
      preMergeRun: completedRun(number, "pull_request", `head-${number}`, 500),
      preMergeJobs: [
        completedJob("CI Scope", 15),
        completedJob("Rust Lint and Architecture", 90 + index),
        completedJob("Rust Tests", 450 + index),
        completedJob("CI Gate", 500 + index),
      ],
      postMergeRun: completedRun(number + 1_000, "push", mergeSha, 520),
      postMergeJobs: [
        completedJob("CI Scope", 15),
        completedJob("Post-Merge Gate", 520 + index),
      ],
    };
  });
  return {
    generatedAt: "2026-08-10T13:00:00Z",
    repository: "acme/widgets",
    baseBranch: "prerelease",
    sampleSize: 10,
    pulls,
    quota: { limit: 5_000, remaining: 4_500, used: 500, collectorRequests: 22 },
    contractEvidence: {
      metrics: {
        duplicateRemediation: { failures: 0 },
        falseActionable: { failures: 0 },
        expiredLeaseTakeoverFailure: { failures: 0 },
        healthyProviderStale: { failures: 0 },
        adminCliTuiPhaseMismatch: { failures: 0 },
      },
      canaries: { failure: { status: "pass", note: "deterministic harness" } },
    },
  };
}

test("nearest-rank percentile is deterministic for small rollout samples", () => {
  assert.equal(nearestRankPercentile([9, 1, 5, 2, 7], 0.5), 5);
  assert.equal(nearestRankPercentile([9, 1, 5, 2, 7], 0.95), 9);
  assert.equal(nearestRankPercentile([], 0.5), null);
});

test("passing live and deterministic evidence recommends remediate without changing Rulesets", () => {
  const evidence = buildRolloutEvidence(passingInput());
  assert.equal(evidence.sample.pullRequestCount, 10);
  assert.equal(evidence.sample.windowHours, 36);
  assert.equal(evidence.criteria.sampleWindow.status, "pass");
  assert.equal(evidence.criteria.evidenceShaMismatch.observed, 0);
  assert.equal(evidence.postMerge.failureRatePercent, 0);
  assert.equal(evidence.timings.fastGate.label, "projected");
  assert.equal(evidence.decision.status, "ready_for_remediate");
  assert.equal(evidence.decision.rulesetChange, "approval_required");
});

test("an independently supplied PR run records the first actual stable Fast Gate", () => {
  const input = passingInput();
  input.actualFastRuns = [{
    run: completedRun(9_090, "pull_request", "current-pr-head", 74),
    jobs: [completedJob("Fast Gate", 74)],
  }];

  const evidence = buildRolloutEvidence(input);
  assert.equal(evidence.timings.fastGate.label, "projected");
  assert.equal(evidence.timings.actualFastGate.sampleCount, 1);
  assert.equal(evidence.timings.actualFastGate.p50Seconds, 74);
  assert.deepEqual(evidence.actualFastGateRuns.map((run) => run.id), [9_090]);
  assert.match(renderRolloutMarkdown(evidence), /run 9090.*74s/);
});

test("a wrong Actions target or missing contract evidence holds observe mode", () => {
  const input = passingInput();
  input.pulls[0].postMergeRun.headSha = "wrong-sha";
  delete input.contractEvidence.metrics.falseActionable;
  const evidence = buildRolloutEvidence(input);
  assert.equal(evidence.criteria.evidenceShaMismatch.status, "fail");
  assert.equal(evidence.criteria.falseActionable.status, "unavailable");
  assert.equal(evidence.decision.status, "hold_observe");
  assert.equal(evidence.decision.queueAdmissionEnabled, false);
});

test("the Markdown report labels projections and the approval boundary", () => {
  const markdown = renderRolloutMarkdown(buildRolloutEvidence(passingInput()));
  assert.match(markdown, /projected_from_existing_jobs/);
  assert.match(markdown, /Ruleset change: \*\*approval_required\*\*/);
  assert.match(markdown, /no intentionally broken merge/);
});
