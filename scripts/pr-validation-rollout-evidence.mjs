#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { pathToFileURL } from "node:url";

export const ROLLOUT_SAMPLE_SIZE = 10;
export const ROLLOUT_WINDOW_HOURS = 24;

const FAST_JOB_NAMES = new Set([
  "CI Scope",
  "Rust Lint and Architecture",
  "Node and Admin Surfaces",
  "Rust Smoke Check",
]);

const CONTRACT_CRITERIA = [
  ["duplicateRemediation", "duplicate remediation"],
  ["falseActionable", "false actionable review/comment"],
  ["expiredLeaseTakeoverFailure", "expired lease takeover failure"],
  ["healthyProviderStale", "healthy-provider stale beyond 2x SLA"],
  ["adminCliTuiPhaseMismatch", "Admin / CLI / TUI phase mismatch"],
];

function required(value, label) {
  if (value === undefined || value === null || value === "") {
    throw new Error(`${label} is required`);
  }
  return value;
}

function asDate(value, label) {
  const date = new Date(required(value, label));
  if (!Number.isFinite(date.getTime())) {
    throw new Error(`${label} is not an ISO timestamp`);
  }
  return date;
}

function secondsBetween(start, end) {
  if (!start || !end) return null;
  const seconds = (asDate(end, "completion time") - asDate(start, "run creation time")) / 1000;
  return Number.isFinite(seconds) && seconds >= 0 ? Math.round(seconds) : null;
}

export function nearestRankPercentile(values, percentile) {
  if (!Array.isArray(values) || values.length === 0) return null;
  if (!(percentile > 0 && percentile <= 1)) {
    throw new Error("percentile must be greater than 0 and no greater than 1");
  }
  const sorted = values.filter(Number.isFinite).toSorted((left, right) => left - right);
  if (sorted.length === 0) return null;
  return sorted[Math.max(0, Math.ceil(sorted.length * percentile) - 1)];
}

function jobByName(jobs, name) {
  return jobs.find((job) => job.name === name) || null;
}

function gateDuration(run, jobs, name) {
  const job = jobByName(jobs, name);
  if (!job) return null;
  return {
    seconds: secondsBetween(run.createdAt, job.completedAt),
    conclusion: job.conclusion,
    jobUrl: job.url,
  };
}

function fastGateDuration(run, jobs) {
  const actual = gateDuration(run, jobs, "Fast Gate");
  if (actual) return { ...actual, source: "actual" };

  const selected = jobs.filter(
    (job) => FAST_JOB_NAMES.has(job.name) && job.conclusion !== "skipped",
  );
  if (selected.length === 0 || selected.some((job) => job.conclusion !== "success")) {
    return null;
  }
  const completedAt = selected
    .map((job) => job.completedAt)
    .filter(Boolean)
    .toSorted()
    .at(-1);
  return {
    seconds: secondsBetween(run.createdAt, completedAt),
    conclusion: "success",
    jobUrl: null,
    source: "projected_from_existing_jobs",
  };
}

function summarizeRun(run, jobs) {
  if (!run) return null;
  return {
    id: run.id,
    event: run.event,
    headSha: run.headSha,
    conclusion: run.conclusion,
    createdAt: run.createdAt,
    updatedAt: run.updatedAt,
    url: run.url,
    fastGate: fastGateDuration(run, jobs),
    ciGate: gateDuration(run, jobs, "CI Gate"),
    postMergeGate: gateDuration(run, jobs, "Post-Merge Gate"),
  };
}

function metric(status, observed, limit, source, note) {
  return { status, observed, limit, source, note };
}

function zeroContractMetric(contractEvidence, key, label) {
  const evidence = contractEvidence?.metrics?.[key];
  if (!evidence || !Number.isFinite(evidence.failures)) {
    return metric("unavailable", null, 0, "deterministic_contract", `${label} evidence missing`);
  }
  return metric(
    evidence.failures === 0 ? "pass" : "fail",
    evidence.failures,
    0,
    "deterministic_contract",
    evidence.note || label,
  );
}

