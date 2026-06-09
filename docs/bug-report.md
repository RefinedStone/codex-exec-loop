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
| `.codex-exec-loop/` | Completed | 2026-06-08 | tracked planning seed prompts, direction detail docs, and release followup prompts inspected. |
| `.gemini/` | Completed | 2026-06-08 | Gemini-specific styleguide and repository references inspected. |
| `artifacts/` | Completed | 2026-06-08 | tracked terminal bridge readiness captures and repository references inspected. |
| `tmp/` | Completed | 2026-06-08 | tracked temporary PNG payload and repository references inspected. |
| `docs/` | Completed | 2026-06-08 | project docs, validation matrix, release runbook, and OSS application draft inspected. |
| `src/` | Completed | 2026-06-08 | source adapters, admin draft flow, GitHub automation, and review polling inspected. |

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
- `cleanup_merged_worktrees.sh` does not appear to have tests for explicit target misspellings or remote branch deletion behavior.
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

## `.codex-exec-loop/`

### Scope

- Inspected files: `.codex-exec-loop/followups/*.md`, `.codex-exec-loop/planning/directions.toml`, `.codex-exec-loop/planning/directions/*.md`, `.codex-exec-loop/planning/prompts/queue-idle-review.md`, and `.codex-exec-loop/planning/result-output.md`.
- Inspected related design and packaging references: `docs/design/06-planning-runtime-and-draft-editor.md` and `scripts/package_native_release.sh`.
- Validation run: `git ls-files .codex-exec-loop`.
- Reference check: `find .codex-exec-loop/planning/directions -type f | sort` compared with `rg -n "detail_doc_path" .codex-exec-loop/planning/directions.toml`.
- Reference check: `find . -name 'plan_priority_queue.md' -o -path './.codex-exec-loop/followups/*'`.
- The audit did not inspect other top-level directories in this pass.

### Findings

#### CODEX-LOOP-001: tracked `directions.toml` contradicts the DB-backed authority model

- Severity: High
- Evidence: `docs/design/06-planning-runtime-and-draft-editor.md:10-14` says git-backed workspaces persist planning authority in SQLite and that tracked planning files remain prompts, direction detail docs, and result-output guidance only. The artifact table at `docs/design/06-planning-runtime-and-draft-editor.md:21-24` lists SQLite direction authority plus detail docs, `result-output.md`, and `queue-idle-review.md`, but not a tracked `directions.toml`.
- Contradicting file evidence: `git ls-files .codex-exec-loop/planning` still includes `.codex-exec-loop/planning/directions.toml`. That file contains queue-idle policy and a direction state at `.codex-exec-loop/planning/directions.toml:3-22`.
- Why this is a logic gap: the repository ships a file that looks like direction authority even though the runtime design says accepted DB direction authority is the source of truth. Operators and workers can read or edit the wrong artifact.
- User impact: a maintainer may update `directions.toml` expecting planning behavior to change, while the actual queue and direction state comes from the SQLite authority store.
- Suggested fix: either remove `directions.toml` from the tracked active seed, move it under a clearly named legacy/example path, or add a validator that fails when tracked file-backed direction authority exists beside the DB-backed contract.

#### CODEX-LOOP-002: two tracked direction detail docs are orphaned from the direction map

- Severity: Medium
- Evidence: `.codex-exec-loop/planning/directions/` contains three tracked detail docs: `claude-first-headless-cli-runner.md`, `context-first-architecture-and-doc-coherence.md`, and `terminal-agent-bridge-research-and-capability-boundary.md`. `.codex-exec-loop/planning/directions.toml:21` references only `context-first-architecture-and-doc-coherence.md`.
- Why this is a usage gap: a tracked detail doc under the canonical direction detail directory looks active, but it has no local direction mapping in the tracked seed. Unless the DB authority separately points to it, it will not be discoverable from the file-backed map.
- User impact: operators can spend time maintaining direction documents that are not part of the active planning authority path.
- Suggested fix: add a consistency check that every tracked `.codex-exec-loop/planning/directions/*.md` file is either referenced by accepted DB direction authority or explicitly marked as archived/research-only. For file seeds, keep a manifest that distinguishes active, proposed, and archived direction docs.

