# Bug Report

This document records a directory-by-directory audit of usage pitfalls, bugs, and logic gaps. Each section is completed after inspecting one directory only.

## Progress

| Directory | Status | Completed at | Notes |
| --- | --- | --- | --- |
| `npm/` | Completed | 2026-06-08 | npm package runtime shim, platform mapping, package staging, and npm tests inspected. |
| `scripts/` | Completed | 2026-06-08 | release scripts, validation scripts, GitHub wrapper, and worktree cleanup inspected. |
| `schema/` | Completed | 2026-06-08 | app-server protocol schema snapshot and schema consumers inspected. |
| `templates/` | Completed | 2026-06-08 | admin Askama templates, template resources, and admin template consumers inspected. |
| `assets/` | Completed | 2026-06-08 | admin game assets, embedded graphics, and app-server skill assets inspected. |
| `.github/` | Completed | 2026-06-08 | PR template and GitHub Actions workflows inspected. |
| `examples/` | Completed | 2026-06-08 | bundled prompt example and native release references inspected. |
| `tests/` | Completed | 2026-06-08 | integration tests for architecture boundaries, binary entrypoints, and native validation scripts inspected. |

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

## `assets/`

### Scope

- Inspected files: `assets/admin/game/*`, `assets/admin/game/src/akra-diorama.ts`, `assets/admin/game/scripts/promote-build.mjs`, `assets/admin/graphics/*.png`, `assets/app-server/skills/akra-planning-queue-mutation/SKILL.md`.
- Inspected consumers: `src/adapter/inbound/admin_api/static_assets.rs`, `src/adapter/inbound/admin_api/tests.rs`, `src/adapter/outbound/app_server/planning_worker_skill.rs`, `src/application/service/planning/task_mutation/commands.rs`, `scripts/check_admin_graphic_visual.sh`, and `npm/scripts/stage-npm-packages.mjs`.
- Validation run: `npm --prefix assets/admin/game ci`.
- Validation run: `npm --prefix assets/admin/game run check`.
- Validation run: `npm --prefix assets/admin/game run build`.
- Validation run: `cargo test queue_mutation_skill_documents_evaluator_contract --lib`.
- Validation run: `cargo test admin_graphic_asset_routes_serve_known_assets_and_reject_unknown_names --lib`.
- Validation run: `cargo test admin_game_asset_route_serves_diorama_bundle_and_rejects_unknown_names --lib`.
- The audit did not inspect other top-level directories in this pass.

### Findings

#### ASSETS-001: the bundled planning skill shows an invalid fallback command shape

- Severity: High
- Evidence: `assets/app-server/skills/akra-planning-queue-mutation/SKILL.md:42-46` first requires exactly one JSON object containing `planning_task_commands`, but `assets/app-server/skills/akra-planning-queue-mutation/SKILL.md:48-53` then shows fallback commands as standalone top-level objects such as `{"op":"create_task","title":"..."}`.
- Parser evidence: `src/application/service/planning/task_mutation/commands.rs:63-77` only deserializes documents shaped as `{"planning_task_commands":{"version":1,"commands":[{"op":"create_task", ...}]}}`. It only enters command extraction when a candidate JSON object has a `planning_task_commands` key at `src/application/service/planning/task_mutation/commands.rs:127`.
- Test evidence: `src/application/service/planning/task_mutation/tests.rs:620-643` confirms wrapped commands without the `op` tag are invalid and the later valid shape must include the `planning_task_commands.commands[]` envelope.
- Why this is a bug: the skill is the runtime asset attached before hidden planning worker prompts. If the planning tool is unavailable and the worker follows the fallback example literally, the host will not apply the intended task mutation.
- User impact: post-turn planning can silently fail to create or update the next task exactly when it is relying on the fallback path.
- Suggested fix: replace the fallback example with a full valid envelope, for example `{"planning_task_commands":{"version":1,"commands":[{"op":"create_task","title":"..."}]}}`. Add a test that extracts every JSON example in the skill and verifies fallback examples parse as commands.