function percentileMetric(values, label) {
  return {
    sampleCount: values.length,
    p50Seconds: nearestRankPercentile(values, 0.5),
    p95Seconds: nearestRankPercentile(values, 0.95),
    label,
  };
}

export function buildRolloutEvidence(input) {
  const pulls = [...input.pulls]
    .sort((left, right) => asDate(right.mergedAt, "mergedAt") - asDate(left.mergedAt, "mergedAt"))
    .slice(0, input.sampleSize || ROLLOUT_SAMPLE_SIZE);
  if (pulls.length === 0) throw new Error("at least one merged pull request is required");

  const latestMergedAt = asDate(pulls[0].mergedAt, "latest mergedAt");
  const earliestMergedAt = asDate(pulls.at(-1).mergedAt, "earliest mergedAt");
  const windowHours = Number(((latestMergedAt - earliestMergedAt) / 3_600_000).toFixed(2));
  const rows = pulls.map((pull) => {
    const preMerge = summarizeRun(pull.preMergeRun, pull.preMergeJobs || []);
    const postMerge = summarizeRun(pull.postMergeRun, pull.postMergeJobs || []);
    return {
      number: pull.number,
      title: pull.title,
      url: pull.url,
      mergedAt: pull.mergedAt,
      headSha: pull.headSha,
      mergeSha: pull.mergeSha,
      preMerge,
      postMerge,
      evidenceShaMatchesActionsTarget: Boolean(postMerge && postMerge.headSha === pull.mergeSha),
    };
  });

  const fastValues = rows.map((row) => row.preMerge?.fastGate?.seconds).filter(Number.isFinite);
  const sampleActualFastValues = rows
    .filter((row) => row.preMerge?.fastGate?.source === "actual")
    .map((row) => row.preMerge.fastGate.seconds)
    .filter(Number.isFinite);
  const actualFastRuns = [
    ...rows.map((row) => row.preMerge),
    ...(input.actualFastRuns || []).map((candidate) =>
      summarizeRun(candidate.run, candidate.jobs || [])),
  ]
    .filter((run) => run?.fastGate?.source === "actual")
    .filter((run, index, runs) => runs.findIndex((candidate) => candidate.id === run.id) === index);
  const actualFastValues = actualFastRuns
    .map((run) => run.fastGate.seconds)
    .filter(Number.isFinite);
  const ciValues = rows.map((row) => row.preMerge?.ciGate?.seconds).filter(Number.isFinite);
  const postMergeValues = rows
    .map((row) => row.postMerge?.postMergeGate?.seconds)
    .filter(Number.isFinite);
  const postMergeGates = rows
    .map((row) => row.postMerge?.postMergeGate)
    .filter(Boolean);
  const postMergeFailures = postMergeGates.filter((gate) => gate.conclusion !== "success").length;
  const shaMismatches = rows.filter((row) => !row.evidenceShaMatchesActionsTarget).length;
  const quotaLimit = Number(input.quota?.limit);
  const quotaUsed = Number(input.quota?.used);
  const quotaUsedPercent = Number.isFinite(quotaLimit) && quotaLimit > 0 && Number.isFinite(quotaUsed)
    ? Number(((quotaUsed / quotaLimit) * 100).toFixed(2))
    : null;

  const criteria = {
    sampleWindow: metric(
      pulls.length >= ROLLOUT_SAMPLE_SIZE && windowHours >= ROLLOUT_WINDOW_HOURS ? "pass" : "fail",
      { pullRequests: pulls.length, windowHours },
      { pullRequests: ROLLOUT_SAMPLE_SIZE, windowHours: ROLLOUT_WINDOW_HOURS },
      "github_live_sample",
      "both the PR count and elapsed window must pass",
    ),
    evidenceShaMismatch: metric(
      shaMismatches === 0 ? "pass" : "fail",
      shaMismatches,
      0,
      "github_live_sample",
      "merge commit SHA compared with push workflow head SHA",
    ),
    apiBudget: metric(
      quotaUsedPercent !== null && quotaUsedPercent < 50 ? "pass" : "fail",
      quotaUsedPercent,
      50,
      "github_rate_limit",
      "percentage of the authenticated core quota currently consumed",
    ),
  };
  for (const [key, label] of CONTRACT_CRITERIA) {
    criteria[key] = zeroContractMetric(input.contractEvidence, key, label);
  }

  const productionSuccessCanary = rows.find(
    (row) => row.postMerge?.postMergeGate?.conclusion === "success"
      && row.evidenceShaMatchesActionsTarget,
  ) || null;
  const failureCanary = input.contractEvidence?.canaries?.failure;
  const failureCanaryPassed = failureCanary?.status === "pass";
  const postMergeSamplePassed = postMergeGates.length > 0 && postMergeFailures === 0;
  const criteriaPassed = Object.values(criteria).every((entry) => entry.status === "pass");
  const readyForRemediate = Boolean(
    criteriaPassed && productionSuccessCanary && failureCanaryPassed && postMergeSamplePassed,
  );

  return {
    schemaVersion: 1,
    generatedAt: input.generatedAt,
    repository: input.repository,
    baseBranch: input.baseBranch,
    decision: {
      status: readyForRemediate ? "ready_for_remediate" : "hold_observe",
      recommendedSchedulerMode: readyForRemediate ? "remediate" : "observe",
      queueAdmissionEnabled: readyForRemediate,
      rulesetChange: "approval_required",
      reason: readyForRemediate
        ? "shadow and deterministic canary criteria passed; Ruleset remains unchanged"
        : "one or more rollout criteria are missing or failed; keep Queue admission disabled",
    },
    sample: {
      pullRequestCount: pulls.length,
      earliestMergedAt: earliestMergedAt.toISOString(),
      latestMergedAt: latestMergedAt.toISOString(),
      windowHours,
      rows,
    },
    timings: {
      fastGate: percentileMetric(
        fastValues,
        sampleActualFastValues.length > 0 ? "mixed_actual_and_projected" : "projected",
      ),
      actualFastGate: percentileMetric(actualFastValues, "actual"),
      ciGate: percentileMetric(ciValues, "actual"),
      postMergeGate: percentileMetric(postMergeValues, "actual"),
    },
    actualFastGateRuns: actualFastRuns.map((run) => ({
      id: run.id,
      headSha: run.headSha,
      conclusion: run.fastGate.conclusion,
      seconds: run.fastGate.seconds,
      runUrl: run.url,
      jobUrl: run.fastGate.jobUrl,
    })),
    postMerge: {
      sampleCount: postMergeGates.length,
      failureCount: postMergeFailures,
      failureRatePercent: postMergeGates.length === 0
        ? null
        : Number(((postMergeFailures / postMergeGates.length) * 100).toFixed(2)),
    },
    quota: {
      ...input.quota,
      usedPercent: quotaUsedPercent,
    },
    criteria,
    canaries: {
      productionSuccess: productionSuccessCanary
        ? {
            status: "pass",
            pullRequestNumber: productionSuccessCanary.number,
            mergeSha: productionSuccessCanary.mergeSha,
            runId: productionSuccessCanary.postMerge.id,
            runUrl: productionSuccessCanary.postMerge.url,
          }
        : { status: "unavailable" },
      failure: failureCanary || { status: "unavailable" },
    },
    contractEvidence: input.contractEvidence || null,
  };
}

