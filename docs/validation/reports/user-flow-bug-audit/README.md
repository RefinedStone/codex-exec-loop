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
| 3 | 세션 브라우저 검색 변경 시 선택 무결성 | 세션을 선택한 상태에서 검색어를 커밋해 해당 세션이 결과에서 사라질 때, stale 선택이 남아 Enter가 잘못된 세션을 여는지 검증. ID 기반 선택 복원(`resolve_selected_index`)과 커밋 후 `sync_session_browser_selection` 재투영, 회귀 테스트(`search_change_drops_stale_selection_instead_of_attaching_wrong_session`)로 확인. | 버그 없음 — 검색 커밋 후 선택이 보이는 첫 행(alpha)으로 재투영됨 | 검증 완료 |
| 4 | 큐 오버레이 선택 이동·삭제 확인 가드 | 작업 목록이 비었거나 선택 id가 목록에서 사라진 상태에서 j/k 이동이 패닉하지 않고, 삭제 확인(x→Enter)이 정확한 authority intent에만 묶여 있는지 검증. 기존 테스트(`remove_confirmation_is_bound_to_the_exact_authority_intent`, `authority_refresh_and_selection_repair_disarm_remove_confirmation`)와 신규 엣지 테스트(`move_selection_never_panics_on_empty_or_unknown_task_lists`)로 확인. | 버그 없음 — 빈 목록/미지정 id에서 안전 no-op, intent 불일치 시 disarm 유지 | 검증 완료 |
| 5 | 병렬 peek 대화 미리보기 스크롤 클램프 | Home(oldest) 이동 시 `usize::MAX` 스크롤 값이 렌더에서 u16으로 변환되며 wrap하지 않고 0으로 클램프되는지, 빈 대화 미리보기에서도 안전한지 검증. `inline_preview_scroll_offset` saturating 연산과 확장 테스트로 확인. | 버그 없음 — usize::MAX/빈 preview 모두 offset 0 클램프 확인 | 검증 완료 |
| 6 | 승인 오버레이 결정 잠금 후 키 처리 | Y/N/Esc 제출로 결정이 잠긴 뒤에도 상세 스크롤(Up/Down/Page)은 계속 동작하고, 재입력된 y/n/Esc가 잠긴 결정을 덮어쓰지 않으며 Ctrl-C만 stop으로 이어지는지 검증. `handle_approval_overlay_key` 분기와 확장 테스트(`approval_overlay_consumes_all_input_and_routes_only_explicit_decisions`)로 확인. | 버그 없음 — 결정 잠금 중 스크롤 유지, 결정 불변 확인 | 검증 완료 |
