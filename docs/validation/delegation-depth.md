# Delegation depth admission (issue #2880)

- Direct submission starts at depth 0 with no parent.
- A child submission includes the optional parent_callback object (parent job ID and its existing callback capability). The SQLite transaction verifies the active, unexpired parent and the capability digest before request-key replay or insertion.
- The maximum depth is 1. An authenticated request from a depth-1 parent returns depth_exceeded before creating a row or consuming the job budget. Invalid capabilities return invalid_parent_callback.
- The child stores only parent_id and depth, not the token. State transitions cannot modify lineage. Retrying a safe unsent job preserves both fields; old records without lineage decode as depth 0.
- Request keys cannot be reused across different lineages. Child submission replay still requires a valid active parent. The existing trusted local direct-submit API remains available; it does not infer ancestry from caller names.
- The regression uses a local fixture ledger and synthetic dispatch states, not a real account or model turn.

Run: cargo test --locked --lib sqlite_delegation_store