function formatSeconds(value) {
  return Number.isFinite(value) ? `${value}s` : "-";
}

function formatObserved(value) {
  if (value === null || value === undefined) return "-";
  return typeof value === "object" ? JSON.stringify(value) : String(value);
}

export function renderRolloutMarkdown(evidence) {
  const timingRow = (label, timing) => `| ${label} | ${timing.sampleCount} | ${formatSeconds(timing.p50Seconds)} | ${formatSeconds(timing.p95Seconds)} | ${timing.label} |`;
  const sampleRows = evidence.sample.rows.map((row) => {
    const fast = row.preMerge?.fastGate;
    const post = row.postMerge?.postMergeGate;
    return `| [#${row.number}](${row.url}) | \`${row.mergeSha.slice(0, 8)}\` | ${formatSeconds(fast?.seconds)} (${fast?.source || "missing"}) | ${formatSeconds(row.preMerge?.ciGate?.seconds)} | ${formatSeconds(post?.seconds)} / ${post?.conclusion || "not available"} | ${row.evidenceShaMatchesActionsTarget ? "pass" : "FAIL"} |`;
  });
  const criterionRows = Object.entries(evidence.criteria).map(([key, value]) =>
    `| ${key} | ${value.status} | ${formatObserved(value.observed)} | ${formatObserved(value.limit)} | ${value.source} |`,
  );
  const actualFastRuns = evidence.actualFastGateRuns.length === 0
    ? "- no completed stable Fast Gate run was supplied yet"
    : evidence.actualFastGateRuns.map((run) =>
      `- [run ${run.id}](${run.runUrl}) / \`${run.headSha.slice(0, 8)}\` / ${formatSeconds(run.seconds)} / ${run.conclusion}`,
    ).join("\n");
  const browserEvidence = evidence.contractEvidence?.browser;
  const browserSection = browserEvidence
    ? `\n## Browser Validation\n\n- status: **${browserEvidence.status}**\n- viewport checks: ${browserEvidence.viewports.map((viewport) => `${viewport.width}x${viewport.height}`).join(", ")}\n- horizontal overflow: ${browserEvidence.horizontalOverflowCount}\n- console errors: ${browserEvidence.consoleErrorCount}\n- lifecycle: ${browserEvidence.lifecycle.join(" -> ")}\n- validation phases: ${browserEvidence.validationPhases.join(" -> ")}\n\n![wide Admin rollout](browser/${browserEvidence.screenshots.wide})\n\n![mobile completed rollout](browser/${browserEvidence.screenshots.mobileComplete})\n`
    : "";
  return `# PR Validation Rollout Evidence

Generated at \`${evidence.generatedAt}\` for \`${evidence.repository}\` / \`${evidence.baseBranch}\`.

## Decision

- status: **${evidence.decision.status}**
- recommended scheduler mode: \`${evidence.decision.recommendedSchedulerMode}\`
- Queue admission: ${evidence.decision.queueAdmissionEnabled ? "enabled" : "blocked"}
- Ruleset change: **${evidence.decision.rulesetChange}**; this collector never mutates Rulesets
- rationale: ${evidence.decision.reason}

## Sample Window

- ${evidence.sample.pullRequestCount} merged PRs
- ${evidence.sample.windowHours} hours from \`${evidence.sample.earliestMergedAt}\` to \`${evidence.sample.latestMergedAt}\`
- older rows without the stable Post-Merge Gate remain visible as \`not available\`; they are not counted as successful canaries

| PR | merge SHA | Fast Gate | CI Gate | Post-Merge Gate | Actions target SHA |
| --- | --- | ---: | ---: | ---: | --- |
${sampleRows.join("\n")}

## Gate Timing

Nearest-rank percentiles include queue and job time from workflow creation until the aggregate gate
is complete. Historical Fast Gate values are projected from the exact jobs the new aggregate waits
for; actual values are reported separately once the stable job exists.

| Gate | samples | p50 | p95 | source |
| --- | ---: | ---: | ---: | --- |
${timingRow("Fast Gate", evidence.timings.fastGate)}
${timingRow("Actual Fast Gate", evidence.timings.actualFastGate)}
${timingRow("CI Gate", evidence.timings.ciGate)}
${timingRow("Post-Merge Gate", evidence.timings.postMergeGate)}

Actual stable Fast Gate runs supplied independently of the merged-PR sample:

${actualFastRuns}

- Post-Merge Gate failure rate: ${evidence.postMerge.failureRatePercent ?? "-"}% (${evidence.postMerge.failureCount}/${evidence.postMerge.sampleCount})
- GitHub core quota used: ${evidence.quota.usedPercent ?? "-"}% (${evidence.quota.used ?? "-"}/${evidence.quota.limit ?? "-"})
- reported core counter delta during collection: ${evidence.quota.collectorRequests ?? "unknown"}

## Rollout Criteria

| Criterion | status | observed | limit | evidence |
| --- | --- | --- | --- | --- |
${criterionRows.join("\n")}

## Canaries

- production success: ${evidence.canaries.productionSuccess.status}${evidence.canaries.productionSuccess.runUrl ? ` — [PR #${evidence.canaries.productionSuccess.pullRequestNumber} run](${evidence.canaries.productionSuccess.runUrl})` : ""}
- failure canary: ${evidence.canaries.failure.status}${evidence.canaries.failure.note ? ` — ${evidence.canaries.failure.note}` : ""}

${browserSection}
## Safety Boundary

The production sample contains no intentionally broken merge. Failure behavior comes from the
deterministic application/Admin harness. Switching the required Ruleset context from \`CI Gate\` to
\`Fast Gate\` remains a separate user-approved operation; no bypass actor or protection weakening is
part of this evidence run.
`;
}

function ghJson(args) {
  return JSON.parse(execFileSync("gh", args, {
    encoding: "utf8",
    maxBuffer: 32 * 1024 * 1024,
    stdio: ["ignore", "pipe", "pipe"],
  }));
}

function normalizeRun(run) {
  return {
    id: run.id,
    event: run.event,
    headSha: run.head_sha,
    conclusion: run.conclusion,
    createdAt: run.created_at,
    updatedAt: run.updated_at,
    url: run.html_url,
    attempt: run.run_attempt || 1,
  };
}

function normalizeJob(job) {
  return {
    name: job.name,
    conclusion: job.conclusion,
    startedAt: job.started_at,
    completedAt: job.completed_at,
    url: job.html_url,
  };
}

function latestRun(runs, event, sha) {
  return runs
    .filter((run) => run.event === event && run.head_sha === sha && run.status === "completed")
    .toSorted((left, right) => (right.run_attempt || 1) - (left.run_attempt || 1)
      || asDate(right.updated_at, "run updated_at") - asDate(left.updated_at, "run updated_at"))[0] || null;
}

function readContractEvidence(path) {
  return path ? JSON.parse(readFileSync(resolve(path), "utf8")) : null;
}

function rateSnapshot() {
  return ghJson(["api", "rate_limit"]).resources.core;
}

export function collectGithubRolloutInput(options) {
  const repository = options.repository || ghJson(["repo", "view", "--json", "nameWithOwner"]).nameWithOwner;
  const baseBranch = options.baseBranch || "prerelease";
  const sampleSize = options.sampleSize || ROLLOUT_SAMPLE_SIZE;
  const workflow = options.workflow || "native-pr-checks.yml";
  const rateBefore = rateSnapshot();
  const pullCandidates = ghJson([
    "pr", "list", "--repo", repository, "--state", "merged", "--base", baseBranch,
    "--limit", String(Math.max(sampleSize, 50)),
    "--json", "number,title,url,mergedAt,headRefOid,mergeCommit",
  ])
    .filter((pull) => pull.mergeCommit?.oid)
    .toSorted((left, right) => asDate(right.mergedAt, "mergedAt") - asDate(left.mergedAt, "mergedAt"))
    .slice(0, sampleSize);
  const workflowPath = `repos/${repository}/actions/workflows/${workflow}/runs`;
  const pullRuns = ghJson(["api", `${workflowPath}?event=pull_request&per_page=100`]).workflow_runs;
  const pushRuns = ghJson(["api", `${workflowPath}?event=push&branch=${baseBranch}&per_page=100`]).workflow_runs;
  const jobs = new Map();
  const loadJobs = (run) => {
    if (!run) return [];
    if (!jobs.has(run.id)) {
      const result = ghJson(["api", `repos/${repository}/actions/runs/${run.id}/jobs?per_page=100`]);
      jobs.set(run.id, result.jobs.map(normalizeJob));
    }
    return jobs.get(run.id);
  };
  const pulls = pullCandidates.map((pull) => {
    const preMergeRaw = latestRun(pullRuns, "pull_request", pull.headRefOid);
    const postMergeRaw = latestRun(pushRuns, "push", pull.mergeCommit.oid);
    return {
      number: pull.number,
      title: pull.title,
      url: pull.url,
      mergedAt: pull.mergedAt,
      headSha: pull.headRefOid,
      mergeSha: pull.mergeCommit.oid,
      preMergeRun: preMergeRaw ? normalizeRun(preMergeRaw) : null,
      preMergeJobs: loadJobs(preMergeRaw),
      postMergeRun: postMergeRaw ? normalizeRun(postMergeRaw) : null,
      postMergeJobs: loadJobs(postMergeRaw),
    };
  });
  const actualFastRuns = (options.actualFastRunIds || []).map((runId) => {
    const raw = ghJson(["api", `repos/${repository}/actions/runs/${runId}`]);
    return {
      run: normalizeRun(raw),
      jobs: loadJobs(raw),
    };
  });
  const rateAfter = rateSnapshot();
  return {
    generatedAt: new Date().toISOString(),
    repository,
    baseBranch,
    sampleSize,
    pulls,
    actualFastRuns,
    quota: {
      limit: rateAfter.limit,
      remaining: rateAfter.remaining,
      used: rateAfter.used,
      reset: rateAfter.reset,
      collectorRequests: Math.max(0, Number(rateAfter.used) - Number(rateBefore.used)),
    },
    contractEvidence: readContractEvidence(options.contractPath),
  };
}

function parseOptions(args) {
  const options = {};
  for (let index = 0; index < args.length; index += 1) {
    const argument = args[index];
    if (!argument.startsWith("--")) throw new Error(`unexpected argument: ${argument}`);
    const key = argument.slice(2);
    const value = args[index + 1];
    if (!value || value.startsWith("--")) throw new Error(`${argument} requires a value`);
    options[key] = value;
    index += 1;
  }
  return {
    repository: options.repository,
    baseBranch: options.base || "prerelease",
    workflow: options.workflow || "native-pr-checks.yml",
    sampleSize: options["sample-size"] ? Number(options["sample-size"]) : ROLLOUT_SAMPLE_SIZE,
    contractPath: options.contracts,
    actualFastRunIds: options["actual-fast-run"]
      ? options["actual-fast-run"].split(",").map((value) => value.trim()).filter(Boolean)
      : [],
    jsonOut: options["json-out"],
    markdownOut: options["markdown-out"],
  };
}

function writeOutput(path, content) {
  if (!path) return;
  const absolute = resolve(path);
  mkdirSync(dirname(absolute), { recursive: true });
  writeFileSync(absolute, content, "utf8");
}

function main() {
  const [command, ...args] = process.argv.slice(2);
  if (command !== "collect") {
    throw new Error("usage: node scripts/pr-validation-rollout-evidence.mjs collect [--repository owner/repo] [--base prerelease] [--contracts file] [--actual-fast-run run-id[,run-id]] [--json-out file] [--markdown-out file]");
  }
  const options = parseOptions(args);
  if (!Number.isInteger(options.sampleSize) || options.sampleSize < ROLLOUT_SAMPLE_SIZE) {
    throw new Error(`--sample-size must be an integer no smaller than ${ROLLOUT_SAMPLE_SIZE}`);
  }
  const evidence = buildRolloutEvidence(collectGithubRolloutInput(options));
  const json = `${JSON.stringify(evidence, null, 2)}\n`;
  const markdown = renderRolloutMarkdown(evidence);
  writeOutput(options.jsonOut, json);
  writeOutput(options.markdownOut, markdown);
  if (!options.jsonOut && !options.markdownOut) process.stdout.write(json);
  process.stderr.write(`${evidence.decision.status}: ${evidence.sample.pullRequestCount} PRs / ${evidence.sample.windowHours}h\n`);
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  main();
}
