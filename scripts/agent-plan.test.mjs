import assert from "node:assert/strict";
import test from "node:test";

import { commandsForPlan } from "./agent-plan.mjs";

test("docs plans remain a cheap textual check", () => {
  assert.deepEqual(
    commandsForPlan({ docs_only: true }, "win32"),
    ["git diff --check"],
  );
});

test("Windows plans select the native proportional entrypoint", () => {
  assert.deepEqual(
    commandsForPlan({ docs_only: false, full: false, rust: true, node: false }, "win32"),
    ["git diff --check", "powershell -File scripts/check_native_pr.ps1 -Mode Rust"],
  );
  assert.deepEqual(
    commandsForPlan({ docs_only: false, full: false, rust: false, node: true }, "win32"),
    ["git diff --check", "powershell -File scripts/check_native_pr.ps1 -Mode Admin"],
  );
});

test("mixed POSIX plans use the complete native gate once", () => {
  assert.deepEqual(
    commandsForPlan({ docs_only: false, full: false, rust: true, node: true }, "linux"),
    ["git diff --check", "bash scripts/check_native_pr.sh"],
  );
});
