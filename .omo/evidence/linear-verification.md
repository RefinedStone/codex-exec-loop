# R12 read-only Linear verification

## Implemented behavior

- Personal-key viewer verification before encrypted storage.
- Fixed-endpoint GraphQL queries only; redirects rejected, time and body bounded.
- Latest 30 assigned incomplete issues; stable-ID team/project and active-cycle
  filters read the main-process cache without a network call.
- Description and ten newest comments on handoff. Prompt capped at 20,000
  characters, including omission markers; branch and issue metadata come first.
- Open local thread folder reuse, otherwise folder selection. Cancellation
  preserves the draft. Handoff sends no thread/start, turn/start or turn/steer.
- Related-document recognition with workspace/identifier canonical identity.
- Disconnect cancels pending requests and prevents stale UI/cache restoration.

## Evidence

- Service tests: 81 passed.
- Work-context tests: 21 passed, including three Linear recognizer cases.
- Full Desktop type checks passed; 334 unit tests passed, three existing skips.
- Full `npm run e2e` passed, including Linear's local HTTP GraphQL fixture and
  fake Gateway, encrypted-file checks, filters, folder cancellation and reuse,
  draft-only handoff, failures, disconnect races and related-document display.
- Planner selected Windows Full. Full stopped at the unchanged admin-game
  picomatch advisory GHSA-fc5q-39xc-vjpf. No audit bypass was applied.
- Independent Windows Rust gate passed, including portable contracts and clippy.

Screenshots: `.omo/evidence/linear/` in this worktree. Parent inspected list views
at 1440 and 900 pixels in light and dark, disconnected, invalid-key, draft and
fetch-error states. The new panel fits its column. Existing narrow-shell wrapping
and home-view scrolling predate this feature and remain outside its scope.
Additional captures include loading, empty/filtered-empty and related documents.

All fixtures use altered data and a temporary userData directory. The real API key
was not supplied. Live Linear account validation is therefore pending, not claimed.
Gate review, PR and CI will be recorded after delivery.

## Review corrections

The initial gate review requested changes for duplicate workflow states, account
replacement races and missing key-creation guidance. External comment 4607993821
independently identified the same account race.

A deterministic test reproduced old-account issues appearing after a replacement
key was verified. Successful credential storage now advances the request generation
again and cancels/cache-invalidates reads started during verification. Failed
reconnect still preserves the previous credentials.

The current schema's `duplicate` state is parsed and excluded with completed and
canceled issues. A mixed open/duplicate response keeps the open issue. The connect
form now points to Security & access / Personal API keys.

After these changes, type checks, all 83 service tests and the real Electron
Linear fixture passed. Updated screenshots are in the same evidence directory.

## Resumed integration with prerelease 52d46b54

Conflict resolution preserves Linear's bridge and service plus the new settings,
zoom, native context menu and Git graph. The E2E command includes all upstream
scenarios and Linear; no test was removed.

The verified settings-fixture teardown fix retains the child process before the
application connection is disposed. Linux CI uses a real unlocked GNOME keyring
inside the same D-Bus session as Electron, matching the passing Jira CI setup.
Product encryption checks and the existing audit policy are unchanged.

Windows Full gate passed with 380 Desktop tests (three existing skips), type
checks, builds and native contracts. The complete Electron suite, including
Linear, passed. Updated 1440/900 light/dark captures were inspected. Existing
narrow-shell tab wrapping and main-column scrolling remain outside this feature.
Actual Linear account validation remains unavailable without a supplied key.

## Approved linear delivery with all five outcomes

The user approved backup and force-with-lease history repair.
`backup/pr2363-approved-rewrite-abb3e214` preserves the previous Linear head.
The new direct base is merged R9 2301ca26, which also contains Jira, Notion and MCP.
Conflict resolution retains both browser-conversation and Linear handoff paths,
all IPC bridges, and the upstream settings cleanup comment.

The prepared tree stayed `e05c7199b582d0e24dd77ce17b6f6eaa4723ef9f` before and after
changing the commit parent. Windows Full passed with 486 Desktop tests (three
existing skips), native build and the complete Electron suite. Actual R9 native
conversation creation and MCP handoff also passed in this same combined app.
Updated Linear light/dark captures at 1440/900 and the integrated browser panel
were inspected. Existing narrow-shell scrolling/wrapping remains unchanged.
