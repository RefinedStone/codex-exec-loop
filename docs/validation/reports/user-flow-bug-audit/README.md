# User Flow Bug Audit Log

이 문서는 유저 플로우상 버그를 일으킬 수 있는 요소를 기능 단위로 하나씩 검증하고,
그 결과(검증 완료 또는 수정 완료)를 순서대로 기록한다.

- 대상: codex-exec-loop 네이티브 TUI/앱 플로우
- 규칙: 기능 단위 1건 = 로그 항목 1개 = 커밋 1개
- 상태 표기: 검증 완료(버그 없음) / 수정 완료(버그 발견 후 수정)

| # | 기능 | 검증 범위 | 결과 | 상태 |
|---|------|-----------|------|------|
| 1 | TUI 종료 확인 오버레이 키 우선순위 | 종료 확인이 보이는 동안 Esc/Enter/오버레이 내비게이션 키가 도움말 스크롤·프롬프트로 새지 않고 확인 핸들러가 먼저 소비하는지 검증. `shell_runtime::handle_key_press`가 exit confirmation을 overlay/steer/composer보다 먼저 호출하는 구조와 회귀 테스트(`exit_confirmation_owns_keys_before_overlay_navigation_and_prompt`)로 확인. | 버그 없음 — N/Esc로 취소 시 오버레이 상태와 help_scroll_offset 보존 확인 | 검증 완료 |
| 2 | 프롬프트 컴포저 멀티바이트(그래핌) 안전성 | 한글+이모지 프롬프트에서 Backspace/Cursor 이동이 그래핌 경계를 벗어나 패닉하거나 문자열을 깨뜨리는지 검증. `conversation_input` 리듀서의 grapheme_indices 기반 커서/삭제 경로와 회귀 테스트(`backspace_and_cursor_moves_never_split_multibyte_graphemes`)로 확인. | 버그 없음 — 이모지 1회 삭제, 커서가 글 앞 바운더리(3바이트)에 착지 확인 | 검증 완료 |
