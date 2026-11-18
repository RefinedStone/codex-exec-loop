# R9 browser agent execution contract

## Scope and ownership

One outcome, branch, and PR. Implement the R9 browser tool set, snapshots, image
pipeline, observers, user handoff, ownership, and offline fixture measurement.
No model turns, OS control, user Chrome automation, or new browser dependency.
PR 2360 owns unrelated Desktop UI states; avoid its controls and integrate any
actual conflict explicitly. Parent owns shared bridge, main index, BrowserPanel,
renderer session/App, build configuration, integration QA, and documentation.

## Native broker contract

Entry: `akra-gateway desktop-browser-bridge`. No daemon/upstream/kernel starts.
Use existing owner-only IPC security through a reusable private IPC listener.
Create an instance-private run directory under AKRA_HOME/run and unique endpoint.
Unix directory/file/socket modes 0700/0600/0600; Windows protected owner DACL.
Tokens are random 256-bit values stored only in broker memory and private sidecar.

Sidecar JSON:
`{version:1, endpoint:string, registrationId:string, token:string}`.

Main->broker stdin newline JSON:
- `{id:string, command:"prepare"}` -> result `{registrationId,sessionFile}`.
- `{id:string, command:"bind",registrationId,threadId}` -> result `{}`.
- `{id:string, command:"revoke",registrationId}` -> result `{}`.
- `{command:"answer",requestId,result}` or
  `{command:"answer",requestId,error:{code:string,message:string}}`.

Broker->main stdout:
- `{type:"ready"}` after endpoint is listening.
- `{id,result}` or `{id,error:{code,message}}` for control requests.
- `{type:"call",requestId,registrationId,threadId,tool,arguments}`.
- `{type:"cancel",requestId}` on client disconnect, timeout, or revocation.

MCP helper->broker per-call connection:
`{kind:"call",registrationId,token,threadId,tool,arguments}`.
Response is `{result}` or `{error:{code,message}}`, then close connection.
The broker generates requestId; it never forwards a token to main.

MCP helper liveness connection:
`{kind:"watch",registrationId,token}` -> `{result:{registrationId}}`, then
remain open until revoked/broker EOF. Helper exits when this connection closes.
This stops stale MCP runtimes after renderer/window lifecycle revocation.

Bind once, reject another thread or another registration owning that thread.
Unbound registrations allow watch/initialize but not tool calls.
Validate threadId from MCP params._meta.threadId; absent/mismatch rejects calls.
One pending call per registration; reject concurrent calls as busy, no retries.
Bound frames to 8 MiB and registrations/pending connections. Normal calls have
60-second broker deadline; browser_handoff may wait up to 10 minutes.
The service allows 55 seconds for a condition wait, including its maximum
45-second condition deadline and result snapshot; the MCP transport allows
65 seconds for the broker reply.
Parent stdin EOF closes all clients, cancels calls, removes only owned files.
No supplied token/frame appears in errors or logs.

## Electron and renderer contract

BrowserAgentTransport is lazy, detached:false, windowsHide:true, piped stdio.
Main alone owns browser/CDP and registration->owner window mapping. Derive owner
from IPC sender, never a caller-supplied window ID.

Renderer receives only:
`{registrationId, sessionFile, command, args}`.
Thread config uses command/args plus env ELECTRON_RUN_AS_NODE=1 and
AKRA_BROWSER_SESSION_FILE=sessionFile; no token in config or recording.

Prepare -> thread/start with dotted akra_browser config -> bind -> thread-scoped
MCP status (threadId + toolsAndAuthOnly) -> return to caller before first turn.
Only local hosts. Preserve other MCP configuration. Release failed registrations.
Do not promise hot attach to an existing active/shared thread. Expose a new
browser conversation action. Resume only when the registration/config is
confirmed; otherwise show an explicit unavailable state rather than silently
using a different view or stale runtime.

Changing selected thread, hiding the panel, replacing the profile/view, renderer
reload, or closing the window aborts work and suspends/revokes ownership.
Human mouse/keyboard input pauses; synthetic CDP input must not self-pause.
User explicitly resumes a matching selected thread and visible view.

## Browser automation module interface

`BrowserAutomation(contents: WebContents, onPause:(reason:string)=>void)`.
`execute(tool,args,signal): Promise<BrowserToolResult>`, `dispose():void`,
`attached:boolean`.
Parent handles browser_status and browser_handoff, and delegates other tools.
`BrowserToolResult` is MCP content: text and optional JPEG image; isError optional.

Required tools:
browser_status, browser_navigate, browser_back, browser_forward, browser_reload,
browser_snapshot, browser_click, browser_type, browser_press, browser_select,
browser_scroll, browser_wait, browser_screenshot, browser_console,
browser_network_failures, browser_handoff.

Use a closed schema for each tool, no general eval/CDP tool. Arguments cannot
select owner/thread/profile/target. Only HTTP(S)/about:blank navigation.
Snapshot refs are stable within a document and never reused after navigation.
Reject detached/stale refs. Same-origin frames supported; reject cross-origin
frame actions explicitly. Bound text/list/snapshot output.
Click must detect overlays and disabled/hidden nodes. Type rejects password input.
Actions return current URL, snapshot change, new console/failure counts.
Waits use exact events/observers with bounded deadlines, not fixed polling.
Console/network buffers are bounded and do not include headers/cookies/storage.
Dismiss JS dialogs safely and report that policy; handoff is available for human
flows. Debugger detach pauses; do not fight DevTools by automatic reattachment.

Screenshot: viewport or ref crop, max long edge1280, JPEG70. Compare 64px grayscale
samples; below1% changed pixels returns text instead of duplicate image.
Optional stability wait uses 200ms sampling, max3s, cancellable. Timers are the
behavior here and tests use controlled timers. Cache resets on view/navigation.
Do not persist screenshots by default.

## UI brief

Operate mode; preserve incumbent tokens/layout. Primary action is new browser
conversation when idle, or explicit resume when paused. Priorities: current
control owner/state, last action/reason, recovery action. No decorative badges
or shell redesign. Existing toolbar/profile/preset/address controls remain.
Layout: fixed toolbar and compact agent status row above the existing native
browser view; ResizeObserver keeps the native view bounds aligned.

## Acceptance

- Native security, invalid token/thread, oversized input, EOF/disconnect/cancel,
  per-registration serialization, revocation and private sidecar cleanup tests.
- Direct MCP initialize/list/call against bundled helper and actual native broker.
- Model-free thread/start + per-thread tool-status registration proof.
- Fixed pages: forms, long lists, same-origin frame, overlay, dialog, canvas,
  delayed content and failing requests. Exercise all tools directly.
- Real Electron before/after wide/narrow light/dark captures, including native
  view and paused/handoff/error states. Parent inspects captures.
- Offline snapshot/image-size measurement. Real private-page/model comparison
  requires separate access/authorization; never claim it from fixtures.
- Planner checks, Desktop check/Vitest/e2e, fmt/diff checks, one gate reviewer,
  PR/CI/review-response workflow. Existing admin picomatch audit blocker stays
  visible; no bypass.
