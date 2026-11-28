# #2965 Windows Electron acceptance

**Result: partial pass.** The merged conversation menu opens the existing task editor. Long code still overflows the conversation, the menu sequence moves the reading position, and closing the editor with Escape leaves focus on `BODY`. This submission changes no product code and does not close #2965.

## Environment and evidence

- Tested prerelease commit: `3110a9e9cfb3eaa83bf32a06af640e4c207dffba`, containing PR #2967.
- Windows x64 `10.0.26200`, Electron `45.2.3`, Node `24.13.0`. A hidden native Electron window used an isolated home, `userData`, and a unique named-pipe fake Gateway. The Skills response was synthetic and empty.
- Regular: 1200×800 content DIP, 1920×1080 display DIP, device scale 1, body zoom 100%, right panel closed. PNGs: 1200×800.
- Compact: requested 1024×700 content DIP, measured 1024×701; Electron reported 1228×691 display DIP under `--force-device-scale-factor=1.25`. The renderer reported DPR 1.25, body zoom 125%, and the Skills tab open at 466.24 CSS px. PNGs: 1280×877. This is a Chromium/Electron override, not a change to Windows system DPI.
- Measurements: [regular](regular.json), [compact](compact.json). Original `capturePage` PNGs from both runs are in `C:\Users\dorani\AppData\Local\Temp\akra-2965-windows-st01a21e78-publish`: `{regular,compact}-{before,menu,editor,after}.png`. `before` is attached and unselected; `after` is after editor Escape. The recorder can regenerate all eight files. PNGs are not committed.

## Observed

| Check | Result |
| --- | --- |
| Persistent task action or reserved strip | Absent in both runs, with attachment pending and complete. The regular pending and attached snapshots match. Compact transcript height changed from 499 to 570 CSS px, while the measured conversation and composer boxes matched. Attachment is not isolated from other renderer settling in this comparison. |
| Selection, change and clear | Conversation, transcript column, composer, scroll offsets and panel measurements match the pre-selection snapshot in both runs. |
| Existing menu and task editor | Empty right-click selection offered no task action. `Shift+F10` on valid text focused the task action. Copy and quote were present. Right-click opened the action; clearing live selection before activation still gave the editor exactly two paragraphs and the two-line code block, without copy controls. Thread and turn IDs matched the fixture. Focus was inside the modal. |
| Long code | Conversation `clientWidth/scrollWidth` was 898/1934 CSS px regular and 204/1899 compact. The code `pre` measured 1875/1875 in both. Overflow was not confined to the code. It was already visible before invoking the task action; these runs do not attribute it to PR #2967. |
| Scroll and focus on Escape | Menu Escape restored focus to the conversation, but the keyboard menu sequence moved regular `scrollLeft` from 0 to 59. Compact `scrollLeft` moved from 0 to 1682.56 and `scrollTop` from 121.60 to 215.68. These offsets persisted after the editor closed. Editor Escape left focus on `BODY` in both cases. |
| Draft, right panel and side effects | The composer draft, panel box and selected tab were unchanged. The recorder observed zero task mutations and zero `turn/start` or `turn/steer` requests. The separate stock Windows task-authoring test passed explicit save, lost-reply recovery, revision checks and active-task protection against its disposable fake. |

Not verified here: OS-level DPI or multiple physical displays, a documented minimum main-window size, manual clipboard or quote execution, the close button, the selection size limit, cross-panel selection, workspace or thread switching, long command-log overflow, or a packaged installer. The compact size is representative, not a claimed supported minimum.

## Reproduce on Windows

Use Node 24 on the evidence branch. Run from the repository root in PowerShell:

```powershell
npm --prefix desktop ci
npm --prefix desktop run build
Push-Location desktop
$sha = git rev-parse HEAD
$out = Join-Path $env:TEMP ("akra-2965-" + [guid]::NewGuid())
node ../docs/validation/2965-windows/capture.mjs regular $out $sha
node ../docs/validation/2965-windows/capture.mjs compact $out $sha
node --experimental-strip-types test/e2e/task-authoring.mjs
Pop-Location
```

The recorder exits successfully when it completes the scenario and writes the receipt. Inspect `checks` for failed acceptance conditions; `false` is deliberately retained, not treated as a harness crash. `stages` contains the measured DOM geometry and focus states.