#### CODEX-LOOP-003: release-bundled followup prompts still point to a legacy queue file

- Severity: Medium
- Evidence: `scripts/package_native_release.sh:221-222` copies `.codex-exec-loop/followups` into native release bundles. `.codex-exec-loop/followups/10-review-queue.md:4` tells the agent to organize next work in `plan_priority_queue.md`, but `find . -name 'plan_priority_queue.md' -o -path './.codex-exec-loop/followups/*'` finds only the followup prompt files and no queue file.
- Related implementation evidence: current planning prompts and code repeatedly name accepted DB authority as the source of truth, for example `.codex-exec-loop/planning/prompts/queue-idle-review.md:9-14`.
- Why this is a bug: a release user following the bundled followup prompt is directed to create or update an unmanaged markdown queue file outside the current DB-backed task authority path.
- User impact: followup work can be recorded in a file the product does not treat as planning authority, so queue state in the app and the file a user edited can diverge.
- Suggested fix: rewrite bundled followup prompts to use the current planning tool/DB task authority flow, or stop packaging `.codex-exec-loop/followups` until the prompts have an active consumer and current queue contract.

### Test Gaps

- No repository check verifies that tracked `.codex-exec-loop/planning` files match the DB-backed planning artifact contract.
- No consistency check flags unreferenced direction detail docs under the canonical direction detail directory.
- No release packaging test asserts that bundled followup prompts mention only supported planning authority paths.

## `.gemini/`

### Scope

- Inspected files: `.gemini/styleguide.md`.
- Inspected repository rule references: `AGENTS.md`, `README.md`, and `Cargo.toml`.
- Reference check: `rg --hidden -n "Gemini|gemini|styleguide|Styleguide|\\.gemini" -S . --glob "!.git" --glob "!target/**" --glob "!docs/bug-report.md"`.
- Reference check: `rg -n "rust-version|edition|MSRV|let_chains|let chains|Rust" Cargo.toml README.md docs AGENTS.md .gemini/styleguide.md -S --glob "!docs/bug-report.md"`.
- The audit did not inspect other top-level directories in this pass.

### Findings

#### GEMINI-001: the Gemini styleguide is tracked but not discoverable from project docs

- Severity: Medium
- Evidence: `.gemini/styleguide.md` is the only tracked file under `.gemini/` and is only five lines long. `rg --hidden -n "Gemini|gemini|styleguide|Styleguide|\\.gemini" -S . --glob "!.git" --glob "!target/**" --glob "!docs/bug-report.md"` finds no project reference outside the file itself.
- Why this is a usage gap: contributors cannot tell whether this file is active review policy, an abandoned Gemini CLI artifact, or a local preference accidentally committed to the repository.
- User impact: Gemini-assisted reviews may follow rules that Codex/CI/human contributors do not know exist, while non-Gemini contributors never see the same guidance.
- Suggested fix: either document `.gemini/styleguide.md` from `AGENTS.md` or `docs/agent/`, or remove it if it is no longer an active project input.

#### GEMINI-002: the styleguide omits the repository's required delivery workflow

- Severity: Medium
- Evidence: `.gemini/styleguide.md:1-5` only requires Korean review/application/refutation text and mentions one Rust syntax point. It does not mention worktrees, `origin/prerelease`, commit/push/PR/rebase-merge delivery, or worktree cleanup. Those are mandatory in `AGENTS.md:1-4` and further detailed in `AGENTS.md:31-33`.
- Why this is a logic gap: an agent or reviewer that reads only the Gemini styleguide can perform changes in the integration checkout or stop after local edits, violating the repository's current delivery contract.
- User impact: Gemini-assisted edits can leave unreviewed local changes or branches that never land in `prerelease`, even while appearing to follow the tracked `.gemini` instructions.
- Suggested fix: make the Gemini file a thin pointer to `AGENTS.md`, or duplicate the minimum worktree/PR/cleanup contract with a warning that `AGENTS.md` is authoritative.

