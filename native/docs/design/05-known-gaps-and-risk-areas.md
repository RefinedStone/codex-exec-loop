# Known Gaps And Risk Areas

## 1. Screen Model Still Breaks Shell Continuity
The shell is live, but the app still routes through `Home` and `SessionList` as full screens. That keeps startup diagnostics and browsing easy, but it reduces the "always in the shell" feeling.

## 2. Transport Lifecycle Is Short-Lived
Each action opens its own app-server process. This keeps the code simple, but it makes the shell feel less like one continuous session runtime and more like repeated transport jobs.

## 3. Large TUI Module
`src/adapter/inbound/tui/app.rs` owns a lot of state, rendering, key handling, and shell flow logic. The branch is still understandable, but future work should avoid turning it into the permanent home for all shell behavior.

## 4. Limited Input UX
The current input model is a buffered single box. It works, but it does not yet support richer editing, focus changes, or advanced shell interactions.

## 5. Testing Gaps
The branch already has useful tests around auto follow-up and template loading, but more coverage is still needed around streamed event reduction, transport failure handling, and long-session UX regression.

## Risk Rule
Do not rewrite the streaming shell from scratch. Most of the missing work is around lifecycle, ergonomics, and decomposition, not missing protocol support.

