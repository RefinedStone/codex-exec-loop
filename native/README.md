# native

Rust TUI client prototype for `codex-exec-loop`.

This crate is the start of the `codex app-server` based client. The initial
milestone is intentionally small:

- spawn `codex app-server` over stdio
- perform `initialize`
- check account/auth state
- render a first terminal screen with startup diagnostics

Protocol shape is pinned with a checked-in schema snapshot under `schema/`.