#### GEMINI-003: Rust syntax guidance is tied to "latest Rust" without a pinned toolchain

- Severity: Low
- Evidence: `.gemini/styleguide.md:4-5` asks reviewers to use the latest Rust syntax and specifically blesses `if let` combined with `&&`. `Cargo.toml:1-5` declares package metadata and `edition = "2024"`, but there is no `rust-version` key in the manifest and no `.gemini` note naming the minimum supported compiler.
- Why this is a maintenance gap: syntax guidance without an MSRV or toolchain baseline can drift ahead of the compiler used by CI, release packaging, or contributors.
- User impact: reviews can accept code because it is valid on a latest local compiler while another maintainer's documented setup fails to build it.
- Suggested fix: define the supported Rust toolchain in `Cargo.toml` or a checked-in toolchain policy, then make `.gemini/styleguide.md` refer to that baseline instead of "latest Rust" generally.

### Test Gaps

- No docs or lint check verifies that tracked agent-specific guidance files are referenced from the main contributor instructions.
- No repository rule check ensures secondary agent guides include the required worktree and PR delivery contract.
- No toolchain policy check ties Rust syntax guidance to a declared minimum compiler version.

## `artifacts/`

### Scope

- Inspected files: `artifacts/terminal-bridge-readiness-2026-04-23/*`.
- Inspected repository references from `docs/`, `src/`, `scripts/`, `README.md`, and `AGENTS.md`.
- Reference check: `rg -n "terminal-bridge-readiness|artifacts/terminal-bridge|05-stream\\.pipe|approval-prompt|missing-capture" docs src scripts README.md AGENTS.md -S --glob "!docs/bug-report.md"`.
- Validation run: `find artifacts -type f -print0 | xargs -0 file`.
- Reference check: `LC_ALL=C rg -n "\\x1b|/dev/pts|pid=|session=|approve\\?|server exited|can't find window" artifacts -S`.
- The audit did not inspect other top-level directories in this pass.

### Findings

#### ARTIFACTS-001: tracked terminal captures have no manifest or documentation entrypoint

- Severity: Medium
- Evidence: `artifacts/terminal-bridge-readiness-2026-04-23/` contains 16 tracked capture files, but `rg -n "terminal-bridge-readiness|artifacts/terminal-bridge|05-stream\\.pipe|approval-prompt|missing-capture" docs src scripts README.md AGENTS.md -S --glob "!docs/bug-report.md"` returns no references.
- Why this is a usage gap: the captures look like validation evidence, but the repository does not explain the scenario, command sequence, expected pass/fail interpretation, related code path, or whether the dated artifact is still current.
- User impact: reviewers cannot tell whether the files are required regression evidence, obsolete research output, or accidental local logs.
- Suggested fix: add `artifacts/terminal-bridge-readiness-2026-04-23/README.md` with the scenario, source commit, capture commands, expected observations, and current relevance. If the evidence is obsolete, move it to docs or remove it from tracked source.

#### ARTIFACTS-002: captures preserve local process and terminal identifiers

- Severity: Medium
- Evidence: `artifacts/terminal-bridge-readiness-2026-04-23/01-pane-discovery.txt:1` records `tty=/dev/pts/10`, `pid=1621979`, and a local tmux session/window name. `artifacts/terminal-bridge-readiness-2026-04-23/12-recovery-anchor.txt:1` records `pane_id=%2`, `tty=/dev/pts/12`, and `pid=1622004`.
- Why this is a bug: tracked artifacts should be scrubbed or intentionally synthetic. Local process IDs, pane IDs, and TTY paths are not reproducible evidence and can leak operator environment details when copied from a real terminal.
- User impact: future captures may accidentally commit more sensitive local context because the current directory normalizes committing raw terminal metadata.
- Suggested fix: replace local identifiers with stable placeholders, and add a capture-scrubbing checklist for terminal artifacts before they are committed.

