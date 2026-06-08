# Bug Report

This document records a directory-by-directory audit of usage pitfalls, bugs, and logic gaps. Each section is completed after inspecting one directory only.

## Progress

| Directory | Status | Completed at | Notes |
| --- | --- | --- | --- |
| `npm/` | Completed | 2026-06-08 | npm package runtime shim, platform mapping, package staging, and npm tests inspected. |
| `scripts/` | Completed | 2026-06-08 | release scripts, validation scripts, GitHub wrapper, and worktree cleanup inspected. |
| `schema/` | Completed | 2026-06-08 | app-server protocol schema snapshot and schema consumers inspected. |
| `templates/` | Completed | 2026-06-08 | admin Askama templates, template resources, and admin template consumers inspected. |

## `npm/`

### Scope

- Inspected files: `npm/package.json`, `npm/bin/akra.js`, `npm/lib/platform.js`, `npm/lib/runtime.js`, `npm/scripts/stage-npm-packages.mjs`, `npm/test/*.test.js`.
- Validation run: `npm test --prefix npm`.
- The audit did not inspect other top-level directories in this pass.

### Findings

#### NPM-001: signal exits from the native binary can be reported as success

- Severity: High
- Evidence: `npm/bin/akra.js:49` registers handlers for `SIGINT`, `SIGTERM`, and `SIGHUP`; `npm/bin/akra.js:53` then handles a child signal exit by calling `process.kill(process.pid, signal)`.
- Why this is a bug: because the parent process already has a listener for the same signal, the self-signal is caught by the forwarding handler instead of restoring Node's default signal termination behavior. A native binary that exits from `SIGTERM` can leave the wrapper alive until the event loop drains, and the wrapper can then exit with code `0`.
- Reproduction used during audit:

```bash
node --input-type=module - <<'NODE'
import { spawn } from "node:child_process";
const child = spawn(process.execPath, ["-e", "process.kill(process.pid, 'SIGTERM')"], { stdio: "ignore" });
const forwardSignal = (signal) => {
  if (child.killed) return;
  try { child.kill(signal); } catch {}
};
for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
  process.on(signal, () => forwardSignal(signal));
}
child.on("exit", (code, signal) => {
  console.log(JSON.stringify({ code, signal, killed: child.killed }));
  if (signal) {
    process.kill(process.pid, signal);
    setTimeout(() => console.log("still-alive"), 100);
    return;
  }
  process.exit(code ?? 1);
});
setTimeout(() => console.log("timeout"), 500);
NODE
echo exit=$?
```

- Observed output:

```text
{"code":null,"signal":"SIGTERM","killed":false}
still-alive
timeout
exit=0
```

- User impact: shell scripts, CI, service managers, and package-manager wrappers can treat an interrupted or terminated `akra` run as successful. This is most visible for cancellation, terminal shutdown, or supervisor-initiated termination.
- Suggested fix: before re-sending the signal to the parent, remove the installed signal listeners or translate the signal into the conventional exit code `128 + signalNumber`. Add a subprocess test that launches `npm/bin/akra.js` against a fixture binary that terminates via `SIGTERM`.

#### NPM-002: binary resolution failures surface as raw Node exceptions

- Severity: Medium
- Evidence: `npm/bin/akra.js:15` calls `resolveBinaryPath` at module top level. `npm/lib/runtime.js:47`, `npm/lib/runtime.js:69`, and `npm/lib/runtime.js:84` throw user-facing resolution errors, but the CLI wrapper does not catch them.
- Why this is a usage gap: unsupported platforms, missing optional dependencies, or damaged platform packages will print a Node stack trace instead of a compact `akra` diagnostic. The underlying message is useful, but the JS stack frames are implementation detail.
- User impact: install failures look like a JavaScript crash even though the actionable path is reinstalling the package or using a supported target.
- Suggested fix: wrap binary resolution in a small `main()` function, catch `Error`, print `akra: <message>`, and exit `1`. Keep stack traces behind a debug environment variable if needed.

#### NPM-003: supported platform policy is implicit and easy to misread

