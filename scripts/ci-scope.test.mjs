import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  classifyChangedPaths,
  evaluateFastGate,
  evaluateGate,
  resolveEffectiveScope,
} from "./ci-scope.mjs";

test("documentation-only changes use the cheap docs plan", () => {
  assert.deepEqual(classifyChangedPaths(["docs/reference/development.md", "README.md"]), {
    scope: "docs",
    full: false,
    postmerge: false,
    rust: false,
    node: false,
    portable: false,
    smoke: false,
    docs_only: true,
  });
});

test("Rust changes keep tests, lint, and portable validation together", () => {
  const plan = classifyChangedPaths(["src/core/app.rs", "tests/startup.rs"]);
  assert.equal(plan.scope, "rust");
  assert.equal(plan.rust, true);
  assert.equal(plan.portable, true);
  assert.equal(plan.node, false);
});

test("admin-only changes run Node surfaces without unrelated Rust jobs", () => {
  const plan = classifyChangedPaths(["assets/admin/game/src/main.ts", "templates/admin/resources/main-sprite.png"]);
  assert.equal(plan.scope, "admin");
  assert.equal(plan.node, true);
  assert.equal(plan.rust, false);
  assert.equal(plan.portable, false);
});

test("compiled Askama admin templates require Rust checks and Node surfaces", () => {
  for (const template of [
    "templates/admin/akra_dashboard.html",
    "templates/admin/base.html",
    "templates/admin/partials/draft_status.html",
  ]) {
    const plan = classifyChangedPaths([template]);
    assert.equal(plan.scope, "mixed", template);
    assert.equal(plan.rust, true, template);
    assert.equal(plan.node, true, template);
    assert.equal(plan.portable, true, template);
    assert.equal(plan.docs_only, false, template);
  }
  // Static images under the same directory are not embedded Askama syntax.
  const image = classifyChangedPaths(["templates/admin/resources/main-sprite.png"]);
  assert.equal(image.rust, false);
  assert.equal(image.node, true);
});

test("desktop-only changes run the Node surfaces without Rust jobs", () => {
  const plan = classifyChangedPaths(["desktop/src/main/index.ts", "desktop/package-lock.json"]);
  assert.equal(plan.scope, "admin");
  assert.equal(plan.node, true);
  assert.equal(plan.rust, false);
  assert.equal(plan.portable, false);
});

test("mixed Rust and admin changes select both validation families", () => {
  const plan = classifyChangedPaths(["src/adapter/inbound/admin_api/mod.rs", "assets/admin/game/src/main.ts"]);
  assert.equal(plan.scope, "mixed");
  assert.equal(plan.rust, true);
  assert.equal(plan.node, true);
  assert.equal(plan.portable, true);
});

test("CI policy and unknown paths fail closed into the full plan", () => {
  assert.equal(classifyChangedPaths([".github/workflows/native-pr-checks.yml"]).full, true);
  assert.equal(classifyChangedPaths(["unexpected/new-surface.bin"]).full, true);
  assert.equal(classifyChangedPaths([]).full, true);
});

test("forced scopes are deterministic", () => {
  assert.equal(classifyChangedPaths([], "docs").docs_only, true);
  assert.equal(classifyChangedPaths([], "rust").portable, true);
  assert.equal(classifyChangedPaths([], "admin").node, true);
  assert.equal(classifyChangedPaths([], "smoke").smoke, true);
  const postmerge = classifyChangedPaths([], "postmerge");
  assert.equal(postmerge.postmerge, true);
  assert.equal(postmerge.rust, true);
  assert.equal(postmerge.node, true);
  assert.equal(postmerge.portable, true);
  assert.equal(classifyChangedPaths([], "full").full, true);
});

test("protected prerelease pushes use postmerge while main pushes remain full", () => {
  assert.equal(
    resolveEffectiveScope("auto", "push", "refs/heads/prerelease"),
    "postmerge",
  );
  assert.equal(resolveEffectiveScope("auto", "push", "refs/heads/main"), "full");
  assert.equal(resolveEffectiveScope("docs", "workflow_dispatch", "refs/heads/prerelease"), "docs");
});

test("the workflow publishes one stable Post-Merge Gate for postmerge plans", () => {
  const workflow = readFileSync(
    new URL("../.github/workflows/native-pr-checks.yml", import.meta.url),
    "utf8",
  );
  assert.match(workflow, /post_merge_gate:\s*\n\s*name: Post-Merge Gate/);
  assert.match(workflow, /needs\.scope\.outputs\.postmerge == 'true'/);
  assert.match(workflow, /github\.event_name == 'push' && github\.sha/);
});