#### ARTIFACTS-003: raw terminal escape sequences make text captures look like binary payloads

- Severity: Low
- Evidence: `find artifacts -type f -print0 | xargs -0 file` reports several `.txt` captures as `RAGE Package Format (RPF)`, and reports stream logs as `ASCII text, with escape sequences`. `LC_ALL=C rg -n "\\x1b" artifacts -S` finds bracketed-paste escape sequences in `05-stream.pipe.log`, `06-stream-at-0.5s.log`, and `07-stream-at-1.8s.log`.
- Why this is a maintenance gap: generic review and security tooling can misclassify these text fixtures, while human reviewers see noisy control bytes instead of the behavioral evidence.
- User impact: diffs, grep output, and artifact scans are harder to interpret, which lowers confidence in terminal-readiness evidence.
- Suggested fix: keep raw captures only when necessary, but also commit normalized `.txt` summaries without escape bytes and document which file is authoritative.

### Test Gaps

- No artifact manifest check requires tracked evidence directories to explain scenario, command, date, source commit, and current status.
- No scrubber or lint check flags TTY paths, process IDs, pane IDs, or other local terminal identifiers in tracked artifacts.
- No check distinguishes intentionally raw terminal captures from normalized reviewable text.

## `tmp/`

### Scope

- Inspected files: `tmp/img.png`.
- Inspected repository references and ignore rules: `.gitignore`, `README.md`, `docs/`, `scripts/`, `src/`, `templates/`, and `assets/`.
- Validation run: `git ls-files tmp`.
- Validation run: `file tmp/img.png && sips -g pixelWidth -g pixelHeight tmp/img.png`.
- Reference check: `rg -n "tmp/img\\.png|img\\.png" -S . --glob "!target/**" --glob "!.git/**" --glob "!docs/bug-report.md"`.
- The audit did not inspect other top-level directories in this pass.

### Findings

#### TMP-001: a temporary screenshot is tracked as source without any references

- Severity: Medium
- Evidence: `git ls-files tmp` returns only `tmp/img.png`. `file tmp/img.png` identifies it as a 1536 x 1024 RGB PNG, and `du -h tmp/img.png` reports about 2.4 MiB. `rg -n "tmp/img\\.png|img\\.png" -S . --glob "!target/**" --glob "!.git/**" --glob "!docs/bug-report.md"` returns no references.
- Why this is a usage gap: a directory named `tmp/` implies disposable local output, but the image is checked into source control and has no consumer, manifest, README, or test relationship.
- User impact: clones and reviews carry a multi-megabyte binary payload that maintainers cannot safely classify as runtime asset, evidence, or accidental scratch output.
- Suggested fix: remove the file if it is scratch output. If it is intentional evidence, move it under a documented artifact or visual-regression directory with a README explaining source, date, expected use, and regeneration steps.

#### TMP-002: `tmp/` is not ignored, so future local scratch files can be committed accidentally

- Severity: Medium
- Evidence: `.gitignore:1-10` ignores `logs/`, `artifacts/`, `.codex`, `.idea/`, `target/`, `dist/`, npm staging/vendor directories, `.codex-exec-loop/runtime/`, and `.reference`, but it does not ignore `tmp/`.
- Why this is a bug: the current tracked PNG proves that scratch output can enter the repository, and the ignore rules do not prevent the next generated file from being staged.
- User impact: future local screenshots, debug dumps, or generated binary files can become review noise or leak environment-specific state.
- Suggested fix: add a `tmp/` ignore rule after moving or deleting the intentional tracked file. If a tracked fixture directory is needed, use a non-temporary name and document it.