- Severity: Low
- Evidence: `npm/lib/platform.js:1` lists only three publish targets: Linux x64, macOS arm64, and Windows x64. `npm/lib/runtime.js:47` rejects unsupported platform/architecture pairs.
- Why this is a logic gap: the npm package metadata and README path do not make the platform support boundary as explicit as the runtime does. Common environments such as macOS x64 and Linux arm64 will fail only at execution time after installation.
- User impact: users can install the package successfully and only discover at first run that their platform is not supported.
- Suggested fix: document the supported platform matrix prominently in `npm/README.md` and the root README install section. If unsupported installs should fail earlier, consider platform-specific optional dependency guidance or a postinstall check that prints a concise warning without blocking supported package-manager behavior.

### Test Gaps

- `npm/test/runtime.test.js` covers binary path selection and unsupported platform mapping, but does not execute `npm/bin/akra.js` as a subprocess.
- No test asserts parent exit status when the native binary exits by signal.
- No test asserts the CLI text emitted for missing optional dependency, missing binary, or unsupported platform failures.

## `scripts/`

### Scope

- Inspected files: `scripts/capture_native_validation.ps1`, `scripts/capture_native_validation.sh`, `scripts/check_admin_graphic_visual.sh`, `scripts/check_native_pr.sh`, `scripts/check_tui_layering.sh`, `scripts/cleanup_merged_worktrees.sh`, `scripts/gh-akra.sh`, `scripts/package_native_release.sh`, `scripts/planning-tool.sh`, `scripts/summarize_native_validation.sh`, `scripts/validate_native_release_version.sh`, `scripts/verify_native_release.sh`.
- Validation run: `bash -n scripts/*.sh`.
- Validation run: `cargo test --test native_validation_scripts`.
- The audit did not inspect other top-level directories in this pass.

### Findings

#### SCRIPTS-001: several operator scripts fail silently when an option value is missing

- Severity: Medium
- Evidence: `scripts/package_native_release.sh:30`, `scripts/verify_native_release.sh:25`, `scripts/validate_native_release_version.sh:23`, and `scripts/gh-akra.sh:14` read `${2-}` and immediately `shift 2` without a local `require_value` check.
- Reproduction used during audit:

```bash
bash -c 'bash scripts/package_native_release.sh --target; printf "status=%s\n" "$?"' 2>&1
bash -c 'bash scripts/verify_native_release.sh --archive; printf "status=%s\n" "$?"' 2>&1
bash -c 'bash scripts/validate_native_release_version.sh --tag; printf "status=%s\n" "$?"' 2>&1
bash -c 'bash scripts/gh-akra.sh --github-login; printf "status=%s\n" "$?"' 2>&1
```

- Observed output: each command returned only `status=1` with no script-specific usage message.
- User impact: release and GitHub automation failures become hard to diagnose when the operator mistypes a command or a CI variable expands to an empty value.
- Suggested fix: share the `require_value` helper pattern already used by `scripts/capture_native_validation.sh:24` and `scripts/cleanup_merged_worktrees.sh:27`, then test each public option parser with missing values.

#### SCRIPTS-002: explicit worktree cleanup targets can be missing while the command still succeeds

- Severity: Medium
- Evidence: `scripts/cleanup_merged_worktrees.sh:225` enables targeted mode when `--branch` or `--path` is supplied, but `process_entry` only iterates existing `git worktree list --porcelain` entries. There is no final check that every explicit target matched a worktree.
- Reproduction used during audit:

```bash
bash scripts/cleanup_merged_worktrees.sh --branch definitely-not-a-real-branch
```

- Observed output:

```text
[skip] not in explicit target set :: ... (prerelease)
[skip] not in explicit target set :: ... (report/scripts-bug-audit)
[skip] not in explicit target set :: ... (test/native-coverage-planning-reset-shell)
dry-run complete: 0 eligible, 3 skipped
```

- User impact: an operator or automation job can believe a finished lane was cleaned up even when the branch/path argument was misspelled or already absent. In cleanup workflows, a false-success result leaves stale worktrees and remote branches behind.
- Suggested fix: track matched explicit branches/paths and exit non-zero when any requested target is not found, unless a new `--ignore-missing-targets` option is explicitly provided.

