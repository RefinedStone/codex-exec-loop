# Product Status and Release Boundaries

[한국어](../ko/reference/product-status.md)

Audit baseline: [`prerelease@fa72e69a8`](https://github.com/RefinedStone/codex-exec-loop/commit/fa72e69a868826e6b61ec0f935260d1e5b9344a8),
2026-11-27 (Asia/Seoul). This is a small map of implemented paths and remaining proof, not a claim
that every planned feature is finished. [Current Product](current-product.md) owns behavior details;
the linked tests describe coverage. Their presence does not establish that a particular CI run or
real OS scenario passed.

## What a User Can Do

Akra ADE brings Codex conversations, code review, task acceptance criteria, execution evidence,
and delivery results into one development workspace. Desktop is available from the `prerelease`
source tree. TUI and CLI also have published native/npm releases; the published versions do not
automatically contain the features on this branch.

| Component | Responsibility | Available entry |
| --- | --- | --- |
| Desktop | Conversation and work panels; displays state and requests actions | `npm --prefix desktop run dev` in a prepared source checkout |
| Gateway | Local client connections, app-server relay, repository Kernel host | Built/prepared by Desktop `predev`; `akra-gateway` also runs independently |
| Kernel | Repository commands, decisions, state, and evidence policy | [Kernel specification](../../schema/akra-kernel-spec.json); `akra describe --json` |
| TUI | Terminal conversations and operation views | `akra` |
| CLI | Scriptable commands and queries | `akra config`, `akra kernel`; see [Current Product](current-product.md) |

## Implementation, UI, and Evidence

All Desktop rows below are source-development features. None is a packaged Desktop release.

| Area and plan | Code / contract | UI connection | Evidence and remaining verification | Distribution |
| --- | --- | --- | --- | --- |
| Conversation and Gateway ([plan 17](../plan/17-akra-desktop/README.md)) | [Gateway](../../src/adapter/inbound/gateway/) and [session](../../desktop/src/renderer/session/desktopSession.ts) | [Composer](../../desktop/src/renderer/conversation/Composer.tsx), approvals and server requests | [Fake Gateway screen flow](../../desktop/test/e2e/server-requests.mjs); real OS/manual differences remain in [#2846](https://github.com/RefinedStone/codex-exec-loop/issues/2846) | Source |
| Next turn queue, model, reasoning, and engine compaction | [Queue](desktop-next-turn-queue.md), [settings](desktop-next-turn-settings.md), [compaction state](../../desktop/src/renderer/session/threadCompaction.ts) | [Settings](../../desktop/src/renderer/conversation/NextTurnSettings.tsx) and [compaction control](../../desktop/src/renderer/conversation/ThreadCompactionControl.tsx) | Fake screen tests: [queue](../../desktop/test/e2e/next-turn-queue.mjs), [settings](../../desktop/test/e2e/next-turn-settings.mjs), [compaction](../../desktop/test/e2e/thread-compaction.mjs). Approval/sandbox selection remains [#2832](https://github.com/RefinedStone/codex-exec-loop/issues/2832) | Source |
| Task criteria, evidence, host verification, and PR results ([plan 24](../plan/24-akra-evidence-harness.md)) | [Acceptance](task-acceptance.md), [verification](task-verification.md), [delivery](task-delivery-evidence.md) | [Criteria](../../desktop/src/renderer/kernel/TaskCriteriaEditor.tsx), [verification](../../desktop/src/renderer/kernel/TaskVerificationView.tsx), [results](../../desktop/src/renderer/kernel/TaskResultView.tsx) | [Task authoring](../../desktop/test/e2e/task-authoring.mjs), [results](../../desktop/test/e2e/task-results.mjs), [waiver](../../desktop/test/e2e/task-criterion-waiver.mjs). Integrated acceptance remains [#2809](https://github.com/RefinedStone/codex-exec-loop/issues/2809) | Source |
| Issue drafts, masked publication, and Vault restoration ([#2414](https://github.com/RefinedStone/codex-exec-loop/issues/2414)) | [Gateway issue drafts](../../src/adapter/inbound/gateway/issue_draft.rs), [Vault unlock](evidence-vault-unlock.md), [Windows validator](issue-validator-windows.md) | [Draft panel](../../desktop/src/renderer/work/IssueDraftPanel.tsx), [creation panel](../../desktop/src/renderer/work/IssueCreationPanel.tsx) | [Draft](../../desktop/test/e2e/issue-draft.mjs), [restore](../../desktop/test/e2e/vault-restore.mjs); [real OS evidence](../validation/vault-real-os-manual.md), [Windows #2724](https://github.com/RefinedStone/codex-exec-loop/issues/2724) and [combined #2705](https://github.com/RefinedStone/codex-exec-loop/issues/2705) remain distinct gates | Source |
| Git review and graph ([plan 27](../plan/27-akra-git-graph.md)) | [Gateway Git API](../../src/adapter/inbound/gateway/git.rs) | [Git graph](../../desktop/src/renderer/git/GitGraphPanel.tsx) | [Graph screen test](../../desktop/test/e2e/git-graph.mjs); cherry-pick [#2847](https://github.com/RefinedStone/codex-exec-loop/issues/2847), protection [#2848](https://github.com/RefinedStone/codex-exec-loop/issues/2848), batch rebase [#2849](https://github.com/RefinedStone/codex-exec-loop/issues/2849), and controls [#2851](https://github.com/RefinedStone/codex-exec-loop/issues/2851) / [#2852](https://github.com/RefinedStone/codex-exec-loop/issues/2852) remain | Source |
| Browser panel and agent browser ([plan 17 R9](../plan/17-akra-desktop/roadmap/R9-agent-browser-and-screenshots.md)) | [Main-process browser bridge](../../desktop/src/main/browserAgentService.ts) and shared bridge boundary | Browser panel in the real Desktop app; the blueprint remains separate mock UI | [Browser](../../desktop/test/e2e/browser.mjs) and [automation](../../desktop/test/e2e/browserAutomation.mjs) tests exist. These do not prove every external-site login or all real OS cases in [#2846](https://github.com/RefinedStone/codex-exec-loop/issues/2846) | Source |
| External work documents and MCP ([plans 21/22](../plan/22-akra-work-context.md)) | [Context API](../../src/adapter/inbound/gateway/context.rs), [MCP API](../../src/adapter/inbound/gateway/mcp.rs) | [Documents](../../desktop/src/renderer/work/DocumentsPanel.tsx), [MCP write approvals](../../desktop/src/renderer/conversation/McpWriteApprovals.tsx) | [Documents screen test](../../desktop/test/e2e/documents-panel.mjs); actual provider accounts remain [#2860](https://github.com/RefinedStone/codex-exec-loop/issues/2860), context freshness/delivery [#2823–#2829](https://github.com/RefinedStone/codex-exec-loop/issues/2823) | Source |
| Local diagnostics | [Diagnostic contract](desktop-local-diagnostics.md) | [Diagnostic dialog](../../desktop/src/renderer/views/DiagnosticsDialog.tsx) | [Fake screen test](../../desktop/test/e2e/diagnostics.mjs); implementation/evidence in [#2780](https://github.com/RefinedStone/codex-exec-loop/pull/2780) | Source |

`parallel.evidence_mode` defaults to `observe`. A user must select `enforce` to hold delivery for
missing required evidence. The current `GitDiffCheckV1` recipe checks whitespace/conflict markers;
it does not prove a build or functional test. See the [delivery contract](task-delivery-evidence.md).

The [runtime capability contract](runtime-capabilities.md) describes supported engine operations.
A future backend or a blueprint screen is not a working production engine or UI. Plan 17's old
P2/R9 milestones are historical planning records; use the paths and evidence above to assess the
current implementation.

## Install and Start the Source Desktop

Use a source checkout of the `prerelease` branch with the Rust toolchain declared in
[rust-toolchain.toml](../../rust-toolchain.toml), Node/npm, official Codex CLI, and `codex login`
already prepared. From that checkout:

```sh
git switch prerelease
npm --prefix desktop ci
npm --prefix desktop run dev
```

The [predev script](../../desktop/scripts/prepare-dev.mjs) prepares Electron and builds
`akra-gateway`. Desktop starts/connects to the Gateway; choose a workspace in the app. If an
explicit `AKRA_GATEWAY_ENDPOINT` or `AKRA_GATEWAY_BIN` is supplied, predev skips its Gateway build.
The [Desktop guide](../../desktop/AGENTS.md) documents the development overrides.

This procedure is a source entry path checked against the scripts. This documentation update did
not launch the app, run a model turn, or certify an installed upgrade. `npm install -g
@refinedstone/akra` installs the published TUI/CLI launcher, not Desktop.

## Release and Rename Boundaries

| Channel / change | Implemented or published state | Remaining proof or action |
| --- | --- | --- |
| Published GitHub release | Latest release observed on the audit date: [v1.3.5](https://github.com/RefinedStone/codex-exec-loop/releases/tag/v1.3.5), published 2026-06-20, titled `codex-exec-loop-native v1.3.5` | It predates this branch's rebranding and Desktop work |
| Native/npm release candidate | [#2742](https://github.com/RefinedStone/codex-exec-loop/pull/2742) changes binary/archive names to `akra-ade`; `@refinedstone/akra` and command `akra` remain | [#2589](https://github.com/RefinedStone/codex-exec-loop/issues/2589): representative release artifacts and supported installation/upgrade checks; no new publication is claimed |
| Desktop | [Manifest](../../desktop/package.json): `0.0.0-development`, `productName: Akra ADE` | Packaged installers, signing and update delivery remain [R8](../plan/17-akra-desktop/roadmap/R8-packaging-and-operations.md) |
| Data and credentials | [#2732](https://github.com/RefinedStone/codex-exec-loop/pull/2732), [#2737](https://github.com/RefinedStone/codex-exec-loop/pull/2737), [#2743](https://github.com/RefinedStone/codex-exec-loop/pull/2743), [#2782](https://github.com/RefinedStone/codex-exec-loop/pull/2782) implement the N2 transition | [Naming and compatibility evidence](naming.md) separates fake/unit/macOS temporary-data proof from unverified OS credential decryption and old-binary upgrades; tracked by [#2588](https://github.com/RefinedStone/codex-exec-loop/issues/2588) |
| Source policy and notices | [L1 decisions](../plan/32-akra-ade-source-policy.md), [LICENSE](../../LICENSE), [contribution policy](../../CONTRIBUTING.md), [third-party notices](../../THIRD_PARTY_NOTICES/README.md) applied by [#2739](https://github.com/RefinedStone/codex-exec-loop/pull/2739) | L2 archive/npm staging contracts passed in the CI run below. Final release-profile installation and upgrade proof remains R1. External contribution terms remain undecided; external PRs are not accepted under the current documented policy |
| GitHub repository name | Current active URL is `RefinedStone/codex-exec-loop` | [G1 #2591](https://github.com/RefinedStone/codex-exec-loop/issues/2591) owns rename, redirects, remotes and consumer verification; future URLs must not replace working download URLs early |

L2 packaging evidence: [Node staging checks](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45909896894/job/140215516270)
and [Rust archive checks](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45909896894/job/140215516446)
passed at `e3c181c5e`. They construct and inspect fixture archives/packages, including the project
license and required notice paths. Packaging, license, notice and corresponding test files are
unchanged between that commit and this audit baseline. This proves the packaging contract; it
does not certify a published binary or an installed release upgrade.

The existing [release runbook](../plan/13-native-packaging-and-operator-runbook.md) and
[naming contract](naming.md) own packaging, compatibility and recovery details. Historic tags,
release assets and third-party notices retain their original identity.

## Ownership and Follow-Up

[D1 #2590](https://github.com/RefinedStone/codex-exec-loop/issues/2590) owns the entry-document
corrections, this small status map and its Korean translation. It covers the factual contradictions
and minimal state tracking of [#2481 R03](https://github.com/RefinedStone/codex-exec-loop/issues/2481);
it does not complete the A01–A12 roadmap, R03's future machine-checked status extensions, or the
unverified product/release work linked above.

Each feature PR that changes a row updates its code/evidence links and both language versions.
Existing protocol generation and CI-scope checks remain the machine-verifiable sources. Do not add
tests that merely assert this prose, or turn a test-file link into a claim that it passed.