#### TMP-003: the screenshot can be mistaken for a current visual baseline

- Severity: Low
- Evidence: visual inspection of `tmp/img.png` shows a graphic admin dashboard screenshot with fixed labels such as `AKRA v0.9.0-beta`, `Lv. 24`, and dashboard metrics. The current crate version is `1.3.3` in `Cargo.toml:3`, and similar hard-coded admin copy is already present in `templates/admin/base.html:974-991`.
- Why this is a maintenance gap: without context, reviewers may treat the image as an expected current UI baseline, even though it appears stale and is not tied to an automated screenshot test.
- User impact: visual regressions can be argued from an unmanaged screenshot rather than a reproducible test artifact.
- Suggested fix: either delete the screenshot or move it into a documented visual reference set with the source route, app version, viewport, fixture data, and regeneration command.

### Test Gaps

- No repository hygiene check prevents tracked files under `tmp/`.
- No binary-asset manifest check requires purpose and regeneration metadata for large PNGs outside asset directories.
- No visual regression workflow ties admin dashboard screenshots to reproducible fixture data.

## `docs/`

### Scope

- Inspected files under `docs/`, with focus on `docs/README.md`, `docs/plan/12-platform-validation-matrix.md`, `docs/plan/13-native-packaging-and-operator-runbook.md`, `docs/plan/14-codex-for-oss-application.md`, and `docs/validation/*`.
- Inspected related root docs: `README.md` and `AGENTS.md`.
- Validation run: `bash scripts/summarize_native_validation.sh`.
- Validation run: `bash scripts/summarize_native_validation.sh --check-profile prompt-input-delay-pty`.
- Reference scan: `rg -n "TODO|FIXME|stale|obsolete|deprecated|tmux|terminal bridge|v0\\.9|0\\.9\\.0|1\\.3\\.3|npm|NPM_TOKEN|plan_priority_queue|directions\\.toml|file-backed|DB authority|latest|2026-04|2026-05|2026-06" docs README.md AGENTS.md -S --glob "!docs/bug-report.md"`.
- The audit did not inspect other top-level directories in this pass.

### Findings

#### DOCS-001: required terminal-baseline validation is documented but has zero passing rows

- Severity: High
- Evidence: `docs/plan/12-platform-validation-matrix.md:51-60` marks macOS Terminal.app, macOS iTerm2, Windows Terminal PowerShell, and Windows Terminal WSL bash as required `terminal-baseline` rows. `docs/validation/README.md:19-25` says to use `bash scripts/summarize_native_validation.sh` before calling the matrix complete.
- Validation evidence: `bash scripts/summarize_native_validation.sh` reports `required pass: 0/8`, `required missing: 8`, and no passing required baseline rows.
- Why this is a bug: the docs define a required validation gate, but the checked-in validation directory cannot satisfy it. There is no top-level warning that terminal-baseline coverage is currently empty.
- User impact: maintainers can cite the validation matrix or committed validation records while the required baseline rows are completely missing.
- Suggested fix: add a visible validation status section that records the current summary output and distinguishes missing required baseline rows from completed coverage. Alternatively, downgrade uncollected rows or make the required baseline a CI/reporting gate.

#### DOCS-002: prompt-input-delay records are mostly blockers but the docs do not surface that status

- Severity: Medium
- Evidence: `docs/plan/12-platform-validation-matrix.md:156-165` defines five required `prompt-input-delay-pty` rows. The current checked-in validation files contain one pass and four blockers, and `bash scripts/summarize_native_validation.sh --check-profile prompt-input-delay-pty` reports `required pass: 1/5` and `required non-pass: 4`.
- Why this is a usage gap: the raw records are present, but the docs do not provide a current status rollup near the matrix. A reader has to run the helper to learn that most required rows are blockers.
- User impact: prompt latency or PTY-related work can be treated as validated even though the recorded matrix still describes unresolved blockers.
- Suggested fix: commit a small generated or manually refreshed summary under `docs/validation/README.md`, and require PRs touching terminal behavior to update it when the helper output changes.

