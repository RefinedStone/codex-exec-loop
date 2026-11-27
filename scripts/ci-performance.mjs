#!/usr/bin/env node
import { appendFileSync, existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { spawn } from "node:child_process";
import { pathToFileURL } from "node:url";

const elapsed = (start, end) => {
  const milliseconds = Date.parse(end) - Date.parse(start);
  return Number.isFinite(milliseconds) && milliseconds >= 0 ? milliseconds / 1000 : null;
};
export function distribution(values) {
  const sorted = values.filter(Number.isFinite).sort((a, b) => a - b);
  if (!sorted.length) return { samples: 0, median: null, p90: null, minimum: null, maximum: null };
  const middle = Math.floor(sorted.length / 2);
  return { samples: sorted.length, median: sorted.length % 2 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2,
    // Nearest-rank p90: an observed sample, never an interpolated value.
    p90: sorted[Math.ceil(sorted.length * 0.9) - 1], minimum: sorted[0], maximum: sorted.at(-1) };
}
const cargoFinishedSeconds = (line) => {
  const match = /in (?:(\d+)m )?([\d.]+)s$/.exec(line);
  return match ? Number(match[1] ?? 0) * 60 + Number(match[2]) : 0;
};
const testResultSeconds = (line) => Number(/finished in ([\d.]+)s/.exec(line)?.[1] ?? 0);
/** Groups command records by artifact lane, label and restored target cache class (exact/fallback/miss). */
export function groupLaneSamples(lanes) {
  const groups = new Map();
  for (const { runId, lane, records, snapshots } of lanes) {
    const targetCache = snapshots.find((snapshot) => snapshot.phase === "restored")?.caches?.target?.hit ?? "unrecorded";
    for (const record of records) {
      // A failed or cancelled command is not a timing sample.
      if (record.exitCode !== 0) continue;
      const key = JSON.stringify([lane, record.label, targetCache]);
      const group = groups.get(key) ?? { lane, label: record.label, targetCache, runs: [], wall: [], compile: [], test: [] };
      group.runs.push(runId);
      group.wall.push(record.seconds);
      group.compile.push(record.cargo.finished.reduce((sum, line) => sum + cargoFinishedSeconds(line), 0));
      group.test.push(record.cargo.testResults.reduce((sum, line) => sum + testResultSeconds(line), 0));
      groups.set(key, group);
    }
  }
  return [...groups.values()].map(({ lane, label, targetCache, runs, wall, compile, test }) => ({
    lane, label, targetCache, wallSeconds: distribution(wall), cargoBuildSeconds: distribution(compile),
    testExecutionSeconds: distribution(test), runs,
  }));
}
export function summarizeWorkflow(run, jobs) {
  const rows = jobs.map((job) => ({
    name: job.name, url: job.html_url, status: job.status, conclusion: job.conclusion,
    seconds: job.status === "completed"
      ? elapsed(job.started_at, job.completed_at) ?? (job.conclusion === "skipped" ? 0 : null) : null,
    steps: (job.steps ?? []).map((step) => ({
      name: step.name, conclusion: step.conclusion,
      seconds: step.status === "completed" ? elapsed(step.started_at, step.completed_at) : null,
    })),
  }));
  const gate = jobs.find((job) => job.name === "CI Gate");
  const completed = jobs.filter((job) => job.status === "completed" && job.completed_at);
  const incompleteJobs = jobs.filter((job) => job.status !== "completed").length;
  const missingDurationJobs = rows.filter((job) => job.status === "completed" && job.seconds === null).length;
  const durationsComplete = incompleteJobs === 0 && missingDurationJobs === 0;
  const last = completed.map((job) => job.completed_at).sort().at(-1);
  return {
    url: run.html_url, commit: run.head_sha, attempt: run.run_attempt,
    status: run.status, conclusion: run.conclusion,
    gateSeconds: gate?.status === "completed" && gate.conclusion === "success"
      ? elapsed(run.run_started_at ?? run.created_at, gate.completed_at) : null,
    workflowSeconds: run.status === "completed" && durationsComplete
      ? elapsed(run.run_started_at ?? run.created_at, last) : null,
    // Occupied job time is a proxy, not GitHub's rounded/billed runner minutes.
    runnerSeconds: durationsComplete ? rows.reduce((sum, row) => sum + row.seconds, 0) : null,
    incompleteJobs, missingDurationJobs, jobs: rows,
  };
}
export function compareSamples(samples) {
  const groups = new Map();
  for (const sample of samples) {
    const { cohort, environment, scenario } = sample;
    if (sample.status !== "completed" || sample.conclusion !== "success"
      || sample.incompleteJobs !== 0 || sample.missingDurationJobs !== 0
      || ["gateSeconds", "workflowSeconds", "runnerSeconds"]
        .some((field) => !Number.isFinite(sample[field]) || sample[field] < 0)) {
      throw new Error("performance comparisons require complete successful samples with all job durations and elapsed/runner time");
    }
    if (typeof cohort !== "string" || !cohort.trim()) {
      throw new Error("each sample needs an explicit cohort, such as baseline or candidate");
    }
    if (!environment || !scenario || !["cold", "cache-reuse", "unchanged", "source-change"].includes(scenario)) {
      throw new Error("each sample needs environment and a cold/cache-reuse/unchanged/source-change scenario");
    }
    for (const field of ["runner", "architecture", "toolchain", "features", "profile", "target"]) {
      if (typeof environment[field] !== "string" || !environment[field]) throw new Error(`missing environment.${field}`);
    }
    const key = JSON.stringify([cohort, scenario, ...["runner", "architecture", "toolchain", "features", "profile", "target"].map((field) => environment[field])]);
    const group = groups.get(key) ?? { cohort, scenario, environment, samples: [] };
    group.samples.push(sample);
    groups.set(key, group);
  }
  return [...groups.values()].map((group) => ({
    cohort: group.cohort, scenario: group.scenario, environment: group.environment,
    gateSeconds: distribution(group.samples.map((sample) => sample.gateSeconds)),
    runnerSeconds: distribution(group.samples.map((sample) => sample.runnerSeconds)),
    evidence: group.samples.map(({ url, commit, attempt }) => ({ url, commit, attempt })),
  }));
}
export function createCargoObserver() {
  const state = { compilingUnits: 0, checkingUnits: 0, freshUnits: 0, dirtyUnits: 0,
    dirtyReasons: [], finished: [], testResults: [] };
  let pending = "";
  const line = (text) => {
    const clean = text.replace(/\x1b\[[0-9;]*m/g, "").trim();
    if (/^Compiling\s/.test(clean)) state.compilingUnits += 1;
    if (/^Checking\s/.test(clean)) state.checkingUnits += 1;
    if (/^Dirty\s/.test(clean)) {
      state.dirtyUnits += 1;
      if (state.dirtyReasons.length < 128) state.dirtyReasons.push(clean);
    }
    if (/^Fresh\s/.test(clean)) state.freshUnits += 1;
    if (/^Finished\s/.test(clean)) state.finished.push(clean);
    if (/^test result:/.test(clean)) state.testResults.push(clean);
  };
  return {
    state,
    feed(chunk) {
      pending += chunk;
      const lines = pending.split(/\r?\n/);
      pending = lines.pop();
      lines.forEach(line);
    },
    finish() { if (pending) line(pending); pending = ""; },
  };
}
export async function measure(label, command, args, {
  directory = process.env.AKRA_CI_MEASUREMENTS || "artifacts/ci-performance",
  spawnCommand = spawn, signalSource = process,
} = {}) {
  if (!label || !command) throw new Error("run requires a label and command after --");
  const start = new Date();
  const clock = performance.now();
  const observers = [createCargoObserver(), createCargoObserver()];
  const child = spawnCommand(command, args, { stdio: ["inherit", "pipe", "pipe"], shell: false });
  child.stdout.on("data", (chunk) => { process.stdout.write(chunk); observers[0].feed(chunk.toString()); });
  child.stderr.on("data", (chunk) => { process.stderr.write(chunk); observers[1].feed(chunk.toString()); });
  let requestedSignal;
  const forward = (signal) => {
    requestedSignal ??= signal;
    if (!child.killed) child.kill(signal);
  };
  const interrupt = () => forward("SIGINT");
  const terminate = () => forward("SIGTERM");
  signalSource.on("SIGINT", interrupt);
  signalSource.on("SIGTERM", terminate);
  let spawnError;
  child.on("error", (error) => { spawnError = error.message; });
  const result = await new Promise((resolve) => child.on("close", (code, signal) => resolve({ code, signal })));
  signalSource.removeListener("SIGINT", interrupt);
  signalSource.removeListener("SIGTERM", terminate);
  observers.forEach((observer) => observer.finish());
  const cargo = {
    compilingUnits: observers.reduce((sum, observer) => sum + observer.state.compilingUnits, 0),
    checkingUnits: observers.reduce((sum, observer) => sum + observer.state.checkingUnits, 0),
    dirtyUnits: observers.reduce((sum, observer) => sum + observer.state.dirtyUnits, 0),
    dirtyReasons: observers.flatMap((observer) => observer.state.dirtyReasons),
    freshUnits: observers.reduce((sum, observer) => sum + observer.state.freshUnits, 0),
    finished: observers.flatMap((observer) => observer.state.finished),
    testResults: observers.flatMap((observer) => observer.state.testResults),
  };
  // A child may handle termination and exit zero; the cancelled measurement still fails.
  const signalExitCode = (signal) => signal === "SIGINT" ? 130 : signal === "SIGTERM" ? 143 : 1;
  const exitCode = requestedSignal ? signalExitCode(requestedSignal) : result.code ?? signalExitCode(result.signal);
  const record = {
    schema: 1, label, command, startedAt: start.toISOString(), finishedAt: new Date().toISOString(),
    seconds: (performance.now() - clock) / 1000,
    exitCode, childExitCode: result.code, signal: result.signal, requestedSignal, spawnError, cargo,
    commit: process.env.GITHUB_SHA, runId: process.env.GITHUB_RUN_ID, attempt: process.env.GITHUB_RUN_ATTEMPT,
    job: process.env.GITHUB_JOB, os: process.env.RUNNER_OS || process.platform, architecture: process.env.RUNNER_ARCH || process.arch,
    toolchain: process.env.RUSTUP_TOOLCHAIN, runnerImage: process.env.ImageOS, runnerImageVersion: process.env.ImageVersion,
    target: process.env.AKRA_CI_TARGET, features: process.env.AKRA_CI_FEATURES, profile: process.env.AKRA_CI_PROFILE || "dev/test",
  };
  mkdirSync(directory, { recursive: true });
  appendFileSync(join(directory, "steps.jsonl"), JSON.stringify(record) + "\n");
  if (spawnError) console.error(spawnError);
  return exitCode;
}
function report(directory) {
  const path = join(directory, "steps.jsonl");
  const records = existsSync(path) ? readFileSync(path, "utf8").trim().split(/\r?\n/).filter(Boolean).map(JSON.parse) : [];
  const lines = [
    "## CI measurements", "",
    "| Command group | Seconds | Compiled units | Exit |", "|---|---:|---:|---:|",
    ...records.map((record) => `| ${record.label} | ${record.seconds.toFixed(2)} | ${record.cargo.compilingUnits} | ${record.exitCode ?? record.signal ?? "error"} |`),
    "", "Command wall time includes build and execution. Cargo Finished/test-result lines and HTML timings separate those costs.",
    "Cache restore/save and job duration come from the Actions jobs API; compiled units are observed Cargo log lines, not cache hit percentages.", "",
  ];
  const markdown = lines.join("\n");
  mkdirSync(directory, { recursive: true });
  writeFileSync(join(directory, "summary.md"), markdown);
  if (process.env.GITHUB_STEP_SUMMARY) appendFileSync(process.env.GITHUB_STEP_SUMMARY, markdown);
  console.log(markdown);
}
function save(path, data) {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, JSON.stringify(data, null, 2) + "\n");
}
export async function main(args) {
  if (args[0] === "run") {
    const divider = args.indexOf("--");
    if (divider !== 2) throw new Error("usage: run <label> -- <command> [args...]");
    process.exitCode = await measure(args[1], args[3], args.slice(4));
  } else if (args[0] === "report") {
    report(args[1] ?? process.env.AKRA_CI_MEASUREMENTS ?? "artifacts/ci-performance");
  } else if (args[0] === "summarize") {
    const [runPath, jobsPath, output] = args.slice(1);
    const run = JSON.parse(readFileSync(runPath, "utf8"));
    const payload = JSON.parse(readFileSync(jobsPath, "utf8"));
    const jobs = Array.isArray(payload) ? payload.flatMap((page) => page.jobs ?? page) : payload.jobs;
    if (!Array.isArray(jobs) || !output) throw new Error("usage: summarize <run.json> <jobs.json> <output.json>");
    // Jobs must be fetched for this exact run attempt, not mixed retry attempts.
    save(output, summarizeWorkflow(run, jobs));
  } else if (args[0] === "compare") {
    const [input, output] = args.slice(1);
    if (!input || !output) throw new Error("usage: compare <samples.json> <comparison.json>");
    save(output, compareSamples(JSON.parse(readFileSync(input, "utf8"))));
  } else if (args[0] === "lanes") {
    const [input, output] = args.slice(1);
    if (!input || !output) throw new Error("usage: lanes <downloads-dir> <lanes.json>");
    // Layout of `gh run download RUN_ID -p 'ci-performance-*' -D <downloads-dir>/RUN_ID`.
    const lanes = readdirSync(input).flatMap((runId) => readdirSync(join(input, runId)).map((lane) => {
      const base = join(input, runId, lane, "artifacts", "ci-performance");
      const lines = (name) => existsSync(join(base, name))
        ? readFileSync(join(base, name), "utf8").split(/\r?\n/).filter(Boolean).map(JSON.parse) : [];
      return { runId, lane, records: lines("steps.jsonl"), snapshots: lines("cache-snapshots.jsonl") };
    }));
    save(output, groupLaneSamples(lanes));
  } else {
    throw new Error("usage: ci-performance.mjs <run|report|summarize|compare|lanes>");
  }
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await main(process.argv.slice(2));
}
