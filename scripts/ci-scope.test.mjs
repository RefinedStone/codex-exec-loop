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

test("Jira source, collection, policy, and credential fences require native validation", () => {
  for (const path of [
    "src/adapter/outbound/jira/issue_source.rs",
    "src/adapter/outbound/provider_http/jira_source.rs",
    "src/application/service/issue_collection/jira.rs",
    "src/domain/issue_automation/policy/jira.rs",
    "src/application/service/provider_credentials.rs",
  ]) {
    const plan = classifyChangedPaths([path]);
    assert.equal(plan.rust, true, path);
    assert.equal(plan.portable, true, path);
    assert.equal(plan.docs_only, false, path);
  }
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

test("Windows trusted-temp contracts run once without selecting Windows-only tests on macOS", () => {
  const workflow = readFileSync(new URL("../.github/workflows/native-pr-checks.yml", import.meta.url), "utf8");
  const windows = readFileSync(new URL("./check_windows_portable.sh", import.meta.url), "utf8");
  const powershell = readFileSync(new URL("./check_native_pr.ps1", import.meta.url), "utf8");
  const macosStep = workflow.match(/node scripts\/run_native_rust_contracts\.mjs[\s\S]*?(?=\n      - name:)/)?.[0] || "";
  const powershellStep = powershell.match(/function Invoke-WindowsPortableChecks[\s\S]*?(?=\nPush-Location)/)?.[0] || "";
  const windowsSelectors = [
    ["Windows Bash", [...windows.matchAll(/^\s+'([^']+)'/gm)].map((match) => match[1])],
    ["PowerShell", [...powershellStep.matchAll(/^\s+"([^"\r\n]+)",?\r?$/gm)].map((match) => match[1])],
  ];
  const macosSelectors = [...macosStep.matchAll(/^\s+'([^'\r\n]+)'(?:\s*\\)?\r?$/gm)]
    .map((match) => match[1]);
  assert.ok(macosSelectors.length > 0, "the macOS contract list must be parsed before checking exclusions");
  const tempFilter = "adapter::outbound::github::automation::tests::windows_temp::";
  const identityFilter = "private_fs::shared_directory_tests::";
  for (const testPath of [tempFilter, identityFilter]) {
    for (const [platform, filters] of windowsSelectors) {
      assert.ok(filters.includes(testPath), `${platform} omits ${testPath}`);
      assert.equal(filters.filter((filter) => testPath.includes(filter)).length, 1,
        `${platform} must select ${testPath} through exactly one group`);
    }
    assert.equal(macosSelectors.filter((filter) => testPath.includes(filter)).length, 0,
      `macOS must not select the Windows-only contract ${testPath}`);
  }

  const cleanHost = powershellStep.match(/if \(\$IncludeCleanHostContracts\) \{\s*\$testFilters \+= @\(([\s\S]*?)\n        \)/)?.[0] || "";
  assert.ok(cleanHost.includes(`"${tempFilter}"`), "the real Git fixture belongs to the clean-host group");
  assert.ok(!cleanHost.includes(`"${identityFilter}"`), "in-memory identity checks must run in ordinary Windows checks too");
});

test("portable native selectors cover immutable evidence policy and the delivery gate", () => {
  const workflow = readFileSync(new URL("../.github/workflows/native-pr-checks.yml", import.meta.url), "utf8");
  const windows = readFileSync(new URL("./check_windows_portable.sh", import.meta.url), "utf8");
  const powershell = readFileSync(new URL("./check_native_pr.ps1", import.meta.url), "utf8");
  for (const filter of [
    "configuration::tests::evidence::",
    "adapter::inbound::cli::tests::config_evidence_mode_",
    "composition::production::delivery_evidence::tests::",
    "domain::planning::delivery_evidence::",
    "application::service::task_delivery_evidence::tests::",
    "composition::kernel_host::simulation::issue_decision::human_confirmation::autonomy::",
    "sqlite_issue_automation_store::autonomy::tests::",
    "application::service::issue_coordinator::tests::auto_delivery::",
    "application::service::issue_coordinator::tests::autonomy::",
    "adapter::inbound::kernel_protocol::issue_autonomy::",
    "sqlite_planning_authority_adapter::work_orders::tests::permits::delivery_review::auto_delivery::",
    "sqlite_planning_authority_adapter::work_orders::tests::permits::delivery_evidence::",
    "application::service::parallel_mode::tests::distributor::evidence_gate::",
    "application::service::parallel_mode::tests::distributor::work_permits::evidence_conditions::",
    "adapter::outbound::github::automation::tests::frozen_integration::",
    "sqlite_planning_authority_adapter::tests::task_evidence::",
  ]) {
    assert.ok(windows.includes(`'${filter}'`), `Windows Bash omits ${filter}`);
    assert.ok(powershell.includes(`"${filter}"`), `PowerShell omits ${filter}`);
    assert.ok(workflow.includes(`'${filter}'`), `macOS omits ${filter}`);
  }
});

test("portable native selectors retain restart recovery and cover Auto once", () => {
  const workflow = readFileSync(new URL("../.github/workflows/native-pr-checks.yml", import.meta.url), "utf8");
  const windows = readFileSync(new URL("./check_windows_portable.sh", import.meta.url), "utf8");
  const powershell = readFileSync(new URL("./check_native_pr.ps1", import.meta.url), "utf8");
  const macosStep = workflow.match(/node scripts\/run_native_rust_contracts\.mjs[\s\S]*?(?=\n      - name:)/)?.[0] || "";
  const powershellStep = powershell.match(/function Invoke-WindowsPortableChecks[\s\S]*?(?=\nPush-Location)/)?.[0] || "";
  const selectors = [
    ["Windows Bash", [...windows.matchAll(/^\s+'([^']+)'/gm)].map((match) => match[1])],
    ["PowerShell", [...powershellStep.matchAll(/^\s+"([^"\r\n]+::[^"\r\n]*)",?\r?$/gm)].map((match) => match[1])],
    ["macOS", [...macosStep.matchAll(/'([^'\r\n]+::[^'\r\n]*)'/g)].map((match) => match[1])],
  ];
  for (const testPath of [
    "private_sqlite::tests::",
    "sqlite_issue_automation_restart_registry::tests::",
    "application::service::issue_coordinator::tests::autonomy::restart_registry::",
    "application::service::issue_coordinator::tests::autonomy::auto::",
    "composition::issue_collection::restart_registry_tests::",
    "composition::repository_kernel::restart_tests::",
    "adapter::inbound::gateway::restart::tests::",
  ]) {
    for (const [platform, filters] of selectors) {
      assert.equal(filters.filter((filter) => testPath.includes(filter)).length, 1,
        `${platform} must cover ${testPath} through exactly one group`);
    }
  }
});

test("portable native selectors cover binding pause without rerunning inherited host and delivery groups", () => {
  const workflow = readFileSync(new URL("../.github/workflows/native-pr-checks.yml", import.meta.url), "utf8");
  const windows = readFileSync(new URL("./check_windows_portable.sh", import.meta.url), "utf8");
  const powershell = readFileSync(new URL("./check_native_pr.ps1", import.meta.url), "utf8");
  const macosStep = workflow.match(/node scripts\/run_native_rust_contracts\.mjs[\s\S]*?(?=\n      - name:)/)?.[0] || "";
  const powershellStep = powershell.match(/function Invoke-WindowsPortableChecks[\s\S]*?(?=\nPush-Location)/)?.[0] || "";
  const selectors = [
    ["Windows Bash", [...windows.matchAll(/^\s+'([^']+)'/gm)].map((match) => match[1])],
    ["PowerShell", [...powershellStep.matchAll(/^\s+"([^"\r\n]+::[^"\r\n]*)",?\r?$/gm)].map((match) => match[1])],
    ["macOS", [...macosStep.matchAll(/'([^'\r\n]+::[^'\r\n]*)'/g)].map((match) => match[1])],
  ];
  for (const filter of [
    "domain::issue_automation::arbiter::tests::arbiter_input_fingerprints_reject_missing_or_unbounded_provenance",
    "domain::issue_automation::arbiter::tests::arbiter_legacy_input_json_keeps_its_bytes_until_a_binding_generation_exists",
    "application::service::issue_binding_admission::tests::",
    "sqlite_issue_automation_store::binding_pause::tests::",
    "sqlite_issue_automation_store::admission::tests::binding_pause::",
    "sqlite_issue_automation_store::admission::tests::arbiter::binding_pause::",
    "sqlite_issue_automation_store::feedback::tests::binding_pause::",
    "application::service::issue_feedback::tests::binding_pause::",
    "application::service::issue_coordinator::tests::binding_pause::",
    "application::service::issue_intervention::tests::issue_intervention_reserves_internal_autonomy_and_binding_pause_command_ids",
    "sqlite_planning_authority_adapter::work_orders::tests::permits::action::",
    "adapter::outbound::issue_assessment_model::tests::denied_actual_dispatch_never_falls_back_to_an_unguarded_backend",
    "adapter::inbound::kernel_protocol::issue_binding_pause::",
    "composition::native_client_runtime::tests::binding_pause::",
    "composition::kernel_host::simulation::issue_decision::human_confirmation::binding_pause::",
    "kernel_events::tests::binding_pause_receipt_deduplicates_after_retention_and_rejects_changed_payload",
  ]) {
    for (const [platform, filters] of selectors) {
      assert.equal(filters.filter((value) => value === filter).length, 1,
        `${platform} must select ${filter} once`);
    }
  }
  // These Pause regressions already belong to required host/command/delivery groups.
  for (const testPath of [
    "application::service::task_verification::tests::binding_pause::",
    "composition::kernel_command_execution::tests::task_verification::binding_pause_requires_a_live_owner_without_creating_workspace_or_receipt",
    "application::service::parallel_mode::tests::distributor::evidence_gate::auto_delivery::binding_pause::",
  ]) {
    for (const [platform, filters] of selectors) {
      assert.equal(filters.filter((filter) => testPath.includes(filter)).length, 1,
        `${platform} must cover ${testPath} through exactly one existing group`);
    }
  }
  // The process fixtures use Unix executables; selecting them on Windows would run zero tests.
  for (const filter of [
    "adapter::outbound::app_server::tests::binding_admission::",
    "adapter::outbound::app_server::codex_thread::tests::binding_admission::",
  ]) {
    assert.equal(selectors[2][1].filter((value) => value === filter).length, 1);
    assert.ok(!selectors[0][1].includes(filter), "Windows Bash must not select a Unix-only process fixture");
    assert.ok(!selectors[1][1].includes(filter), "PowerShell must not select a Unix-only process fixture");
  }
  const linuxTests = workflow.match(/\n  rust_tests:[\s\S]*?(?=\n  rust_lint:)/)?.[0] || "";
  assert.match(linuxTests, /runs-on: ubuntu-latest/);
  assert.match(linuxTests, /run: (?:node scripts\/ci-performance\.mjs run rust-tests -- )?cargo test --workspace --locked(?: --timings)?\s*\n/);
});

test("portable native selectors cover Jira and inherited credential fences exactly once", () => {
  const workflow = readFileSync(new URL("../.github/workflows/native-pr-checks.yml", import.meta.url), "utf8");
  const windows = readFileSync(new URL("./check_windows_portable.sh", import.meta.url), "utf8");
  const powershell = readFileSync(new URL("./check_native_pr.ps1", import.meta.url), "utf8");
  const posix = readFileSync(new URL("./check_native_pr.sh", import.meta.url), "utf8");
  const macosStep = workflow.match(/node scripts\/run_native_rust_contracts\.mjs[\s\S]*?(?=\n      - name:)/)?.[0] || "";
  const powershellStep = powershell.match(/function Invoke-WindowsPortableChecks[\s\S]*?(?=\nPush-Location)/)?.[0] || "";
  const selectors = [
    ["Windows Bash", [...windows.matchAll(/^\s+'([^']+)'/gm)].map((match) => match[1])],
    ["PowerShell", [...powershellStep.matchAll(/^\s+"([^"\r\n]+)",?\r?$/gm)].map((match) => match[1])],
    ["macOS", [...macosStep.matchAll(/^\s+'([^'\r\n]+)'(?:\s*\\)?\r?$/gm)].map((match) => match[1])],
  ];
  for (const testPath of [
    "adapter::outbound::jira::issue_source::tests::",
    "adapter::outbound::provider_http::jira_source::tests::",
    "adapter::outbound::provider_http::tests::feedback::",
    "application::service::provider_credentials::tests::mutations::",
    "adapter::outbound::jira::issue_feedback::tests::",
    "domain::issue_automation::source::tests::jira::",
    "domain::issue_automation::triage::tests::jira::",
    "application::service::issue_automation_policy::tests::jira::",
    "application::service::issue_automation_policy::tests::jira_examples::",
    "application::service::issue_collection::tests::jira::",
    "adapter::outbound::db::sqlite_issue_source_claims::tests::",
    "application::service::issue_coordinator::tests::jira::",
    "application::service::issue_feedback::tests::jira::",
    "adapter::outbound::db::sqlite_issue_automation_store::feedback::tests::jira::",
    "adapter::inbound::kernel_protocol::issue_automation::triage::tests::",
    "application::port::outbound::planning_work_order_repository_port::tests::jira_",
    "application::service::parallel_mode::tests::pool::lease::malformed_jira_branch_keys_",
    "application::service::parallel_mode::distributor::delivery::github::tests::jira_pr_metadata_",
    "application::service::parallel_mode::tests::pool::lease::jira_branch_",
    "application::service::parallel_mode::tests::distributor::work_permits::link_reference::jira_work_link_",
  ]) {
    for (const [platform, filters] of selectors) {
      assert.equal(filters.filter((filter) => testPath.includes(filter)).length, 1,
        `${platform} must cover ${testPath} through exactly one group`);
    }
  }
  const cleanHost = powershellStep.match(/if \(\$IncludeCleanHostContracts\) \{\s*\$testFilters \+= @\(([\s\S]*?)\n        \)/)?.[1] || "";
  for (const filter of [
    "application::service::parallel_mode::tests::pool::lease::jira_branch_",
    "application::service::parallel_mode::tests::distributor::work_permits::link_reference::jira_work_link_",
  ]) {
    assert.ok(cleanHost.includes(`"${filter}"`), `the actual Git fixture needs the clean-host group: ${filter}`);
  }
  const linuxTests = workflow.match(/\n  rust_tests:[\s\S]*?(?=\n  rust_lint:)/)?.[0] || "";
  assert.match(linuxTests, /runs-on: ubuntu-latest/);
  assert.match(linuxTests, /run: (?:node scripts\/ci-performance\.mjs run rust-tests -- )?cargo test --workspace --locked(?: --timings)?\s*\n/);
  assert.match(posix, /run_step "Rust tests" cargo test --workspace --locked\s*\n/);
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

test("selective Rust replaces full test jobs only after exact row reconciliation succeeds", () => {
  const plan = { rust: true, portable: true, selectedRust: true };
  const results = { scope: "success", rust_lint: "success", rust_tests: "skipped", portable_native: "skipped", selected_rust_gate: "success" };
  assert.deepEqual(evaluateGate(plan, results), []);
  for (const result of [undefined, "skipped", "cancelled", "failure", "in_progress"]) {
    assert.deepEqual(evaluateGate(plan, { ...results, selected_rust_gate: result }), [{ job: "selected_rust_gate", result }]);
  }
  assert.deepEqual(evaluateGate({ ...plan, selectedRust: false }, results), [
    { job: "rust_tests", result: "skipped" }, { job: "portable_native", result: "skipped" },
  ]);
  assert.deepEqual(evaluateGate(plan, { ...results, rust_lint: "failure" }), [{ job: "rust_lint", result: "failure" }]);
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

test("Windows gates retain Propose and native validator contracts", () => {
  for (const script of ["check_windows_portable.sh", "check_native_pr.ps1"]) {
    const source = readFileSync(new URL("./" + script, import.meta.url), "utf8");
    for (const filter of [
      "application::service::parallel_mode::tests::issue_automation_flow::",
      "adapter::outbound::issue_code_validator::windows::",
    ]) {
      assert.equal(source.split(filter).length - 1, 1, script + " must include " + filter + " once");
    }
  }
});

test("workspace member source changes retain Rust and portable validation", () => {
  const plan = classifyChangedPaths(["crates/issue-core/src/issue_validation.rs"]);
  assert.equal(plan.rust, true);
  assert.equal(plan.portable, true);
  assert.equal(plan.docs_only, false);
});

