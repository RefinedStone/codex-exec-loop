# R9 browser control verification

## Scope

Desktop owns one embedded browser per window. A private native broker binds each
registration to one thread. The bundled stdio MCP helper exposes 16 closed tools.
No general JavaScript, CDP, OS automation, or user Chrome attachment is exposed.
The renderer prepares a new local browser conversation and verifies its tools.
Existing conversations are not silently attached after a cold restart.

## Verification ledger

- Native broker: seven Windows tests passed, including pipe and sidecar ACLs,
  authentication, thread isolation, cancellation, revocation, limits, EOF cleanup.
- `node scripts/agent-plan.mjs`: selected Full Windows gate.
- `powershell -File scripts/check_native_pr.ps1 -Mode Full`: failed at the
  unchanged admin game's picomatch audit, GHSA-fc5q-39xc-vjpf. No bypass applied.
- `powershell -File scripts/check_native_pr.ps1 -Mode Rust`: passed, exit 0.
- Desktop `npm run check`: passed.
- Desktop `npx vitest run`: 310 passed, three existing skips, 33 files.
- After correcting status request to typed `detail: "toolsAndAuthOnly"`:
  type checks and all 26 session-input tests passed.
- Desktop `npm run e2e`: passed, including existing browser profiles, approvals,
  terminal, work panels, document context and remote-host behavior.
- `node test/e2e/browserAutomation.mjs`: passed against real Electron. Covers
  navigate/back/forward/reload, snapshot refs, type, key, select, click, condition
  wait, same-origin frame input, password rejection, JPEG screenshot and duplicate
  suppression, scroll, dialog dismissal, console/failure buffers, human input.
- `AKRA_GATEWAY_BIN=<built gateway> node test/e2e/browserBridge.mjs`: passed.
  Actual native pipe and bundled helper; initialize, 16 schemas, foreign-thread
  rejection, call/reply, revoke, helper exit and sidecar deletion.
- With `AKRA_CODEX_ENTRY=<installed codex/bin/codex.js>` the same script also
  passed actual Codex initialize, ephemeral thread/start and thread-scoped status:
  16 tools, runtime `connected`. No model turn was sent.
- Browser profile test now waits for native view creation instead of fixed
  delays. Its complete flow passed after that change.

## Manual UI evidence

Parent captured and inspected composited native windows at 1440 and 900 pixels
in light and dark themes. The new control row stays within the browser panel.
Browser content is visible, not the blank slot from renderer-only screenshots.
Actual native registration, pause and explicit resume passed in the running UI.

Evidence lives in the integration checkout at `.omo/evidence/r9/`:
`before/before-window-{1440,900}-{light,dark}.png`,
`after/after-window-{1440,900}-{light,dark}.png`, `after/paused.png`.
The reproducible capture script is `baseline.mjs` in that directory.

Existing narrow-shell navigation wrapping and conversation horizontal scrolling
remain visible. They predate this control row and belong to other UI work.

## Corrections found through real execution

Offline fixture measurement: snapshot 1,105 UTF-8 bytes; JPEG 15,656 bytes at
984 by 735 pixels; raw viewport PNG 14,026 bytes. The sparse fixture does not show
a JPEG size reduction. The unchanged second screenshot returned no image.
These are payload measurements, not model-token or private-page benchmarks.

- CDP click needs pointer movement before press/release. Without it the command
  returned successfully but the page received no mouse event. Real Electron
  verification and a strict event-order unit assertion cover the fix.
- Codex status uses `detail: "toolsAndAuthOnly"`, not a boolean property.
  The request now satisfies the generated parameter type.
- Process cleanup waits for `close`, not just `exit`, before deleting the
  isolated Codex home on Windows.

## Gate review corrections

The initial review of 9df8d4d8 requested changes. The working revision adds
interactive snapshots, default diffs, actionable refs, unnamed-group folding,
interactive fallback at the size cap, submit, and a 500 ms network-idle interval.
Network timing tests use controlled clocks. Snapshot results carry a sanitized URL
and `contentKind: "untrusted_page_data"`.

It also rejects inherited disabled controls and active shadow-root passwords.
Element captures intersect each ancestor frame's viewport. Real Electron checks
cover disabled fieldsets, normal/shadow password input, overlay rejection, a
600-pixel input in a 200-pixel frame, and actionable refs after 1,500 list entries.