#### ASSETS-002: large unreferenced admin graphics remain in the tracked asset set

- Severity: Medium
- Evidence: a filename reference scan across `assets`, `src`, `templates`, `scripts`, `docs`, `README.md`, and `tests` found zero references to these tracked files:

```text
assets/admin/graphics/isometric_agent_character_1.png
assets/admin/graphics/isometric_office_props_1.png
assets/admin/graphics/isometric_pixel_sprites_1.png
assets/admin/graphics/isometric_server_rack_1.png
assets/admin/graphics/isometric_tiles_partitions_1.png
assets/admin/graphics/isometric_workstation_1.png
assets/admin/graphics/sprite_fd_control_desk.png
```

- Size evidence: those seven files total 8,410,147 bytes, or about 8.02 MiB. The tracked `assets/admin/graphics` directory is about 23.17 MiB, so unreferenced graphics account for roughly one third of the tracked admin graphics payload.
- Why this is a usage gap: there is no manifest that marks these files as source art, reference art, or runtime assets. A maintainer cannot tell whether removing them would break the product, while clone and review size continue to grow.
- User impact: repository checkout, archive review, and asset audits are heavier than necessary, and unused artwork can be mistaken for supported runtime assets.
- Suggested fix: either remove unused files, move source/reference art under a clearly documented non-runtime directory, or add an asset manifest that records purpose, owner, and runtime inclusion status.

#### ASSETS-003: admin game asset names are duplicated without a single manifest

- Severity: Low
- Evidence: `assets/admin/game/src/akra-diorama.ts:271-283` hard-codes the browser asset URL map, while `src/adapter/inbound/admin_api/static_assets.rs:53-75` independently maps route names to embedded PNG bytes. The Vite build can type-check the string map, but it cannot prove that every URL is served by the Rust route.
- Related test evidence: `src/adapter/inbound/admin_api/tests.rs:854-920` checks that a fixed Rust-side list of known assets is servable, and `scripts/check_admin_graphic_visual.sh:153-160` fetches a narrower visual subset. These checks do not derive the expected set from the TypeScript asset map itself.
- Why this is a logic gap: adding, renaming, or deleting a sprite requires updating at least the TypeScript asset map, the Rust static asset match, and tests/scripts by hand. A drift can compile cleanly and only show up as missing sprites in the browser.
- User impact: the graphic admin dashboard can degrade visually after an otherwise successful Rust and TypeScript build.
- Suggested fix: introduce a small checked-in manifest for admin graphic assets and generate or validate both the TypeScript URL map and Rust static route table from it. At minimum, add a test that parses `akra-diorama.ts` asset filenames and verifies every referenced file is served by the admin asset route.

### Test Gaps

- The current skill asset test checks for broad contract phrases but does not parse the JSON examples in `SKILL.md`.
- Admin graphic route tests verify static route availability, but not that the TypeScript diorama asset map and Rust route table are exactly the same set.
- No CI guard flags tracked admin graphics that have no runtime reference or manifest entry.

## `.github/`

### Scope

- Inspected files: `.github/PULL_REQUEST_TEMPLATE.md`, `.github/workflows/native-pr-checks.yml`, `.github/workflows/release-native-assets.yml`.
- Inspected delegated scripts: `scripts/check_native_pr.sh`, `scripts/validate_native_release_version.sh`, and `npm/scripts/stage-npm-packages.mjs`.
- Validation run: `bash scripts/validate_native_release_version.sh --tag v1.3.3`.
- Validation run: `bash scripts/validate_native_release_version.sh --tag 1.3.3`.
- The audit did not inspect other top-level directories in this pass.

### Findings

#### GITHUB-001: required PR checks do not exercise npm or admin game packaging

