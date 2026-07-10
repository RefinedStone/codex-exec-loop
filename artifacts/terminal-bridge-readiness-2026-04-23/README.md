# Terminal Bridge Readiness Evidence (Historical)

This directory preserves the terminal-bridge feasibility observations captured on 2026-04-23 at
commit `b14949a2bdf265c552654adb3435c7c08053f0ac`. It is historical research evidence, not a current
release baseline and not evidence for any required row in `docs/validation/`.

The original exact command transcript was not retained. The files show an isolated tmux server,
local bash panes, multiline send/paste, progressive output capture, interrupt status, an interactive
answer, pane recovery, and missing-target failures. Because that setup cannot be reproduced exactly
from the repository, current terminal claims must use `scripts/capture_native_validation.sh` or
`scripts/capture_native_validation.ps1` and the metadata contract in `docs/validation/README.md`.

## Sanitization

The capture text was sanitized on 2026-07-10. Local tmux names, pane IDs, pseudo-terminal paths,
and process IDs use angle-bracket placeholders. Raw terminal escape bytes are rendered as the
literal token `<ESC>` so every tracked capture remains reviewable plain text. No prompt, source
code, credential, hostname, username, or repository path is intentionally retained.

## Manifest

| File | Observation |
| --- | --- |
| `01-pane-discovery.txt` | Sanitized pane discovery fields |
| `02-discovery-pane.txt` | Ready shell prompt |
| `03-sendkeys-multiline.txt` | Multiline `send-keys` input and output |
| `04-paste-multiline.txt` | Multiline pasted input, spaces, and JSON-like text |
| `05-stream.pipe.log` | Full progressive-output transcript with escaped terminal controls |
| `06-stream-at-0.5s.log` | Partial progressive output at the early checkpoint |
| `07-stream-at-1.8s.log` | Complete progressive output at the later checkpoint |
| `08-stream-capture-pane.txt` | Plain pane view of the completed progressive command |
| `09-interrupt.txt` | Interrupt returns shell status 130 |
| `10-approval-prompt.txt` | Interactive prompt waiting for input |
| `11-approval-answer.txt` | Interactive prompt receives an answer |
| `12-recovery-anchor.txt` | Sanitized pane recovery anchor fields |
| `13-recovery-by-pane-id.txt` | Pane recovery reproduces the pasted transcript |
| `14-missing-target.txt` | Missing tmux window failure |
| `15-missing-capture.txt` | Capture against a missing tmux window fails |
| `16-no-server.txt` | Missing tmux server failure |

Do not add new captures here. Use a dated, metadata-complete record under `docs/validation/` for
current validation, or keep disposable local output under ignored `artifacts/` paths.
