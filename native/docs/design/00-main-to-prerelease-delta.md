# Main To Prerelease Delta

The native docs should be read from the `prerelease` baseline, not from `main`.

## What Changed Materially On `prerelease`
- the app now opens into a live conversation shell instead of a dashboard-first placeholder
- startup diagnostics, recent sessions, and template inspection moved into shell-adjacent overlays
- thread creation, thread resume, snapshot loading, and streamed turn execution are all wired to the app-server flow
- auto follow-up, workspace templates, and stop rules became first-class product features
- the large inbound TUI surface was split into more focused modules, leaving `app.rs` closer to a composition root

## Documentation Rule
- assume the shell, streaming turn flow, and auto follow-up already exist
- do not write new docs as if the native client still needs the original shell bootstrap work
- keep future comparisons focused on stable runtime and UX direction, not on the old `main` prototype