The real submission fixture exposed input loss before the target received pointer
movement. Type now moves the pointer to the validated element before inserting
text and submitting. Window focus changes and focus emulation were not needed and
were removed. The fixture blurs the native window and waits for the actual form
submission event, rather than treating dispatch as proof.

Offline token measurement uses tiktoken 0.14.0 with `o200k_base`, installed only in
an external temporary directory. The public fixture's saved variants measured:
raw AX JSON 5,516 tokens; compact text 288; interactive text 112; unchanged diff 0.
This is not a Playwright/model comparison. Reproduce with
`AKRA_BROWSER_SNAPSHOT_OUTPUT=<path> node test/e2e/browserAutomation.mjs`, then
run the integration evidence script `.omo/evidence/r9/count_tokens.py <path>`.

## Remaining delivery checks

One dedicated-worktree gate review and PR CI are still required.
Unix runtime validation belongs to CI; this workstation is Windows.
No private-page/model token benchmark was run.

The corrected native UI capture includes browser content in paused, handoff and
error states. The new-browser-conversation button also passed with a fake Gateway:
it sent thread/start and thread-scoped status discovery, then showed active control,
without sending a turn. Actual Codex tool discovery was verified separately.
The delayed-content fixture installs the real observer before adding its text.

PR 2362 CI run 44205212792 failed only the unchanged picomatch admin audit and its
dependent Fast/CI gates. Rust tests, lint/architecture and portable checks passed.

External comments 4607529033 and 4607529038 found independent service defects.
The wait boundary now allows 55 seconds, broker 60, and helper 65, preserving
the advertised 45-second condition wait plus result processing. A controlled-clock
test proves a 45-second wait can succeed at 35 seconds. A closed broker remains
unavailable through hide/show and rejects resume. Desktop type checks and all ten
focused service/transport tests passed after these corrections.

The second gate pass approved the timeout/lifecycle delta and retained two
findings. Keyboard focus now uses CDP node inspection to reject a closed shadow
host and direct the user to handoff. The real closed-root password remained empty.
Repeated lists retain ten preview items per list and report `omittedListItems`;
interactive descendants stay addressable. The 1,500-item fixture reports 1,490
omitted items and retains its final action. Type checks, 33 engine tests and the
complete real Electron automation fixture passed after this delta.

## Resumed integration with prerelease 52d46b54

The integration retains settings, zoom, native context menus, Git graph and the
foreign-writer state. Starting a browser conversation now uses `visit` with its
thread ID, matching the navigation-history contract.

Real Electron testing reproduced acknowledged CDP key events without a submit.
The explicit rawKeyDown / char / keyUp sequence fixed the unchanged fixture;
synthetic-input matching maps rawKeyDown to Electron's keyDown. Protected inputs,
human interruption, frame crops, list compaction and the original deadlines remain.
Temporary diagnostic wrappers were removed. See `r9-debug.md` for the experiments.

Final Full Windows gate passed, including 361 Desktop tests (three existing skips),
types, build and native contracts. The complete general Electron suite passed.
The real automation fixture and native broker/MCP round trip passed separately.
Updated native panel captures at 1440/900 in light/dark were inspected under the
integration checkout's `.omo/evidence/r9/resumed`. New conversation, pause/resume,
MCP handoff and error states passed without a model turn. Existing narrow shell
tab wrapping and conversation scrolling remain outside the R9 change.

## Approved linear delivery after Jira merge

The user approved backing up and rewriting the published feature branches.
`backup/pr2362-approved-rewrite-2f26f2ff` preserves the original R9 head.
The current base is fae6eaed, which includes merged Jira and upstream G2/search
changes. The index tree remained `821dfb8aa3c17e9e81b601ab132e5d7c5fc04b1f`
before and after changing the commit parent.

Windows Full passed with 403 Desktop tests (three existing skips). The native
gateway build and complete Electron suite, including Jira, passed. Actual browser
conversation creation and native MCP handoff passed without model turns. Updated
1440/900 light/dark captures were inspected in integration
`.omo/evidence/r9/linear-delivery`. The existing automation engine regression
evidence remains valid; that implementation was not changed in this integration.