#### SCRIPTS-003: explicit cleanup bypasses merge ancestry checks before deleting branches

- Severity: High
- Evidence: `scripts/cleanup_merged_worktrees.sh:14` documents that explicit targets do not require ancestor detection, and `scripts/cleanup_merged_worktrees.sh:277` skips `branch_is_merged_into_base` when `explicitly_targeted=true`. `scripts/cleanup_merged_worktrees.sh:123` then uses `git branch -D`, and `scripts/cleanup_merged_worktrees.sh:129` deletes the remote branch when it exists.
- Why this is a logic gap: the script is named `cleanup_merged_worktrees`, but explicit cleanup can remove a clean, unmerged branch from both local and remote state. The current behavior is useful for disposable lanes, but it is a dangerous default for a command name that implies merged-only cleanup.
- User impact: a typo or premature cleanup command can delete reviewable work that has not actually reached `prerelease`, especially after a clean worktree has no local changes.
- Suggested fix: require ancestor detection for explicit targets by default, and move the current behavior behind an explicit flag such as `--allow-unmerged-explicit` or `--force-unmerged`.

#### SCRIPTS-004: native validation summary is non-failing by default even when all required rows are missing

- Severity: Low
- Evidence: `scripts/summarize_native_validation.sh:236` defaults to `docs/validation`, `scripts/summarize_native_validation.sh:239` sets `fail_on_incomplete=0`, and `scripts/summarize_native_validation.sh:474` exits non-zero only when `--fail-on-incomplete` is supplied.
- Reproduction used during audit:

```bash
bash -c 'bash scripts/summarize_native_validation.sh >/tmp/akra-summary.out; status=$?; printf "status=%s\n" "$status"; sed -n "1,18p" /tmp/akra-summary.out'
```

- Observed output started with:

```text
status=0
Native Validation Summary
records dir: docs/validation
check profile: terminal-baseline

Required Rows
- missing  macOS / Terminal.app / zsh / inline
...
```

- User impact: a human can run the documented summary command, see a successful shell status, and miss that validation coverage is incomplete. This matters because the terminal validation docs tell operators to summarize coverage before calling a matrix complete.
- Suggested fix: keep the current read-only summary mode if desired, but print a visible warning when required rows are incomplete. Use `--fail-on-incomplete` in any docs or scripts that describe a gate.

### Test Gaps

- Missing-value parser behavior is not covered for release scripts or `gh-akra.sh`.
- `cleanup_merged_worktrees.sh` does not appear to have tests for explicit target misspellings, unmerged explicit targets, or remote branch deletion behavior.
- `summarize_native_validation.sh` has Rust integration coverage for profile filtering, but no test that the default success status is safe or intentionally non-gating when required rows are missing.

## `schema/`

### Scope

- Inspected files: `schema/codex_app_server_protocol.v2.schemas.json`.
- Inspected consumers: `src/domain/startup_diagnostics.rs` and `src/adapter/outbound/app_server/protocol/contract_tests.rs`.
- Validation run: `jq empty schema/codex_app_server_protocol.v2.schemas.json`.
- Validation run: `cargo test schema_notification_vocabulary_requires_adapter_classification --lib`.
- The audit did not inspect other top-level directories in this pass.

### Findings

#### SCHEMA-001: numeric Rust integer formats are not enforceable by draft-07 validators

- Severity: High
- Evidence: the schema declares draft-07 at the root, but uses non-standard `format` values such as `uint`, `uint16`, `uint32`, `uint64`, `int32`, and `int64`. A query found 62 integer fields with these formats and no `maximum` constraint.
- Example evidence:

```bash
jq '.definitions.ThreadRollbackParams.properties.numTurns' schema/codex_app_server_protocol.v2.schemas.json
```

```json
{
  "description": "The number of turns to drop from the end of the thread. Must be >= 1\n\nThis only modifies the thread's history and does not revert local file changes that have been made by the agent. Clients are responsible for reverting these changes.",
  "type": "integer",
  "format": "uint32",
  "minimum": 0
}
```

