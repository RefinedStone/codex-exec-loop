# Approval review operability

## Outcome

Eventually make approval review actionable from within the shell.

## Why this direction is paused

Current product state still depends on upstream app-server protocol and schema support before interactive approve or deny actions become a reliable slice.

## Long-horizon plan

- unify review visibility, risk levels, and next actions with the rest of the shell state model
- support in-shell approve or deny actions when upstream support is ready
- reduce out-of-band manual approval handling

## Activation gate

- upstream protocol and schema expose stable approve or deny actions
- current operator-facing shell, queue, and planning flows are already coherent enough to absorb a new interactive path

## Relevant inputs

- `docs/design/01-current-product-state.md`
- `src/adapter/inbound/tui/conversation_text.rs`
- `src/adapter/outbound/codex_app_server_adapter/protocol.rs`

## Until activated

- keep this direction paused
- allow only lightweight visibility or copy groundwork if it supports current-state clarity
