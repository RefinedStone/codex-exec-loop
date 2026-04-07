# Known Gaps And Risk Areas

## 1. Shell Continuity Improved, But Overlays Are Still Modal
The shell is now the default landing surface, and startup diagnostics plus recent sessions moved into overlays. That is closer to a continuous shell, but the overlays are still modal and can still interrupt the shell rhythm even though the adapter runtime is no longer fully action-scoped.

## 2. Transport Lifecycle Is Better, But Still Not Fully Continuous
The adapter now reuses a shared initialized connection for startup, recent-session, snapshot, and turn execution, and reconnect/reset events are surfaced as shell-visible warnings. Concurrent request actions still need an isolated fallback connection while a turn stream is active, though, so the transport is closer to one session runtime without being fully unified.

## 3. Large TUI Module
`src/adapter/inbound/tui/app.rs` owns a lot of state, rendering, key handling, and shell flow logic. The branch is still understandable, but future work should avoid turning it into the permanent home for all shell behavior.

## 4. Limited Input UX
The current input model is a buffered single box. It works, but it does not yet support richer editing, focus changes, or advanced shell interactions.

## 5. Testing Gaps
The branch already has useful tests around auto follow-up and template loading, but more coverage is still needed around streamed event reduction, transport failure handling, and long-session UX regression.

## Risk Rule
Do not rewrite the streaming shell from scratch. Most of the missing work is around lifecycle, ergonomics, and decomposition, not missing protocol support.