- Severity: High
- Evidence: `.github/workflows/native-pr-checks.yml:41-43` delegates the whole PR gate to `bash scripts/check_native_pr.sh`. That script currently runs only TUI layering, Rust formatting, Rust tests, and Rust clippy at `scripts/check_native_pr.sh:20-23`.
- Missing coverage evidence: the workflow does not run `npm test --prefix npm`, `npm --prefix assets/admin/game run check`, `npm --prefix assets/admin/game run build`, or npm package staging tests, even though the repository ships npm packages and checked-in admin game assets.
- Why this is a bug: changes to `npm/`, `assets/admin/game/`, or release staging can pass the required Native PR Checks while breaking install/runtime packaging surfaces.
- User impact: a PR can be green on GitHub and still ship a broken npm wrapper, stale admin game bundle, or invalid platform package staging.
- Suggested fix: extend `scripts/check_native_pr.sh` or the workflow to run the npm package tests and admin game check/build. If runtime cost is a concern, use path-aware jobs but keep them required for matching paths.

#### GITHUB-002: release tags can be duplicated with and without a `v` prefix

- Severity: Medium
- Evidence: `.github/workflows/release-native-assets.yml:4-6` triggers on every pushed tag. `scripts/validate_native_release_version.sh:53` strips one leading `v`, and `npm/scripts/stage-npm-packages.mjs:47-49` does the same for npm package versions.
- Reproduction used during audit:

```bash
bash scripts/validate_native_release_version.sh --tag v1.3.3
bash scripts/validate_native_release_version.sh --tag 1.3.3
```

- Observed output: both commands accepted the same Cargo version and printed `release_version=1.3.3`.
- Why this is a logic gap: pushing both `v1.3.3` and `1.3.3` can create two GitHub Release records for the same package version. npm staging normalizes both to `1.3.3`, so the second tag can only skip or collide with already-published npm versions.
- User impact: release history can show duplicate native releases for one version while npm has only one immutable package version.
- Suggested fix: enforce a single tag convention in the workflow trigger and validator, for example `tags: ["v*"]` plus a validator that rejects unprefixed tags.

#### GITHUB-003: a tag workflow can succeed while npm publication is skipped

- Severity: Medium
- Evidence: `publish-release` creates or updates the GitHub Release before npm publication at `.github/workflows/release-native-assets.yml:110-149`. The later `publish-npm` job treats a missing `NPM_TOKEN` as success by writing `configured=false` and exiting `0` at `.github/workflows/release-native-assets.yml:175-178`, which skips every npm staging and publish step.
- Why this is a usage gap: the workflow name and release notes describe an automated native/npm release, but a repository without `NPM_TOKEN` still gets a green tag workflow and GitHub Release assets only.
- User impact: operators can believe a tag completed the full distribution flow while `@refinedstone/akra` was never published for that version.
- Suggested fix: make npm publication an explicit workflow mode. If npm is required for release tags, fail when `NPM_TOKEN` is missing. If npm is optional, rename the job/release notes and emit a visible GitHub step summary that says npm was skipped.

#### GITHUB-004: the release workflow has mixed line endings

- Severity: Low
- Evidence: `file .github/workflows/release-native-assets.yml` reports `ASCII text, with CRLF, LF line terminators`. A CRLF scan found CRLF lines in the same workflow while other chunks are LF-only.
- Why this is a maintenance gap: editing the workflow on different platforms can create noisy diffs unrelated to release logic, and local tooling can rewrite large portions of the file unexpectedly.
- User impact: review signal drops on future release workflow changes, especially when a small YAML edit appears as broad line-ending churn.
- Suggested fix: normalize `.github/workflows/release-native-assets.yml` to LF and add a `.gitattributes` rule for workflow files.

### Test Gaps

- `actionlint` is not part of the local or GitHub PR gate, so workflow expression and action-input mistakes are not caught before merge.
- The PR gate does not cover npm package tests, admin game build freshness, or npm staging.
- No workflow test or script asserts a single accepted release tag convention.

## `examples/`

### Scope

- Inspected files: `examples/initial_prompt.txt`.
- Inspected release references: `scripts/package_native_release.sh`, `README.md`, and `docs/plan/13-native-packaging-and-operator-runbook.md`.
- Validation run: `rg -n "initial_prompt" -S --glob "!docs/bug-report.md" .`.
- Validation run: `git ls-files examples`.
- The audit did not inspect other top-level directories in this pass.