#### DOCS-003: release docs omit the duplicate-tag and optional-npm-publish risks

- Severity: Medium
- Evidence: `README.md:444-447` says release tags build native bundles, create or update the GitHub Release, and publish npm packages when `NPM_TOKEN` is configured. `docs/plan/13-native-packaging-and-operator-runbook.md:180-200` says accepted tags are any pushed tag and that npm platform packages are published before the main package.
- Why this is a logic gap: the docs do not warn that both `v<version>` and bare `<version>` tags are accepted by the current validator, and they do not say that a missing `NPM_TOKEN` makes the npm job skip successfully after GitHub Release assets are created.
- User impact: release operators can create duplicate GitHub Releases for one package version or mistake a green tag workflow for a full npm release.
- Suggested fix: document the single intended tag convention and the npm publish mode explicitly. If npm publishing is required, state that missing `NPM_TOKEN` must fail the release; if optional, document GitHub-only release semantics.

#### DOCS-004: the OSS application draft stores time-sensitive public metrics

- Severity: Low
- Evidence: `docs/plan/14-codex-for-oss-application.md:32-40` stores public signals "As of 2026-06-04", including GitHub stars, forks, open issues, and npm downloads. The same section includes refresh commands at `docs/plan/14-codex-for-oss-application.md:41-48`.
- Why this is a maintenance gap: the document is a form draft, but the time-sensitive metrics sit in checked-in prose and can become stale quickly after the stated date.
- User impact: future submissions or README copy can accidentally reuse outdated usage signals.
- Suggested fix: keep the dated snapshot only as historical evidence and add a clear "refresh before use" warning to the form draft body, or move mutable metrics into a generated note that is not treated as current project truth.

### Test Gaps

- No docs check fails when required validation matrix rows have zero passing records.
- No docs status artifact records the current `summarize_native_validation.sh` output for reviewers.
- No release-doc check verifies that tag convention and npm publish behavior match the workflow and validation scripts.

## `src/`

### Scope

- Inspected source under `src/`, with focus on outbound adapters, admin draft routes, planning workspace storage, GitHub automation, review polling, and subprocess handling.
- Evidence scans: `rg -n "Command::new|gh|draft_name|csrf|token|wait_with_output|\\.output\\(" src`.
- Line evidence captured with `nl -ba` for the files cited below.
- The audit did not inspect other top-level directories in this pass.

### Findings

#### SRC-001: admin draft names are trusted as storage identifiers

- Severity: High
- Evidence: `src/adapter/inbound/admin_api/pages.rs:572-590` and `src/adapter/inbound/admin_api/api.rs:176-193` take `draft_name` directly from the route path and pass it into `PlanningAdminDraftLoadRequest`. Save, validate, and promote routes repeat the same pattern at `src/adapter/inbound/admin_api/pages.rs:624-706` and `src/adapter/inbound/admin_api/api.rs:196-263`.
- Evidence: `src/application/service/planning/admin/draft_session.rs:70-72`, `src/application/service/planning/admin/draft_session.rs:82-88`, and `src/application/service/planning/admin/draft_session.rs:100-107` pass that `draft_name` into workspace operations without validating a draft-id grammar.
- Evidence: `src/adapter/outbound/filesystem/planning_workspace.rs:56-60` builds the draft directory with `Path::new(workspace_dir).join(PLANNING_DRAFTS_DIRECTORY).join(draft_name)`. In direct-filesystem mode, `src/adapter/outbound/filesystem/planning_workspace.rs:306-317` and `src/adapter/outbound/filesystem/planning_workspace.rs:389-392` create or write under that path.
- Why this is a bug: `active_path` is normalized, but the draft namespace key is not. Generated draft names are safe, but public admin/API routes also accept caller-supplied draft names. A malformed draft name can become a filesystem path segment in direct mode, or a confusing DB/display identifier in repo-scoped mode.
- User impact: local admin users and automation can target draft names that do not follow the generated `admin-...` contract. In non-git/direct workspaces this can escape the intended draft directory; in git-backed workspaces it can create unreadable or misleading draft records.
- Suggested fix: add a shared draft-name validator before facade calls and before workspace adapter joins. Accept only the generated draft id grammar or a conservative segment grammar such as ASCII alnum plus `._-`, with no slash, backslash, colon, control characters, empty values, or `.`/`..`. Return `400` from inbound handlers for invalid names, and add HTML/API tests for encoded slash and parent-directory attempts.

