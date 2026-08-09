import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  classifyChangedPaths,
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
  const plan = classifyChangedPaths(["assets/admin/game/src/main.ts", "templates/admin/index.html"]);
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
