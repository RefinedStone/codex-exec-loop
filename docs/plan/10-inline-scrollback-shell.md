# Inline Scrollback Shell

This file is the current contract for inline mode on `prerelease`.

## Durable Facts

- inline mode is the only frontend
- host terminal scrollback is the canonical history surface
- viewport replay is an explicit diagnostic fallback, not an automatic OS or terminal default
- committed transcript history is appended into scrollback separately from live-tail updates
- the live tail is the only place that owns the active prompt, transient streaming text, and compact notices
- `thread_id` and `turn_id` come from `codex app-server` and stay out of routine operator copy

## Terminal Ownership

- `NativeTuiApp` owns transcript, live-turn, prompt, and overlay state only
- terminal viewport bookkeeping lives below the app in the inline terminal adapter
- the inline terminal adapter owns viewport size, cursor recovery, pending history flush, and back-buffer invalidation policy
- one inline draw transaction runs in this order: autoresize or cursor repair, pending history flush, viewport clear or invalidate, then live frame draw
- raw scroll, newline fallback insertion, and viewport clear always invalidate the adapter's back-buffer trust before the next frame draw

## Prompt And Streaming

- the active prompt always owns a visible cursor
- buffered input stays intact while a turn is streaming
- operator-visible assistant text changes before turn completion
- once the turn completes, final assistant text moves into normal scrollback history

## Layout And Status

- blank startup reads as startup context, conversation placeholder, and prompt
- once committed history exists, the live tail starts at the first visible row of its viewport so prompt and notices stay attached to the latest line
- diagnostics, sessions, automation controls, planning, and queue inspections reuse the same inline shell surface
- routine status stays compact and flow-oriented instead of acting like a permanent heavy footer