- Why this is a bug: JSON Schema draft-07 treats `format` as annotation unless a validator opts into custom format logic. Even when format validation is enabled, `uint32` and `uint64` are not standard draft-07 formats. A standard validator can accept an integer that Rust deserialization later rejects for overflowing `u32`, `u64`, `i32`, or `i64`.
- The same example also contradicts itself: the description says the rollback count must be `>= 1`, while the schema sets `minimum` to `0`.
- User impact: external clients or generated tests that rely on this checked-in schema can produce payloads that validate but fail against the actual app-server/client contract.
- Suggested fix: add explicit `maximum` and `minimum` bounds for all fixed-width integer fields, or publish the custom format vocabulary and ship validator support with the schema snapshot.

#### SCHEMA-002: the checked-in protocol snapshot has no provenance, version, or stable identifier

- Severity: Medium
- Evidence: the root object has `$schema`, `title`, and `type`, but no `$id`, `description`, generator metadata, source commit, app-server version, or snapshot checksum.
- Reproduction used during audit:

```bash
jq '{schema: .["$schema"], id: .["$id"], title, description, generated: .["x-generated-from"], version: .version}' schema/codex_app_server_protocol.v2.schemas.json
```

- Observed output:

```json
{
  "schema": "http://json-schema.org/draft-07/schema#",
  "id": null,
  "title": "CodexAppServerProtocolV2",
  "description": null,
  "generated": null,
  "version": null
}
```

- User impact: when an app-server protocol mismatch appears, support can only compare the file path and current repository contents. The binary startup label is also based on path plus byte length, so two different snapshots with the same byte length would be indistinguishable in logs.
- Suggested fix: add stable provenance fields such as `$id`, `x-generated-from`, `x-source-revision`, `x-generated-at`, or `x-content-sha256`. Include the same checksum in the startup diagnostics label.

#### SCHEMA-003: the schema snapshot is minified, making protocol reviews hard to audit

- Severity: Low
- Evidence: `schema/codex_app_server_protocol.v2.schemas.json` is a single-line 173,208-byte JSON file. Pretty-printing it with `python3 -m json.tool` expands it to 12,773 lines and 445,000 bytes.
- User impact: PR review and line-level diagnosis are poor. A large protocol change appears as one changed line, and report references cannot point to useful local lines inside the schema.
- Suggested fix: store the schema in a stable pretty-printed form, or commit both the minified runtime snapshot and a formatted review copy. Add a check that fails when generated formatting is not stable.

### Test Gaps

- Current Rust coverage confirms the checked-in `ServerNotification.oneOf` vocabulary is classified, but it does not validate numeric bounds or schema compatibility with a standard JSON Schema validator.
- No test asserts schema provenance fields or startup diagnostics checksum stability.
- No formatter/generator check prevents the schema from remaining a one-line diff-hostile artifact.

## `templates/`

### Scope

- Inspected files: `templates/admin/*.html`, `templates/admin/partials/draft_status.html`, `templates/admin/resources/**`.
- Inspected consumers: `src/adapter/inbound/admin_api/views.rs`, `src/adapter/inbound/admin_api/pages.rs`, `src/adapter/inbound/admin_api/static_assets.rs`, `src/adapter/inbound/admin_api/tests.rs`, `assets/admin/game/akra-diorama.js`, and `scripts/check_admin_graphic_visual.sh`.
- Validation run: `cargo test admin_html_page_routes_render_live_templates --lib`.
- Validation run: `cargo test akra_graphic_dashboard_visual_contract_has_regression_guardrails --lib`.
- The audit did not inspect other top-level directories in this pass.

### Findings

#### TEMPLATES-001: admin pages depend on public CDNs at runtime

