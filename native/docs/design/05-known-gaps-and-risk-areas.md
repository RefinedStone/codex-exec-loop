# Known Gaps And Risk Areas

## 1. Shell Continuity Improved, But Overlays Are Still Modal
The shell is now the default landing surface, and startup diagnostics plus recent sessions moved into overlays. That is closer to a continuous shell, but the overlays are still modal and the runtime behind them is still action-scoped.

## 2. Transport Lifecycle Is Short-Lived
The adapter now reuses a shared initialized connection for startup, recent-session, and snapshot requests, but turn execution still opens its own app-server process. That means the transport is less wasteful than before, yet it still does not behave like one continuous session runtime.

## 3. Large TUI Module
`src/adapter/inbound/tui/app.rs` owns a lot of state, rendering, key handling, and shell flow logic. The branch is still understandable, but future work should avoid turning it into the permanent home for all shell behavior.

## 4. Limited Input UX
The current input model is a buffered single box. It works, but it does not yet support richer editing, focus changes, or advanced shell interactions.

## 5. Testing Gaps
The branch already has useful tests around auto follow-up and template loading, but more coverage is still needed around streamed event reduction, transport failure handling, and long-session UX regression.

## Risk Rule
Do not rewrite the streaming shell from scratch. Most of the missing work is around lifecycle, ergonomics, and decomposition, not missing protocol support.
