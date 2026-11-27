# Release and owner acceptance receipts

All 15 issues below remain open. Safe local checks passed. The genuine legacy
upgrade check failed at the candidate's authority read. This report adds executed
evidence to the existing runbooks; it does not replace their native acceptance.

Date: 2026-11-27 UTC / 2026-11-28 KST. Product source:
`865e0ade186221b24d02a4f54ab58d53f78b693d`. Host: Windows 10.0.26200 x64,
Rust 1.95.0, Node 24.13.0. Machine-readable evidence:
[owner-acceptance-20261127.json](owner-acceptance-20261127.json).
Historical smoke tool: `ee84865a2f14a03275caa2217955fb6fc3f007c0`.
Scoped tool PR: [#3021](https://github.com/RefinedStone/codex-exec-loop/pull/3021).
This is the failing baseline. Successor migration/recovery work must record its
own source and result rather than changing this failed run into a pass.

## Executed checks

| Check | Result | Boundary actually exercised |
| --- | --- | --- |
| Release, launcher and rename suites | 46 pass, 0 fail, 4 existing POSIX signal skips | Current archive rejection, launcher resolution, isolated config smoke, rename snapshots |
| Thirteen focused Desktop files | 251 pass, 0 fail | Profile paths/migration, Vault window/power/source, fake delegated chat and tunnel, issue-draft reasons |
| Desktop typecheck and production build | Both exit 0 | Unchanged accepted Desktop source |
| Real Electron Vault screen | 32 pass | Fake Gateway, synthetic original text and power events; actual Electron IPC/window separation |
| Native Rust selection | 84 pass, 0 fail, 1 existing manual case ignored | 46 restore-bridge, 14 shared-copy Gateway, 14 Windows validator, 1 Windows pointer-sharing failure, 9 Propose-flow tests |
| Unsupported-isolation publication | 1 pass | Real service blocks publication and exposes the existing actionable reason |
| Current/legacy archive and smoke suites | 51 pass, 0 fail | Historical digest pin, bounded GNU long names, task continuity, unchanged current release requirements |
| Genuine current archive fresh install | Exit 0 | Extracted native launcher, disposable home, config write and independent read |
| Genuine published-old/current pair | **Exit 1; acceptance blocked** | Old binary creates and rereads a real task; candidate cannot read the old authority |

The native Rust run took 408.81 seconds. Its single ignored case is
`manual_os::real_os_confirmation_and_credential_restore_roundtrip`, which requires
an explicit operator and disposable interactive OS account. No ignored case is
counted as a pass.

The smoke-fix worktree also ran the required
`node scripts/agent-plan.mjs --run`. Its Windows Full gate passed the Node checks,
Desktop typecheck/build, 1,527 Desktop tests with 8 existing skips, compilation,
and earlier native selectors. It then stopped making progress after the
repository-kernel restart module reported 23 passes. The native runner had no
live test child but did not return. The owned stalled monitor was stopped. This
full-gate result is **blocked**, with no normal completion code. The cause is not
established; no test or gate was weakened to hide it.

Commands used from the repository root:

```text
node --test scripts/repository-rename-audit.test.mjs npm/test/smoke-native-release.test.js npm/test/stage-packages.test.js npm/test/platform.test.js npm/test/bin.test.js
cargo test --locked --all-features --lib --target x86_64-pc-windows-msvc -- adapter::outbound::issue_code_validator::windows:: adapter::inbound::gateway::tests::issue_shared_copy_flow:: adapter::inbound::desktop_restore_bridge::tests:: issue_automation_flow storage_failures:: --nocapture --test-threads=2
cargo test --locked --all-features --lib --target x86_64-pc-windows-msvc unavailable_isolation_blocks_publish_with_actionable_reason -- --nocapture --test-threads=1
node --test npm/test/legacy-release-smoke.test.js npm/test/native-release-legacy-archive.test.js npm/test/smoke-native-release.test.js npm/test/stage-packages.test.js
```

Desktop commands used in its isolated checkout:

```text
npm test -- test/endpointAndVersion.test.ts test/vaultRestore.test.ts test/vaultRestorePower.test.ts test/vaultRestoreSource.test.ts test/vaultRestoreWindow.test.ts test/delegatedChatDriver.test.ts test/delegatedChatGateway.test.ts test/delegatedChatWindow.test.ts test/delegatedTunnelStatus.test.ts test/issueDraft.test.ts test/userDataMigration.test.ts test/dotsSettings.test.ts test/issueAutomationPolicyFile.test.ts
npm run check
npm run build
node --experimental-strip-types --disable-warning=ExperimentalWarning --disable-warning=MODULE_TYPELESS_PACKAGE_JSON test/e2e/vault-restore.mjs
```

## Actual legacy boundary

The published Windows `v1.3.5` archive is a genuine historical executable. Its
archive hash matched both the GitHub asset digest and sidecar; all 10 internal
payload hashes matched. `--help`, `doctor`, `status`, `queue` and its planning
contract run. Its absence of the later `config` command is not evidence that the
binary itself is unsupported.

The earlier config-based smoke made the wrong assumption for this artifact.
The explicit historical mode now verifies the old format and uses its supported
`doctor` and `planning-tool run` path. It preserves the strict verifier used for
new releases and npm staging. See [the scoped smoke receipt](native-release-smoke-20261128.md#published-legacy-input).

The candidate was built as a real **debug** package from the source SHA above.
The old binary wrote a synthetic task through its public CLI. In the non-Git
workspace, the candidate returned `unsupported authority-store schema version: 5`.
The current schema floor is 7 and the current version is 21
(`src/adapter/outbound/db/sqlite_planning_authority_adapter.rs:103`).
The old binary still read the same task after the candidate refusal.

The disposable Git-workspace probe found a separate intentional boundary:
`workspace_paths.rs:423` creates a new incarnation namespace when the historical
namespace is occupied. It preserves ambiguous old data rather than adopting a
possibly recycled repository's authority. Fixing the smoke must not lower either
guard. Schema-5 migration and explicit pre-incarnation recovery remain product
work under #2588.

The new archive and executable hashes are in the JSON receipt and the scoped
smoke document. This run proves neither release-profile packaging nor an npm
install, credential migration, or complete supported-platform upgrade matrix.

## Issue dispositions and required owner input

| Issue | New receipt | Remaining work and exact requirement |
| --- | --- | --- |
| [#2588](https://github.com/RefinedStone/codex-exec-loop/issues/2588) | Profile/migration fixtures pass; genuine legacy task creation and candidate refusal measured | Implement schema-5 migration and explicit pre-incarnation recovery. Owner must identify the supported legacy profile/cutover policy. Two-profile default SID-pipe matching needs a disposable Windows account without the owner's Gateway. Native credential continuity needs dedicated OS accounts. |
| [#2589](https://github.com/RefinedStone/codex-exec-loop/issues/2589) | Actual old artifact verified; actual new debug archive fresh install passes; corrected old/new task check fails truthfully | Resolve #2588, then select the release version and supported platform/channel matrix. Run real release-profile old/new upgrades and npm installation. Publication remains a separate owner operation. |
| [#2591](https://github.com/RefinedStone/codex-exec-loop/issues/2591) | Existing before-capture CLI succeeds against repository ID 1201646585 | Owner must accept N2/R1, inventory external consumers, schedule a quiet cutover and authorize rename. Capture after-state and compare only after the actual operation. |
| [#2703](https://github.com/RefinedStone/codex-exec-loop/issues/2703) | 14 native Windows validator tests pass, including actual LPAC/Job cleanup | Supply disposable native Linux 22.04/bubblewrap 0.13.1 and macOS hosts for remaining native contracts. WSL and ordinary unit tests cannot supply this evidence. |
| [#2704](https://github.com/RefinedStone/codex-exec-loop/issues/2704) | Restore-bridge fixtures and synthetic Electron boundaries pass | Operator must run real approval/refusal, dedicated OS key storage, restore and reconnect in each declared native profile. This run did not authorize native consent or credential changes. |
| [#2705](https://github.com/RefinedStone/codex-exec-loop/issues/2705) | Multi-session/late-result, actual Windows sharing-denial, restart and synthetic power boundaries pass | Actual OS lock/suspend and credential integration remain. ENOSPC/flush-failure acceptance requires an isolated fault volume; do not fill the user's disk. Existing [failure matrix](issue-vault-failure-matrix.md) remains the scenario owner. |
| [#2724](https://github.com/RefinedStone/codex-exec-loop/issues/2724) | 32 Electron assertions establish isolation and re-masking under synthetic events | Supply a disposable interactive Windows user and explicit native-dialog/Credential Manager authorization. Execute real approval, denial, expiry, lock and reconnect. The manual OS test remains ignored. |
| [#2846](https://github.com/RefinedStone/codex-exec-loop/issues/2846) | Current Desktop build, type contracts and focused fixtures pass | Windows/macOS operators must execute the existing [16-row matrix](desktop-manual-os-checklist.md). IME, physical DPI/monitors, editor launch and actual writer contention are unverified. Native credentials, real PR writes and model rows require their separate permissions. |
| [#2860](https://github.com/RefinedStone/codex-exec-loop/issues/2860) | Public Projects CLI read attempted; rejected for missing `read:project` | Owner must authorize the Projects scope or provide an approved read identity. Jira/Linear/Notion need named test site/team/page and credential setup. Jira/Linear comment creation/update require sandbox write authorization. No provider write occurred. |
| [#2861](https://github.com/RefinedStone/codex-exec-loop/issues/2861) | Desktop reason and matching Rust publication test pass | Default Ubuntu 24.04 remains unsupported under the existing ADR. A native Linux host is needed to compare unsupported and supported profiles without changing host policy. |
| [#2868](https://github.com/RefinedStone/codex-exec-loop/issues/2868) | Live `CI Gate` requirement and `RepositoryRole/5` PR bypass read | Owner must decide the required context and bypass policy separately, using the existing performance/acceptance package, then approve an exact settings delta. No settings were changed. |
| [#2870](https://github.com/RefinedStone/codex-exec-loop/issues/2870) | No tag ruleset or environment; repository metadata confirms `NPM_TOKEN` exists | Owner must name release actors/reviewers, tag policy, self-review/bypass policy and authorized package scope. Approve tag/environment protection changes. Token value and package permissions were not read or verified. |
| [#2879](https://github.com/RefinedStone/codex-exec-loop/issues/2879) | Fake driver/Gateway/window/tunnel contracts pass; tunnel-core issue 1 remains open | Owner must settle activation/default/migration and immediate-disable policy, plus linked boundary acceptance. Environment opt-in remains. No ChatGPT account/history was accessed. |
| [#2555](https://github.com/RefinedStone/codex-exec-loop/issues/2555) | All 9 Git/SQLite/fake-provider Propose flows pass | Owner must authorize native Start, planning and implementation model budgets, exact test repository/issue/base, and PR/status-comment writes. Fake task/PR/comment IDs are not live evidence. |
| [#2644](https://github.com/RefinedStone/codex-exec-loop/issues/2644) | Current issue and its no-merge/no-close contract read | Its document and PR must come from the authorized #2555 product flow. A manually written substitute would not satisfy the test. No substitute was created. |

## Read-only remote receipts

The before-rename snapshot contained 14 branches, 6 open PRs, 12 releases and 1
ruleset. It had 34 queued, 1 in-progress and 1 pending run. These are capture-time
observations, not a quiet window or values to reuse at cutover.

Ruleset `20411125` required `CI Gate` and contained one `RepositoryRole` actor 5
bypass in `pull_request` mode. The ruleset list had no tag target. The environments
API returned `total_count: 0`. Repository Actions secret **metadata** reported one
secret and the presence of `NPM_TOKEN`. This does not establish its token value,
package scope or environment protection.

Commands: `gh api repos/RefinedStone/codex-exec-loop/rulesets`,
`gh api repos/RefinedStone/codex-exec-loop/rulesets/20411125`,
`gh api repos/RefinedStone/codex-exec-loop/environments`, and a projection of
`gh api repos/RefinedStone/codex-exec-loop/actions/secrets` to count and
`NPM_TOKEN` presence only. The failed public read was
`gh project view 4247 --owner github --format json`. No scope refresh was run.

Raw local evidence is retained under
`C:/dev/akra-queue/codex-exec-loop-audit-20261127/owner-acceptance-*st01a21ea2*`
and `legacy-smoke-*st01a21ea2*`. The committed JSON omits temporary user paths,
secret values, raw native-screen content and private provider data.

Model calls, product-flow provider writes, native consent clicks, real credential
access, repository setting changes and package publication were all zero.
Review PR creation and Git delivery for these receipts are separate authorized
development operations.