test("stacked codex pull requests use their own base and the ordinary full CI gate", () => {
  const workflow = readFileSync(new URL("../.github/workflows/native-pr-checks.yml", import.meta.url), "utf8");
  const pullRequest = workflow.match(/\n  pull_request:([\s\S]*?)\n  push:/)?.[1] || "";
  const push = workflow.match(/\n  push:([\s\S]*?)\n  workflow_dispatch:/)?.[1] || "";
  assert.match(pullRequest, /- 'codex\/\*\*'/);
  assert.doesNotMatch(push, /codex/);
  assert.match(workflow, /AKRA_CI_BASE_SHA:.*github\.event\.pull_request\.base\.sha/);
  assert.match(workflow, /AKRA_CI_HEAD_SHA:.*github\.event\.pull_request\.head\.sha/);
  assert.equal(resolveEffectiveScope("auto", "pull_request", "refs/pull/2665/merge"), "auto");
  const plan = classifyChangedPaths([".github/workflows/native-pr-checks.yml"]);
  assert.equal(plan.full, true);
  assert.deepEqual(evaluateGate(plan, { scope: "success", rust_tests: "success", rust_lint: "success",
    node_surfaces: "failure", desktop_e2e: "success", portable_native: "success" }), [{ job: "node_surfaces", result: "failure" }]);
});

test("portable native selectors cover host verification through the actual Windows Bash path", () => {
  const workflow = readFileSync(new URL("../.github/workflows/native-pr-checks.yml", import.meta.url), "utf8");
  const windows = readFileSync(new URL("./check_windows_portable.sh", import.meta.url), "utf8");
  const powershell = readFileSync(new URL("./check_native_pr.ps1", import.meta.url), "utf8");
  const windowsStep = workflow.match(/- name: Run Windows portable validation[\s\S]*?(?=\n      - name:)/)?.[0] || "";
  assert.match(windowsStep, /shell: bash/);
  assert.match(windowsStep, /run: node scripts\/ci-performance\.mjs run windows-portable -- bash scripts\/check_windows_portable\.sh/);
  for (const filter of [
    "task_evidence_artifact::tests::",
    "sqlite_planning_authority_adapter::tests::task_evidence::",
    "parallel_worker_commit::tree_snapshot::tests::",
    "parallel_mode::tests::pool::lease::task_evidence::",
    "worker_evidence::",
    "domain::planning::verification::",
    "domain::kernel_event::task_verification::",
    "adapter::inbound::kernel_protocol::task_verification::",
    "kernel_events::tests::task_evidence_receipts_survive_ack_loss_retention_and_reject_changed_payload",
    "task_evidence_outbox::tests::host_verification_",
    "application::service::task_verification::tests::",
    "application::service::task_evidence::tests::verification::",
    "composition::native_client_runtime::task_verification_tests::",
    "composition::kernel_command_execution::tests::task_verification::",
    "trusted_executable::fingerprint::tests::",
    "subprocess::controlled::tests::",
    "adapter::outbound::git::task_verification::tests::",
    "parallel_mode::tests::pool::lease::task_verification::",
  ]) {
    assert.ok(windows.includes(`'${filter}'`), `Windows Bash omits ${filter}`);
    assert.ok(powershell.includes(`"${filter}"`), `PowerShell omits ${filter}`);
    assert.ok(workflow.includes(`'${filter}'`), `macOS omits ${filter}`);
  }
});

test("the workflow publishes a stable Fast Gate without weakening CI Gate", () => {
  const workflow = readFileSync(
    new URL("../.github/workflows/native-pr-checks.yml", import.meta.url),
    "utf8",
  );
  const fastGate = workflow.match(/\n  fast_gate:[\s\S]*?\n  ci_gate:/)?.[0] || "";
  assert.match(fastGate, /name: Fast Gate/);
  assert.match(fastGate, /node scripts\/ci-scope\.mjs fast-gate/);
  assert.doesNotMatch(fastGate, /\n\s+- rust_tests/);
  assert.doesNotMatch(fastGate, /\n\s+- portable_native/);
  assert.match(workflow, /\n  ci_gate:\s*\n\s*name: CI Gate/);
});

