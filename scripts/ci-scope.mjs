#!/usr/bin/env node

import { appendFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { pathToFileURL } from "node:url";

const VALID_SCOPES = new Set([
  "auto",
  "docs",
  "rust",
  "tui",
  "admin",
  "full",
  "smoke",
  "postmerge",
]);

const FULL_PATHS = [
  /^\.github\//,
  /^\.cargo\//,
  /^AGENTS\.md$/,
  /^Cargo\.(toml|lock)$/,
  /^rust-toolchain(?:\.toml)?$/,
  /^scripts\/(?:agent-plan\.|check_|ci-scope\.|package_|validate_|verify_)/,
  /^(?:deny|clippy)\.toml$/,
];

const DOC_PATHS = [
  /^docs\//,
  /^(?:README|CHANGELOG|CONTRIBUTING|SECURITY|CODE_OF_CONDUCT)\.md$/,
];

const ADMIN_PATHS = [
  /^assets\/admin\//,
  /^templates\/admin\//,
];

const NODE_PATHS = [
  /^npm\//,
  ...ADMIN_PATHS,
];

const RUST_PATHS = [
  /^src\//,
  /^tests\//,
  /^benches\//,
  /^examples\//,
  /^build\.rs$/,
  /^schema\//,
  /^templates\//,
  /^assets\//,
];

function matchesAny(path, patterns) {
  return patterns.some((pattern) => pattern.test(path));
}

function normalizePaths(paths) {
  return [...new Set(paths.map((path) => path.trim().replaceAll("\\", "/")).filter(Boolean))].sort();
}

function forcedPlan(scope) {
  switch (scope) {
    case "docs":
      return buildPlan(scope, false, false, false, false, true);
    case "admin":
      return buildPlan(scope, false, true, false, false, false);
    case "smoke":
      return buildPlan(scope, false, false, false, true, false);
    case "postmerge":
      return buildPlan(scope, true, true, true, false, false, false, true);
    case "rust":
    case "tui":
      return buildPlan(scope, true, false, true, false, false);
    case "full":
      return buildPlan(scope, true, true, true, false, false, true);
    default:
      throw new Error(`unsupported forced CI scope: ${scope}`);
  }
}

function buildPlan(
  scope,
  rust,
  node,
  portable,
  smoke,
  docsOnly,
  full = false,
  postmerge = false,
) {
  return {
    scope,
    full,
    postmerge,
    rust,
    node,
    portable,
    smoke,
    docs_only: docsOnly,
  };
}

export function classifyChangedPaths(inputPaths, requestedScope = "auto") {
  if (!VALID_SCOPES.has(requestedScope)) {
    throw new Error(`invalid CI scope: ${requestedScope}`);
  }
  if (requestedScope !== "auto") {
    return forcedPlan(requestedScope);
  }

  const paths = normalizePaths(inputPaths);
  if (paths.length === 0 || paths.some((path) => matchesAny(path, FULL_PATHS))) {
    return forcedPlan("full");
  }

  if (paths.every((path) => matchesAny(path, DOC_PATHS))) {
    return forcedPlan("docs");
  }

  let rust = false;
  let node = false;
  let portable = false;
  for (const path of paths) {
    if (matchesAny(path, DOC_PATHS)) {
      continue;
    }
    const isAdminPath = matchesAny(path, ADMIN_PATHS);
    if (matchesAny(path, NODE_PATHS)) {
      node = true;
    }
    if (!isAdminPath && matchesAny(path, RUST_PATHS)) {
      rust = true;
      portable = true;
    }
    if (!matchesAny(path, [...DOC_PATHS, ...NODE_PATHS, ...RUST_PATHS])) {
      return forcedPlan("full");
    }
  }

  if (!rust && !node) {
    return forcedPlan("full");
  }
  return buildPlan(rust && node ? "mixed" : rust ? "rust" : "admin", rust, node, portable, false, false);
}

export function evaluateGate(plan, results) {
  const required = [["scope", results.scope]];
  if (plan.rust) {
    required.push(["rust_tests", results.rust_tests], ["rust_lint", results.rust_lint]);
  }
  if (plan.node) {
    required.push(["node_surfaces", results.node_surfaces]);
  }
  if (plan.portable) {
    required.push(["portable_native", results.portable_native]);
  }
  if (plan.smoke) {
    required.push(["smoke_check", results.smoke_check]);
  }
  return required.filter(([, result]) => result !== "success").map(([job, result]) => ({ job, result }));
}

export function resolveEffectiveScope(requestedScope, eventName, gitRef) {
  if (eventName !== "push" || requestedScope !== "auto") {
    return requestedScope;
  }
  return gitRef === "refs/heads/prerelease" ? "postmerge" : "full";
}

function changedPathsFromGit(baseSha, headSha) {
  if (!baseSha || !headSha || /^0+$/.test(baseSha)) {
    return [];
  }
  const output = execFileSync(
    "git",
    ["diff", "--name-only", "--diff-filter=ACMRD", baseSha, headSha],
    { encoding: "utf8" },
  );
  return output.split(/\r?\n/);
}

function writeOutputs(plan, paths) {
  const summary = `${plan.scope}: ${paths.length} changed path(s)`;
  const outputLines = [
    ...Object.entries(plan).map(([key, value]) => `${key}=${value}`),
    `summary=${summary}`,
  ];
  const outputPath = process.env.GITHUB_OUTPUT;
  if (outputPath) {
    appendFileSync(outputPath, `${outputLines.join("\n")}\n`, "utf8");
  }
  const summaryPath = process.env.GITHUB_STEP_SUMMARY;
  if (summaryPath) {
    appendFileSync(
      summaryPath,
      `## CI scope\n\n- Plan: \`${plan.scope}\`\n- Changed paths: ${paths.length}\n- Rust: ${plan.rust}\n- Node/admin: ${plan.node}\n- Portable: ${plan.portable}\n- Smoke: ${plan.smoke}\n- Post-merge: ${plan.postmerge}\n`,
      "utf8",
    );
  }
  console.log(JSON.stringify({ ...plan, changed_paths: paths }, null, 2));
}

function booleanEnvironment(name) {
  return process.env[name] === "true";
}

function runGate() {
  const plan = {
    rust: booleanEnvironment("AKRA_CI_RUST_REQUIRED"),
    node: booleanEnvironment("AKRA_CI_NODE_REQUIRED"),
    portable: booleanEnvironment("AKRA_CI_PORTABLE_REQUIRED"),
    smoke: booleanEnvironment("AKRA_CI_SMOKE_REQUIRED"),
  };
  const results = {
    scope: process.env.AKRA_CI_SCOPE_RESULT,
    rust_tests: process.env.AKRA_CI_RUST_TESTS_RESULT,
    rust_lint: process.env.AKRA_CI_RUST_LINT_RESULT,
    node_surfaces: process.env.AKRA_CI_NODE_RESULT,
    portable_native: process.env.AKRA_CI_PORTABLE_RESULT,
    smoke_check: process.env.AKRA_CI_SMOKE_RESULT,
  };
  const failures = evaluateGate(plan, results);
  if (failures.length > 0) {
    console.error("CI gate rejected required jobs:");
    for (const failure of failures) {
      console.error(`- ${failure.job}: ${failure.result || "missing"}`);
    }
    process.exitCode = 1;
    return;
  }
  console.log("CI gate accepted every job required by the selected scope.");
}

function runClassify() {
  const requestedScope = process.env.AKRA_CI_REQUESTED_SCOPE || "auto";
  const eventName = process.env.AKRA_CI_EVENT_NAME || "local";
  const paths = requestedScope === "auto"
    ? changedPathsFromGit(process.env.AKRA_CI_BASE_SHA, process.env.AKRA_CI_HEAD_SHA)
    : [];
  const effectiveScope = resolveEffectiveScope(
    requestedScope,
    eventName,
    process.env.AKRA_CI_REF,
  );
  const plan = classifyChangedPaths(paths, effectiveScope);
  writeOutputs(plan, paths);
}

function main() {
  const command = process.argv[2];
  if (command === "classify") {
    runClassify();
    return;
  }
  if (command === "gate") {
    runGate();
    return;
  }
  throw new Error("usage: node scripts/ci-scope.mjs <classify|gate>");
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  main();
}