#### SRC-002: PR creation sends full PR title/body through argv and failure labels

- Severity: Medium
- Evidence: `src/adapter/outbound/github/automation.rs:321-337` calls `bash scripts/gh-akra.sh pr create` with `--title title` and `--body body` as command arguments.
- Evidence: `src/adapter/outbound/github/automation.rs:500-509` includes `args.join(" ")` in command failure errors. `src/adapter/outbound/github/automation.rs:521-534` also passes the same joined argument string as the subprocess timeout label and spawn context. `src/subprocess.rs:63-70` formats that label into timeout errors.
- Why this is a bug: PR bodies are generated from task and delivery context, so they can contain user prompts, local paths, issue details, or copied diagnostics. Passing them as argv exposes them to process listings while the command runs, and the joined command label can echo the whole body into logs/errors on failure or timeout.
- User impact: a failed or slow GitHub automation operation can leak large private planning context into terminal logs, admin status, TUI notices, or support screenshots. Long multiline PR bodies also make failure messages noisy and hard to diagnose.
- Suggested fix: pass PR body through stdin or a temporary body file, and redact title/body in command labels. Keep a structured operation label like `bash scripts/gh-akra.sh pr create --base <base> --head <head> --title <redacted> --body <redacted>`. Add a unit test that simulates a failed `pr create` and asserts sensitive body text is absent from the error.

#### SRC-003: GitHub review credential discovery can block without the shared subprocess timeout

- Severity: Medium
- Evidence: `src/adapter/outbound/github/review_poller.rs:167-176` runs `git ... .output()` directly while resolving repo metadata. `src/adapter/outbound/github/review_poller.rs:240-246` runs `gh auth token` directly with `.output()`. `src/adapter/outbound/github/review_poller.rs:278-300` spawns `git credential fill` and waits with `child.wait_with_output()`.
- Why this is a bug: these calls bypass the shared timeout in `src/subprocess.rs`. The adapter does set `GIT_TERMINAL_PROMPT=0` for `git credential fill`, but a credential helper, `gh`, or local git command can still hang due to platform credential UI, locked keychain, broken helper configuration, or network-backed helpers.
- User impact: review polling discovery can stall the TUI or background review integration instead of degrading to "no review poller available." This is especially painful because review polling is a convenience path, not a core startup requirement.
- Suggested fix: route these subprocesses through the shared `subprocess::command_output` / `wait_with_output` helpers or a review-poller-specific timeout, keep stdin null where possible, and treat timeout as `Ok(None)` for credential discovery. Add fake `gh` and fake credential-helper tests that sleep longer than the timeout and verify discovery returns promptly.

### Test Gaps

- No admin route test sends encoded slash, backslash, `..`, empty, or control-character draft names and asserts `400`.
- No workspace adapter test proves `draft_name` cannot escape `.codex-exec-loop/planning/drafts` in direct-filesystem mode.
- No GitHub automation test asserts PR body/title text is redacted from argv-derived error labels.
- No review-poller test covers hanging `gh auth token` or `git credential fill` helpers under the shared subprocess timeout.