test("postmerge classification writes machine outputs and a readable summary", () => {
  const fixtureRoot = mkdtempSync(join(tmpdir(), "akra-ci-scope-postmerge-"));
  const outputPath = join(fixtureRoot, "output.txt");
  const summaryPath = join(fixtureRoot, "summary.md");
  try {
    const result = spawnSync(
      process.execPath,
      [fileURLToPath(new URL("./ci-scope.mjs", import.meta.url)), "classify"],
      {
        encoding: "utf8",
        env: {
          ...process.env,
          AKRA_CI_EVENT_NAME: "push",
          AKRA_CI_REF: "refs/heads/prerelease",
          AKRA_CI_REQUESTED_SCOPE: "auto",
          GITHUB_OUTPUT: outputPath,
          GITHUB_STEP_SUMMARY: summaryPath,
        },
      },
    );
    assert.equal(result.status, 0, result.stderr);
    assert.match(readFileSync(outputPath, "utf8"), /^postmerge=true$/m);
    assert.match(readFileSync(summaryPath, "utf8"), /Post-merge: true/);
  } finally {
    rmSync(fixtureRoot, { recursive: true, force: true });
  }
});

test("the gate ignores skipped optional jobs and requires selected jobs", () => {
  const docsFailures = evaluateGate(
    { rust: false, node: false, portable: false, smoke: false },
    { scope: "success" },
  );
  assert.deepEqual(docsFailures, []);

  const rustFailures = evaluateGate(
    { rust: true, node: false, portable: true, smoke: false },
    {
      scope: "success",
      rust_tests: "success",
      rust_lint: "failure",
      portable_native: "skipped",
    },
  );
  assert.deepEqual(rustFailures, [
    { job: "rust_lint", result: "failure" },
    { job: "portable_native", result: "skipped" },
  ]);
});

test("the fast gate waits for policy checks but leaves long jobs post-admission", () => {
  const failures = evaluateFastGate(
    { rust: true, node: true, portable: true, smoke: false },
    {
      scope: "success",
      rust_tests: "in_progress",
      rust_lint: "success",
      node_surfaces: "success",
      desktop_e2e: "success",
      portable_native: "in_progress",
      smoke_check: "skipped",
    },
  );
  assert.deepEqual(failures, []);

  assert.deepEqual(
    evaluateFastGate(
      { rust: true, node: false, portable: true, smoke: false },
      { scope: "success", rust_lint: "failure" },
    ),
    [{ job: "rust_lint", result: "failure" }],
  );
});

test("executable client validation tools under docs select the Node gate", () => {
  for (const file of ["proxy_ws_check.mjs", "multi_client_check.py", "client_count_validation.test.mjs"]) {
    const path = "docs/plan/17-akra-desktop/scripts/" + file;
    const plan = classifyChangedPaths([path, "docs/plan/17-akra-desktop/README.md"]);
    assert.equal(plan.scope, "admin", path);
    assert.equal(plan.node, true, path);
    assert.equal(plan.docs_only, false, path);
    assert.equal(plan.rust, false, path);
  }
  assert.equal(classifyChangedPaths(["docs/plan/17-akra-desktop/README.md"]).docs_only, true);
});


test("both gates require all selected Desktop shards to succeed", () => {
  const plan = { rust: false, node: true, portable: false, smoke: false };
  for (const evaluate of [evaluateGate, evaluateFastGate]) {
    const complete = { scope: "success", node_surfaces: "success", desktop_e2e: "success" };
    assert.deepEqual(evaluate(plan, complete), []);
    for (const result of ["failure", "cancelled", "skipped", undefined]) {
      assert.deepEqual(evaluate(plan, { ...complete, desktop_e2e: result }),
        [{ job: "desktop_e2e", result }]);
    }
    assert.deepEqual(evaluate({ ...plan, node: false }, { scope: "success", desktop_e2e: "skipped" }), []);
  }
});

test("the CLI gates read the Desktop shard result and reject a missing result", () => {
  for (const command of ["gate", "fast-gate"]) {
    const env = { ...process.env, AKRA_CI_NODE_REQUIRED: "true", AKRA_CI_NODE_RESULT: "success",
      AKRA_CI_SCOPE_RESULT: "success", AKRA_CI_RUST_REQUIRED: "false",
      AKRA_CI_PORTABLE_REQUIRED: "false", AKRA_CI_SMOKE_REQUIRED: "false" };
    delete env.GITHUB_OUTPUT;
    delete env.GITHUB_STEP_SUMMARY;
    for (const desktopResult of ["success", undefined]) {
      if (desktopResult) env.AKRA_CI_DESKTOP_RESULT = desktopResult;
      else delete env.AKRA_CI_DESKTOP_RESULT;
      const result = spawnSync(process.execPath,
        [fileURLToPath(new URL("./ci-scope.mjs", import.meta.url)), command], { env, encoding: "utf8" });
      assert.equal(result.status, desktopResult ? 0 : 1, result.stderr);
      if (!desktopResult) assert.match(result.stderr, /desktop_e2e: missing/);
    }
  }
});