- Severity: Medium
- Evidence: `templates/admin/base.html:11` loads the graphic admin font from jsDelivr. The same base template loads PixiJS from cdnjs and HTMX from unpkg at `templates/admin/base.html:839-840`, then dynamically imports CodeMirror modules from esm.sh at `templates/admin/base.html:911-915`.
- Related evidence: local static assets are embedded through `src/adapter/inbound/admin_api/static_assets.rs:5-14` and `src/adapter/inbound/admin_api/static_assets.rs:48`, and the graphic dashboard loads local `/admin/assets/game/akra-diorama.js` at `templates/admin/akra_dashboard.html:1973`. The vendor browser dependencies are the main assets that still come from the network.
- Why this is a usage gap: the admin surface is documented and shaped as a local operator console, but every admin page makes third-party requests before the operator interacts with it. Offline, firewalled, or privacy-sensitive environments lose the custom font, HTMX validation flow, CodeMirror enhancement, and potentially the Pixi-based diorama.
- User impact: an operator can see a degraded or partially non-interactive admin UI even though the native binary and local embedded assets are present. The browser also leaks admin page access timing to third-party CDNs.
- Suggested fix: vendor the exact browser dependencies under `assets/admin/vendor/`, serve them through the existing admin static asset boundary, and load PixiJS only on the graphic dashboard pages that need it.

#### TEMPLATES-002: the graphic sidebar reports hard-coded success and version state

- Severity: Medium
- Evidence: `templates/admin/base.html:970-992` renders fixed achievement/status copy such as `Lv. 24`, `+120 today`, `AKRA v0.9.0-beta`, and the Korean equivalent of `all systems normal`. `src/adapter/inbound/admin_api/views.rs:18-40` shows the graphic templates already receive real `AkraAdminDashboardView` data, but the shared base sidebar does not use any dynamic dashboard health field.
- Related test evidence: `src/adapter/inbound/admin_api/tests.rs:1479` asserts that the base template keeps `AKRA v0.9.0-beta`, so current tests preserve at least part of the hard-coded sidebar state.
- Why this is a bug: operator chrome should not claim all systems are normal or display stale version/progress values independently of the actual planning/runtime/distributor state.
- User impact: during a blocked pool, failed worker, stale dashboard snapshot, or version mismatch, the always-green sidebar can contradict the main dashboard and make the operator trust the wrong status signal.
- Suggested fix: move sidebar status data into an explicit layout view model, or replace the hard-coded achievement area with neutral static branding that cannot be read as live health.

#### TEMPLATES-003: editor mutation URLs rebuild raw draft names instead of using an encoded route field

- Severity: Low
- Evidence: `templates/admin/editor.html:7`, `templates/admin/editor.html:26`, `templates/admin/editor.html:28`, and `templates/admin/editor.html:37` interpolate `{{ session.draft_name }}` directly into form, formaction, and HTMX URLs.
- Related evidence: `src/adapter/inbound/admin_api/pages.rs:787-811` has a dedicated `draft_editor_location` helper that percent-encodes `draft_name` for redirects, but `render_editor_page` passes only the raw `PlanningAdminSessionView` into the template at `src/adapter/inbound/admin_api/pages.rs:814-831`. `PlanningAdminSessionView` stores only `draft_name` and not a pre-encoded action URL at `src/application/service/planning/admin/surface.rs:211-216`.
- Why this is a logic gap: current generated draft names are safe timestamp slugs, but the load and mutation routes treat the path value as a general draft handle. If an imported, restored, or manually staged draft name ever contains characters that need URL path encoding, the create/load redirect can work while the rendered save/validate/promote buttons point at a different route.
- User impact: a valid editor session can become unsaveable from the browser, or a mutation can target the wrong draft handle after the page is rendered.
- Suggested fix: add encoded action paths to the editor view model or expose a small template-safe route helper, then assert rendered editor actions for draft names containing spaces, slashes, and percent-like characters.

### Test Gaps

- Existing admin template tests mostly assert static substrings. They do not fail when the base template adds new third-party CDN dependencies.
- The visual regression script verifies served local graphic assets, but it does not cover an offline/no-CDN browser run.
- No rendered-template test asserts that editor save, validate, promote, and HTMX URLs are percent-encoded consistently with `draft_editor_location`.