### Findings

#### EXAMPLES-001: bundled example prompt is not discoverable from docs or code

- Severity: Medium
- Evidence: `examples/initial_prompt.txt` is the only tracked file under `examples/`. `scripts/package_native_release.sh:221-222` copies the tracked `examples/` directory into every native release bundle, while `README.md:108-110` and `docs/plan/13-native-packaging-and-operator-runbook.md:38-46` only list `examples/` as a bundle content item.
- Reference evidence: `rg -n "initial_prompt" -S --glob "!docs/bug-report.md" .` returns no references, so no code path, README section, or runbook step names the file or explains how to use it.
- Why this is a usage gap: users receive a release artifact that looks intentional, but the project never explains whether the prompt should be pasted into `akra`, used as a planning seed, edited before use, or treated as a smoke-test sample.
- User impact: the example can silently rot, and operators looking for a supported starter prompt have to infer the intended workflow from a three-line text file.
- Suggested fix: add `examples/README.md` or a release runbook section that names every bundled example, its target workflow, and the exact command or UI action that consumes it.

#### EXAMPLES-002: the prompt asks for autonomous work without the repository delivery guardrails

- Severity: Medium
- Evidence: `examples/initial_prompt.txt:1-3` asks the agent to choose the most important task in the repository and proceed, then summarize the result. It does not mention worktree isolation, branch base, commit/push/PR delivery, or cleanup. By contrast, `AGENTS.md:1-4` requires a worktree for every change and `commit -> push -> PR(prerelease) -> rebase merge(prerelease)` for meaningful work, and `docs/plan/04-worktree-branch-rules.md:7-10` requires `origin/prerelease`, sibling worktrees, and one reviewable slice per branch.
- Why this is a logic gap: the bundled starter prompt can trigger broad autonomous changes, but it omits the safety and delivery rules that make those changes reviewable in this repository.
- User impact: an operator following only the example can get local unreviewed edits, branches based from the wrong ref, or completed work that never reaches the `prerelease` integration flow.
- Suggested fix: rewrite the example as a scoped operational prompt that explicitly requires reading `AGENTS.md`, creating a worktree from `origin/prerelease`, opening a PR to `prerelease`, and cleaning up after merge. If the file is meant to be generic, rename it and add warnings that repository-specific rules still apply.

#### EXAMPLES-003: the only bundled example is Korean-only in an English release surface

- Severity: Low
- Evidence: `examples/initial_prompt.txt:1-3` is Korean-only, while the release-facing bundle documentation around it is English in `README.md:108-110` and `docs/plan/13-native-packaging-and-operator-runbook.md:38-46`.
- Why this is a usability gap: non-Korean operators can see that examples are shipped, but the only actual example cannot be understood without translation and has no language marker in the filename.
- User impact: the example directory is less useful for OSS users and release consumers who otherwise interact with the English README/runbook.
- Suggested fix: add an English equivalent, use language-qualified filenames such as `initial_prompt.ko.txt` and `initial_prompt.en.txt`, and document both in `examples/README.md`.

### Test Gaps

- No release packaging check verifies that bundled examples are documented by name.
- No prompt contract test verifies that operational examples include the repository's required worktree and PR delivery constraints.
- No localization or filename convention check prevents shipping a single-language example without a language marker.

## `tests/`

### Scope

- Inspected files: `tests/architecture_boundaries.rs`, `tests/binary_entrypoints.rs`, and `tests/native_validation_scripts.rs`.
- Inspected related CI entrypoint: `scripts/check_native_pr.sh`.
- Validation run: `. "$HOME/.cargo/env" && cargo test --test architecture_boundaries --test binary_entrypoints --test native_validation_scripts`.
- Reference check: `rg -n "npm/bin|akra\\.js|SIGTERM|SIGHUP|resolveBinaryPath|npm" tests -S`.
- The audit did not inspect other top-level directories in this pass.

