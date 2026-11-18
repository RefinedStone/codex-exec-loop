# External tools: sequential implementation and delivery

## Contract

- User requested planning and execution of all five outcomes, in order.
- One outcome owns one worktree, branch, and PR against `prerelease`.
- Sequence: implementation, required checks, PR, self-review, CI/review fixes,
  rebase merge, cleanup, next outcome.
- Revisit prior PR review threads and post-merge checks before final delivery.
- User clarified during Jira delivery: when existing CI blocks integration,
  keep the PR open and proceed to the next independent outcome after self-review.
  Do not weaken checks or merge a failing PR. Revisit integration in the final
  review/CI sweep. Start independent branches from current `origin/prerelease`.
- No real model turns, external service mutations, or changes to M5 worktrees.
- No Claude subagents. Independent planning investigations are read-only.
- Baseline: `56b99b83757e98b72a53f960b7d2eec7b2af0d09`.
- Concurrent work: `codex/q5-session-slots-2`; do not edit its checkout.

## Planning status

Source investigations are complete. This is the execution plan, not a claim
that implementation or validation has passed.
The user selected existing Desktop login for Jira in this session.
Reuse Desktop credentials without exporting secrets to the renderer or Gateway.
Use session-scoped reverse RPC through the existing renderer JSON-RPC client.
The main process retains opaque Gateway transport, as desktop/AGENTS.md requires.
Renderer forwards a validated target to JiraService through typed preload IPC.
No token or authorization header leaves main.

### Jira transport decisions

- Register: `akra/resourceVersion/register {kind:"jira",siteUrl}`.
  Replace the session's existing registration and cancel its pending requests.
- Unregister: `akra/resourceVersion/unregister {kind:"jira"}`.
- Reverse request: `akra/resourceVersion/current {kind:"jira",key,url}`.
  Result: `{version}`. Use reserved string IDs with prefix
  `akra/resourceVersion/request/`; consume late responses rather than forwarding.
- Registry tracks actual Gateway session ownership; no client-provided identity.
  Select one matching session, not a broadcast; retain others as standbys.
- Disconnect drops registrations and immediately fails pending requests.
  A call is bounded to ten seconds; no retry across clients within one call.
- Remote-host sessions cannot register, answer, or receive these requests.
- Desktop registers on local connection and Jira connection changes.
  Jira disconnect/account replacement invalidates in-flight lookup results.
- Pass full resources to the version provider and expose availability. Fresh
  shared versions can establish a thread baseline even without a live provider.
- Site identity includes non-default ports and Jira context paths. Keep root
  default-port keys compatible. Legacy ambiguous Jira links require transcript
  recollection rather than assigning the first stored URL to every old link.
  Preserve unrecovered legacy links under non-queryable keys; never discard
  references merely because they fall outside the transcript pagination cap.
- Tests cover session isolation, stale/late answers, timeout/disconnect,
  context-path and port isolation, baseline and failed-check preservation.

## Ordered outcomes

- [ ] 1. Jira document change detection.
  - Read the existing `ExternalResourceVersionPort`, `WorkContextService`,
    GitHub provider, Jira recognizer, and Desktop `JiraService`.
  - Existing Jira credentials belong to Electron safeStorage. The Rust Gateway
    cannot directly reuse them. Resolve the connection boundary before coding.
  - Keep version comparison, read baseline, throttling, and persistence in the
    existing work-context service.
  - Query only the issue update time. Reject a mismatched site and malformed key.
  - Verify first observation, unchanged and changed timestamps, mark-as-read,
    unavailable login, permission failures, malformed response, and disconnect.
  - Do not replace an existing version after a failed check.
  - Worktree: `C:/dev/akra-queue/codex-exec-loop-jira-versions`.
  - Branch: `codex/jira-document-versions`.
  - Official Jira Cloud API documentation confirms a GET issue request can
    select `fields=updated`; do not set `updateHistory=true`.
  - Preserve Cloud REST v3 and Server REST v2 selection and Server context paths.
- [ ] 2. Notion document change detection.
  - Reuse the resident MCP backend and its authentication.
  - Verify actual tool contracts before choosing the version extraction logic.
  - Account for read-only pages and inaccessible pages.
  - Preserve the same work-context comparison and failure semantics as outcome 1.
  - Test structured/text MCP results, tool errors, missing version, changed and
    unchanged pages, and registration changes.
