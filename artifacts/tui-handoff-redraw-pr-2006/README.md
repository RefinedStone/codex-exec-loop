# PR 2006 Handoff Redraw Evidence

This supplemental artifact set records the primitive-sensitive validation for candidate commit
`f0ca13f87e70ca99132abc219ca3618910f4442a` and tree
`f821a1ce4145926d24fc76a38dc4c9ab2a5aff1c`. The candidate fixes a stale inline viewport by
requesting one semantic frame after a successfully delivered transcript-handoff acknowledgment.

Every row ran a source build of the exact candidate. A bounded synthetic Codex app-server held one
turn open until the Help overlay was visible, then emitted one exact final answer. This makes the
delivery/ACK/redraw order deterministic; it does not claim released or production Codex activity.

The records are `supplemental-unmatched`: they do not alter the dated validation matrix totals, but
they cover all four first-class environments required for a shared `inline_terminal_adapter.rs`
primitive change.

## Environment Matrix

| Row | Environment | History modes | Resize evidence | Handoff result |
| --- | --- | --- | --- | --- |
| E1 | Windows Terminal 1.24 + WSL Ubuntu + bash | `ViewportReplay`, `StandardScrollRegion` overrides | 1249x635 px -> 650x460 px -> 1100x720 px | held behind Help; settled canary once |
| E2 | Windows Terminal 1.24 + Windows PowerShell | `ViewportReplay`, `StandardScrollRegion` overrides | 1249x635 px -> 650x460 px -> 1100x720 px | held behind Help; settled canary once |
| E3 | tmux 3.4 detached PTY + bash | `ViewportReplay`, `StandardScrollRegion` overrides | 120x30 -> 48x18 -> 120x30 cells | held behind Help; settled canary once |
| E4 | XTerm 390 via WSLg + bash | `ViewportReplay`, `StandardScrollRegion` overrides | 120x30 requested -> 676x477 px -> 120x30 requested | held behind Help; settled canary once |

All rows also show a live turn before release, a stable restored viewport, `prompt: response held
while the dialog is open`, a post-Esc `prompt: session ready`, and a clean exit. The raw E1/E3/E4
typescript files are omitted because they contain control bytes and local absolute paths; their
hashes and exit footers are recorded in the environment stamps.

## Evidence Index

- `e1-windows-terminal-wsl/`: E1 screenshots, stamp, and scenario results
- `e2-windows-terminal-powershell/`: E2 screenshots, stamp, and scenario results
- `e3-tmux/`: E3 plain-text frames, stamp, and scenario results
- `e4-direct-xterm/`: E4 screenshots, stamp, and scenario results
- `automated-proof.txt`: named regression proof for the full methodology checklist
- `candidate-integrity.txt`: candidate, binary, and fixture provenance

## Scope And Limitations

- These captures exercise the directly changed `ViewportReplay` handoff path in real terminal
  programs. The also-changed parallel `HostScrollback` receipt/redraw path is covered by named
  frame-recorder proofs; unchanged newline-fallback, reset, identity-switch, and focus-reacquire
  branches use the remaining named automated proofs.
- The app-server fixture emits no credentials, network traffic, tool output, or user data.
- Startup frames and raw transcripts are excluded from the checked-in set. Selected evidence has
  no credentials, UUIDs, PTY identifiers, repository paths, or authentication state.
- No admin, CLI, Telegram, persistence, recovery, performance, or comparative product claim
  follows from these captures.

Review status: pass. Independent reviewer `Codex /root/handoff_evidence_review` found no remaining
artifact, integrity, scenario, or privacy finding after the HostScrollback scope wording was fixed.
