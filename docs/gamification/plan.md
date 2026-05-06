# AKRA Dashboard TUI Redesign

## Summary
- `docs/gamification/img.png` 기준으로 TUI 전체를 고정 패널형 게임 대시보드로 재구성한다.
- 첫 출시는 `CODEX_EXEC_LOOP_TUI_SKIN=dashboard` flag 뒤에 둔다. unset/unknown 값은 기존 inline UI 유지.
- 입력 정책은 `Prompt Primary`: 일반 문자, Space, Backspace, Enter는 항상 하단 프롬프트가 우선 소유한다. 패널은 읽기/선택 UI이며 프롬프트 제출을 막지 않는다.
- XP, rank, badge, reward 같은 가짜 게임 수치는 넣지 않는다. 패널명과 분위기만 MUD/quest 스타일로 가져가고, 숫자는 실제 상태에서만 파생한다.

## Implemented
- Added `ShellUiSkin { Inline, Dashboard }` and `CODEX_EXEC_LOOP_TUI_SKIN=dashboard`; blank, unset, and unknown values keep the legacy inline renderer.
- Added adapter-local `DashboardUiState` for dashboard panel focus/row selection.
- Added `shell_rendering/dashboard.rs` with masthead, fixed panel grid, bordered `Prompt Primary`, and bottom key/status bar.
- Added dashboard panel packs for home, startup, sessions, supersession/parallel, queue, planning, task intake, directions, and help overlays.
- Added narrow layout behavior that stacks panels while keeping the bottom prompt and key bar fixed.
- Kept domain/application APIs unchanged.
- Kept `SupersessionMudUiState` as a compatibility projection for existing inline MUD rendering and used dashboard navigation state separately.

## Parallel Dashboard Markers
- Supersession/Parallel renders the concept markers from the image-inspired plan:
  - `Agent Tavern`
  - `Distributor`
  - `Worktree Pool`
  - `Realm Map`
  - `Quest Log`
  - `Event Feed`
  - `System Status`

## Input Policy
- Dashboard Supersession loading no longer steals normal prompt input.
- Ordinary characters, Space, Backspace, and Enter continue through the prompt path in dashboard mode.
- Tab, Shift+Tab, and arrows update only `DashboardUiState`.
- Existing modal/editor/search surfaces keep their existing input priority.
- Legacy inline Supersession loading lock remains unchanged and covered by regression tests.

## Test Coverage Added
- Dashboard flag on/off rendering contract.
- Dashboard prompt visibility at 80x24, 120x36, and 160x48 with Korean input.
- Supersession dashboard required marker rendering.
- Guard that fake game-stat copy (`XP`, `Rank`, `Badge`, `Reward`) is absent.
- Dashboard Supersession loading prompt-primary input.
- Dashboard navigation mutation boundary: dashboard state changes without mutating supervisor/domain state or `SupersessionMudUiState`.

## Remaining Delivery Gates
- Run targeted regression tests:
  - `cargo test inline_terminal_adapter`
  - `cargo test inline_supersession`
  - `cargo test shell_rendering_contract`
  - `cargo test shell_runtime::tests::input`
  - `cargo test supersession::`
- Run repository hygiene:
  - `git diff --check`
  - `cargo fmt --check`
  - `cargo clippy --all-targets --all-features -D warnings`
- Commit, push, open PR, rebase-merge into `prerelease`, and clean up the finished worktree.