### Findings

#### TESTS-001: integration tests do not exercise the shipped npm wrapper or signal contract

- Severity: High
- Evidence: `tests/binary_entrypoints.rs:7-10` binds only Cargo-built Rust binaries through `CARGO_BIN_EXE_codex-exec-loop-native`, `CARGO_BIN_EXE_akra`, `CARGO_BIN_EXE_akra-admin`, and `CARGO_BIN_EXE_akra-telegram`. The tests then check help text, unsupported command errors, admin ephemeral port startup, interrupt shutdown, and telegram bootstrap errors at `tests/binary_entrypoints.rs:12-86`.
- Missing coverage evidence: `rg -n "npm/bin|akra\\.js|SIGTERM|SIGHUP|resolveBinaryPath|npm" tests -S` returns no matches, so the integration suite never launches `npm/bin/akra.js`, never checks platform package resolution, and never verifies the wrapper's signal exit behavior.
- Why this is a bug: the repository ships both native binaries and an npm entrypoint, but the root integration test suite only proves the Cargo binaries. Wrapper regressions can remain invisible to `cargo test` and to the current native PR gate.
- User impact: installed npm users can see different exit codes, stack traces, or missing-binary diagnostics than developers see from the Rust binaries.
- Suggested fix: add an integration or npm-driven test that launches `node npm/bin/akra.js` against a fixture native binary, covers missing optional package diagnostics, and asserts signal exits map to non-success process outcomes.

#### TESTS-002: release version tests do not pin a single tag convention

- Severity: Medium
- Evidence: `tests/native_validation_scripts.rs:70-89` accepts only a matching `v1.3.4` happy path, and `tests/native_validation_scripts.rs:91-106` rejects only a mismatched `v1.3.4` tag. There is no test for an unprefixed matching tag such as `1.3.4`.
- Related script evidence: `scripts/validate_native_release_version.sh:10-12` documents both `--tag v1.3.4` and `--tag 1.3.4 --manifest Cargo.toml`, while `scripts/validate_native_release_version.sh:53` strips one leading `v`.
- Why this is a logic gap: the tests prove version equality, not release identity. They do not force the project to choose whether tags must be `v<version>` or bare `<version>`, which leaves duplicate release tags possible.
- User impact: a maintainer can push two different tag names for the same package version and still satisfy the tested validator behavior.
- Suggested fix: add explicit tests for the accepted tag convention and rejected alternatives. If `v` tags are the intended public contract, test that a bare matching tag fails with a clear diagnostic.

#### TESTS-003: architecture boundary guards use raw text scanning instead of Rust-aware parsing

- Severity: Medium
- Evidence: `tests/architecture_boundaries.rs:1580-1623` enforces allowed crate references by scanning each source line for `crate::`; `tests/architecture_boundaries.rs:1672-1683` returns every raw substring from `crate::` to the end of the line. `tests/architecture_boundaries.rs:1798-1858` builds production lines with a hand-rolled filter, and `tests/architecture_boundaries.rs:1878-1881` skips only comment-only lines.
- Why this is a test bug: forbidden tokens inside string literals or inline comments can trip architecture tests even when no dependency exists, while semantic dependency forms outside the literal patterns are not checked as Rust syntax. This makes a high-value architecture gate noisy and easier to accidentally game.
- User impact: maintainers can lose trust in the gate after harmless copy or diagnostic strings fail it, and boundary reviews still need manual source-graph reasoning despite the test name implying stronger enforcement.
- Suggested fix: move the boundary checks to a Rust-aware parser such as `syn` for imports/paths, or at least strip string literals and inline comments before pattern matching. Keep the current text checks only as a lightweight fallback.

### Test Gaps

- No root integration test covers the npm-installed command path.
- No release validation test asserts that duplicate `v` and non-`v` tag forms cannot both pass.
- No fixture verifies the architecture scanner's behavior on strings, inline comments, aliases, or multi-reference lines.
