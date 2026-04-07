# TODO Backlog

## Already Landed On `prerelease`
The following are intentionally not listed as TODO items because they already shipped between `origin/main` and `origin/prerelease`:

- live conversation shell
- new thread flow
- streamed turn updates
- auto follow-up loop and strategy picker
- workspace follow-up template loading
- stop rules for `AUTO_STOP` and no-file-change turns
- `Ctrl+C` back navigation
- explicit workspace path in startup diagnostics

These are the current baseline, not the final UX target. The next phase pivots from a full-screen TUI feel toward a stream-first shell.

## P0. Current-State Docs
- [x] align docs with shipped `prerelease` features before drafting new shell plans
- [x] remove obsolete "placeholder shell" assumptions from the current doc set
- [x] reframe the roadmap around a scrollback-native shell target instead of a panel-first TUI target

## P1. UX Pivot
- [ ] replace the current panel-heavy shell layout with a single flowing transcript
- [ ] keep the composer anchored at the bottom while transcript output continues above it
- [ ] reduce dependence on alternate-screen navigation for normal conversation flow
- [ ] decide which auxiliary surfaces still deserve overlays versus inline command entry

## P1. Transitional Work Already Done
- [x] move `Home` responsibilities into a shell overlay/panel
- [x] move recent sessions into a shell overlay instead of a separate screen
- [x] reduce friction between session browse and live shell entry

## P1. Runtime
- [x] introduce a shared adapter runtime for startup, session, and snapshot requests
- [x] preserve current streaming event mapping while changing transport lifecycle
- [x] add clearer reconnect/reset behavior

## P2. Stream Shell Ergonomics
- [ ] improve multiline input editing behavior
- [ ] redesign focus and status affordances for a single-column transcript plus fixed composer
- [ ] decide whether status and tool activity stay inline, move to a slim footer, or remain optional overlays
- [ ] review whether any transcript viewport controls are needed beyond terminal scrollback and lightweight in-app navigation

## P2. Auto Follow-Up
- [x] support richer template inspection and preview in the UI
- [ ] consider editable stop keyword value from the shell
- [x] make skip reasons more operator-visible when auto follow-up does not continue

## P3. Code Health
- [ ] split large shell state and reducer code into smaller units after the stream-first model settles
- [ ] add focused tests for event reduction and failure paths
- [ ] add docs or comments only where runtime behavior is otherwise hard to infer
