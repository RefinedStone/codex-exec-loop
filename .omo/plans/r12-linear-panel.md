# R12 read-only Linear delivery

One worktree, branch and PR from fetched origin/prerelease. This is the fifth
sequential outcome. R9 has gate approval; its existing audit blocker is not bypassed.
Do not touch concurrent desktop-explicit-ui-states work or merge unapproved work.

## Contract

- Personal API key only. Main sends POST queries to https://api.linear.app/graphql
  with Authorization equal to the key, not Bearer. No mutations or OAuth.
- Verify viewer.id/name before saving. Encrypt the token and viewer metadata in
  userData/linear-token.bin through safeStorage; no plaintext token in DTOs/logs.
  Encryption unavailable means no save. Failed reconnect preserves the old key.
- Bound network time and response bytes. Reject redirects, malformed/partial
  GraphQL errors, missing required data, and unsuccessful HTTP responses.
- Read viewer.assignedIssues(first:30, orderBy:updatedAt, includeArchived:false,
  filter:{state:{type:{nin:["completed","canceled","duplicate"]}}}).
- Query stable IDs for team/project/cycle and cycle.isActive. Filter the cached
  latest thirty in main by IDs/currentCycle; filtering makes no network request.
  Open and refresh fetch. Disconnect invalidates pending results and cached data.
- Fetch detail only for handoff. Use comments(first:10, orderBy:createdAt):
  the current schema defines first as newest first; roadmap last:10 was stale.
  Header preserves identifier, title, URL and branchName before bounded Markdown
  description/comments. Each comment <=2000 characters and total <=20000,
  including omission markers. No actual branch or thread is created by handoff.
- Handoff uses the open local thread's cwd; otherwise ask for a folder.
  Cancel leaves existing draft intact. Fill an editable home draft, never send.
- Exact HTTPS linear.app issue URLs only for external opening. Reject credentials,
  lookalikes, unsafe schemes and non-issue paths.
- Add Rust LinearRecognizer: canonical workspace/IDENTIFIER key, slug excluded.
  Same issue with different slug/query/fragment deduplicates. No version provider.

## Ownership

- Service child: desktop/src/main/linearService.ts and desktop/test/linearService.test.ts.
- UI child: desktop/src/renderer/work/LinearPanel.tsx and its own new style file only.
- Recognizer child: src/domain/work_context.rs only, including colocated tests.
- Parent: shared DTOs/bridge/preload/main/App wiring, folder handoff, document label,
  full-bridge test mocks, real Electron fixture, docs, gates and delivery.
- Shared types live in desktop/src/shared/linear.ts. Children do not edit them.

## UI brief

Operate mode. Reuse existing work-panel classes, tokens, row/group hierarchy.
Connection form, loading, clear errors, grouped issue list, empty/filtered-empty
states, compact wrapping filters, refresh/disconnect, and draft handoff.
No shell redesign. Clear the API key field after successful connection.

## Verification

Service tests cover authentication/errors/bounds/filters/key lifecycle/races and
prompt caps. Recognizer tests cover valid/invalid URLs, key case, punctuation and
deduplication. No fixed sleeps in tests.

Parent runs real Electron against fake GraphQL + fake Gateway + isolated userData:
invalid key, encryption, list, filters, refresh/disconnect, draft handoff and
folder cancellation with no model turn. Inspect native UI at wide/narrow sizes
in light/dark and relevant error/loading states.

Run planner-selected gates, Desktop check/Vitest/e2e, fmt and diff checks.
One gate reviewer, fix findings, push PR, monitor CI and reviews. Existing
picomatch audit stays visible. Live Linear account validation requires a
user-provided account/key; finish all independent fixture checks first.

## Schema evidence

Context7 official Linear docs and current public Apollo schema verified:
viewer.assignedIssues filters/order/limits; cycle.isActive; team.activeCycle;
comments first=newest/last=oldest; personal-key auth header; HTTP200 may contain
GraphQL errors. Sources: linear.app/developers/graphql and the public
Linear-API current schema in studio.apollographql.com.
