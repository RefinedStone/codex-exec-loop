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

## P0. Current-State Docs
- [ ] keep docs aligned with `prerelease` features before adding new shell plans
- [ ] avoid reintroducing obsolete "placeholder shell" assumptions

## P1. UX Continuity
- [ ] decide whether `Home` stays as a full screen or becomes an overlay/panel
- [ ] decide whether recent sessions should become a shell drawer instead of a separate screen
- [ ] reduce friction between session browse and live shell entry

## P1. Runtime
- [ ] evaluate a persistent app-server runtime boundary
- [ ] preserve current streaming event mapping while changing transport lifecycle
- [ ] add clearer reconnect/reset behavior

## P2. Shell Ergonomics
- [ ] improve multiline input editing behavior
- [ ] add clearer focus and status affordances
- [ ] review whether activity and transcript panel sizes should become configurable

## P2. Auto Follow-Up
- [ ] support richer template inspection and preview in the UI
- [ ] consider editable stop keyword value from the shell
- [ ] make skip reasons more operator-visible when auto follow-up does not continue

## P3. Code Health
- [ ] split large TUI state and reducer code into smaller units
- [ ] add focused tests for event reduction and failure paths
- [ ] add docs or comments only where runtime behavior is otherwise hard to infer
