import assert from "node:assert/strict";
import test from "node:test";

import { classifyChangedPaths, evaluateGate } from "./ci-scope.mjs";

test("documentation-only changes use the cheap docs plan", () => {
  assert.deepEqual(classifyChangedPaths(["docs/reference/development.md", "README.md"]), {
    scope: "docs",
    full: false,
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
  assert.equal(classifyChangedPaths([], "full").full, true);
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
