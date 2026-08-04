# Long-session scroll performance — 2026-08-05

## Scope

This profile isolates the fullscreen Ratatui transcript path used after opening a session from
`:sessions`. The fixture uses the production retention ceiling of 2,048 messages, four body lines
per message, a 120×40 `TestBackend`, and repeated three-row upward scrolls. The release test binary
is executed directly so Cargo and compilation time are excluded from CPU and memory sampling.

The ignored profile is reproducible with:

```powershell
cargo test --release long_session_scroll_profile -- --ignored --nocapture
```

Windows `System.Diagnostics.Process` samples working set and private bytes while that test process
is alive. Wall-clock figures come from `Instant` inside the renderer fixture. CPU is total process
CPU time and therefore includes fixture hydration, the initial projection, and the scroll burst.

## Results

| Measurement | Before | After | Change |
|---|---:|---:|---:|
| Initial long-session frame | 134.189 ms | 28.444 ms | 78.8% lower |
| 60 scroll frames | 7,664.641 ms | 46.812 ms | 99.4% lower |
| Mean scroll frame | 127.744 ms | 0.780 ms | 163.8× faster |
| Process CPU | 8,218.750 ms | 500.000 ms | 93.9% lower |
| Peak working set | 21.418 MiB | 19.094 MiB | 10.9% lower |
| Peak private bytes | 12.988 MiB | 10.066 MiB | 22.5% lower |
| Transcript projection rebuilds | every frame | 1 | scroll reuses the document |

A 600-frame soak retained one projection, averaged 0.786 ms per frame, and peaked at 19.320 MiB
working set / 10.180 MiB private bytes. The tenfold-longer scroll burst therefore did not grow the
cache or process memory materially.

## Root cause and correction

Before this change every scroll frame reformatted all messages, split every grapheme, recalculated
all soft wraps, repeatedly summed card prefixes, and cloned the remaining transcript into a new
`Paragraph`. That made each wheel/key event proportional to the entire session and created large
short-lived allocations. The final wrapper also borrows Unicode graphemes from the formatted lines
instead of allocating one temporary `String` per grapheme, reducing the initial session frame.

`ShellRuntime` now retains exactly one width-bound presentation document. Its key covers canonical
document identity/revision, transcript revision, view/debug mode, tool expansion revision, and
terminal width. The document stores formatted lines, wrapped-row metadata, line row starts, and
projected card ranges. Scroll-only frames binary-locate the viewport, borrow only its visible lines,
and binary-locate visible cards. A key change replaces the sole cache entry; it never appends cache
entries by session, width, or revision.

`ConversationViewModel.messages` remains the only semantic transcript authority. The cache is an
immutable renderer read model and cannot mutate or outlive canonical session state.

## Evidence

- [`baseline.txt`](./baseline.txt)
- [`optimized-60.txt`](./optimized-60.txt)
- [`optimized-600.txt`](./optimized-600.txt)