- [ ] 3. MCP tool-list cache.
  - Cache discovery metadata per server; do not cache tool execution results.
  - Define freshness, configuration/authentication invalidation, explicit reload,
    and unsuccessful discovery handling from existing backend behavior.
  - Prove fewer discovery calls with deterministic call counts.
  - Test cold/warm lookup, stale refresh, changed/removed/disabled servers,
    authentication changes, and failed refresh.
  - Add a catalog-read backend operation rather than caching
    `McpHubServerStatus`. `servers`, `describe`, Desktop statuses, and `call`
    retain live behavior.
  - Persist complete tool metadata beneath the private hub home, with a fixed
    ten-minute freshness interval and injected clock for tests.
  - Cache identity includes effective configuration and relevant environment
    inputs as digests, never plaintext credentials.
  - Refresh only missing/stale servers. Do not cache partial discovery or
    authentication/connection failures; a successful empty catalog is valid.
  - Configuration changes must retire an old resident engine before discovery.
  - Invalidate on register/remove/login and explicit reload, not idle release.
    Separate shutdown from explicit invalidation if the existing reload path
    is shared. Prevent an in-flight old refresh from resurrecting stale entries.
  - Verify persistence across backend reconstruction, exact discovery call
    counts, invalidation races, and old-Gateway routing behavior.
  - Main edit surfaces: backend port, MCP domain wire values, search service,
    MCP backend and new cache module, Gateway client, Gateway MCP handler.
- [ ] 4. Full R9 browser control and screenshots.
  - Implement the scope of `R9-agent-browser-and-screenshots.md`, not just a
    status/snapshot prototype.
  - Preserve Electron sandbox and existing browser profile ownership.
  - Include the local tool bridge, snapshots and references, browser actions,
    observations, screenshots, user handoff, and lifecycle/error handling.
  - Validate tools without model turns against controlled local pages.
  - Capture and inspect real UI at desktop and narrow widths.
  - Local hosts only; one controlling thread lease per visible view. Calls to
    hidden, destroyed, wrong-owner, or remote views fail without acting.
  - Keep one view and same-origin frame support as the roadmap specifies.
  - Prove per-thread MCP registration and executable build before action work.
  - Invalidate refs on navigation/profile replacement. Pause on human input or
    debugger detach. Resume requires explicit user action.
  - Use bounded observer buffers, direct MCP fixtures, and deterministic event
    synchronization. Do not copy existing fixed-sleep tests.
  - B5 offline measurements use fixed local fixtures. Private-site and
    model-backed comparison evidence requires access/authorization and must be
    reported separately if unavailable.
- [ ] 5. R12 read-only Linear panel.
  - Implement connection, issue list, documented filters, prompt preparation,
    and resource recognition from `R12-linear-panel.md`.
  - Keep secrets in main-process secure storage.
  - Sending a prepared prompt remains a user action.
  - Use stable team/project/cycle identifiers. Filter the latest thirty cached
    issues in main; label that scope and avoid network calls on filter changes.
  - On handoff use the open local thread folder, otherwise ask through the
    existing folder chooser; cancellation preserves the draft.
  - Validate GraphQL errors even on HTTP 200. Preserve essential issue metadata
    before truncating description/comments.
  - No issue writes, OAuth expansion, or Linear change provider in this outcome.
  - Validate normal, empty, disconnected, unauthorized, and failed-network states.
  - Inspect real UI screenshots and verify the prepared prompt without a model turn.
  - The roadmap suggests splitting Desktop and Rust into two PRs. The user's
    explicit one-outcome-at-a-time PR workflow takes precedence: keep the panel
    and recognizer together, with small independently verified commits.
  - Live workspace validation depends on a user-owned Linear account/key.
    Never create accounts, expose secrets in chat, or claim fixture verification
    as proof of the live account.

## Per-PR evidence and integration

1. Recheck active worktrees and open PRs before each branch or shared-file edit.
2. Fetch `origin/prerelease`; start each outcome from that branch after the prior
   outcome has passed its delivery gate.
3. Use local neighboring tests; add regression tests for behavior, not prose.
4. Run diagnostics on edited source before builds.
5. Run `node scripts/agent-plan.mjs` and exactly its selected local checks.
6. For Desktop edits also run `npm run check` and `npx vitest run`; UI edits
   require `npm run e2e` and inspected captures.
7. Run `cargo fmt --all` for Rust edits and `git diff --check` before commits.
8. Verify repo-local GitHub write identity is `RefinedStone`; never print tokens.
9. Commit verified increments, push, and create the outcome PR.
10. Review the whole diff and real-surface evidence; record and fix valid findings.
11. Subscribe to CI completion. Inspect review threads for the reviewed head.
12. Merge only with required `CI Gate` passing and known blocking findings fixed.
    Use rebase merge; refresh an existing branch only for an actual need.
13. Use the repository cleanup helper for this outcome's clean merged worktree.
14. Maintain a ledger of PR number, head/merge SHA, checks, review findings,
    response links, and remaining external verification.

## Final sweep

- [ ] Inspect all five PRs for subsequently posted reviews and failed checks.
- [ ] Rebase-merge and clean up PRs only after their CI blockers are resolved.
- [ ] Address valid feedback in scoped follow-up PRs when the original is merged.
- [ ] Reply to review feedback in Korean with the fix and verification evidence.
- [ ] Confirm no worktree cleanup touches M5 or other owners' uncommitted changes.
- [ ] Report all five delivered outcomes and any explicitly unverified external
  account behavior without presenting mocks as live-service proof.
