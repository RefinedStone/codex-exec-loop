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
| 7 | 턴 스티어 확인 모달 키 소유권과 stale 확인 | Tab으로 띄운 스티어 확인 모달이 활성인 동안 일반 입력이 컴포저로 새지 않고 Esc/Ctrl-C가 초안을 보존한 채 모달만 닫는지, 그리고 모달 표시 후 백그라운드에서 턴이 끝난 뒤 Enter를 눌러도 잘못된 턴에 steer를 보내지 않고 초안을 유지하는지 검증. `handle_turn_steer_confirmation_key`의 소비 규약과 `confirm_turn_steer` exactness 가드, 신규 회귀 테스트(`steer_confirmation_modal_owns_keys_and_esc_preserves_draft`, `stale_steering_confirmation_enter_keeps_draft_without_dispatching`)로 확인. | 버그 없음 — 모달 중 입력 비유입, Ctrl-C/Esc 후 draft 유지, stale Enter는 dispatch 없이 draft 보존 확인 | 검증 완료 |
| 8 | 세션 rename 편집기 빈 이름 커밋 가드 | 이름을 전부 지우거나 공백만 남긴 상태에서 Enter를 누르면 rename 명령이 Core로 나가지 않고 편집기가 피드백과 함께 유지되는지 검증. `prepare_rename_request`의 trim+empty 가드와 신규 회귀 테스트(`rename_commit_with_blank_name_stays_in_editor_without_dispatch`)로 확인. | 버그 없음 — 공백 이름 커밋 시 editor 유지 + empty feedback, pending admission 없음 | 검증 완료 |
| 9 | 인라인 셸 명령 팔레트 미지 명령·빈 제안 안전성 | 알 수 없는 토큰(예: `:xyz`)을 입력해 제안이 0개인 팔레트가 활성 상태로 남을 때 이동/선택/Esc가 패닉 없이 동작하고, Enter 수락이 선택 없이는 no-op로 초안과 오버레이를 보존하는지 검증. `sync_to_input` 빈 목록 클램프, `move_selection` inactive 가드, `accept_inline_command_palette_selection` None 조기 반환을 신규 테스트(`palette_state_survives_unknown_prefix_without_selection`, `palette_acceptance_without_selection_keeps_prompt_and_overlay_untouched`)로 확인. | 버그 없음 — 빈 제안에서 이동/수락 inert, 초안 `:xyz` 보존, 반복 dismiss 안전 | 검증 완료 |
| 10 | 시작 진단 오버레이 End 스크롤 클램프 | End 키가 `usize::MAX` 점프 표식을 저장한 뒤 다음 프레임에서 u16 렌더 변환 시 wrap하지 않고 콘텐츠 높이로 클램프되어 되돌려 쓰기(receipt)가 적용되는지, stale 프레임 거절 시 표식이 키 처리로 새지 않고 유지되는지 검증. `fullscreen_frame_model`의 min 클램프·receipt write-back을 확장 테스트(`startup_end_scroll_clamps_to_frame_and_survives_stale_receipt`)로 확인. 기존 닫기 리셋 테스트도 함께 통과. | 버그 없음 — usize::MAX가 렌더 offset 0(빈 경고)으로 클램프되고 receipt로 정규화됨 | 검증 완료 |
