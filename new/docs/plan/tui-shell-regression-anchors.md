# TUI Shell Regression Anchors

> 상태: 과거 regression anchor 문서다. 현재 구현 판정과 다음 작업은
> [repository-wide-rebuild-roadmap.md](./repository-wide-rebuild-roadmap.md)를 따른다.
> 이 문서는 테스트 참고 자료이며 완료 판정 authority가 아니다.

## 목적

이 문서는 `TUI-00C`의 산출물이다. `TUI-00A`와 `TUI-00B`가 state/background message
inventory를 고정했다면, 이 문서는 conversation/automation split 전에 반드시 유지해야 할
shell input/rendering regression anchor를 묶는다.

## Regression Matrix

| Contract | Test anchor |
| --- | --- |
| plain character input은 modifier가 비어 있을 때만 prompt buffer로 들어간다 | `plain_character_input_uses_empty_modifier_check` |
| Supersession loading 중에는 overlay가 prompt input을 막는다 | `supersession_overlay_blocks_prompt_input_while_loading` |
| Supersession loading 완료 후에는 overlay를 유지한 채 prompt input이 가능하다 | `supersession_overlay_allows_prompt_input_after_loading_finishes` |
| Supersession ready board에서 Space/Enter는 prompt edit/submit으로 내려간다 | `supersession_overlay_allows_space_and_enter_prompt_submit_after_loading_finishes` |
| Supersession MUD navigation은 UI selection만 바꾸고 supervisor/domain projection을 mutate하지 않는다 | `supersession_mud_navigation_changes_only_ui_selection_state` |
| Supervisor projection refresh는 overlay focus와 MUD selection을 보존한다 | `parallel_projection_refresh_preserves_supersession_overlay_focus_and_selection` |
| inline command palette selection은 prompt submit과 shell command execution을 구분한다 | `enter_executes_selected_inline_command_palette_item`, `escape_dismisses_inline_command_palette_without_clearing_buffer`, `up_wraps_inline_command_palette_selection` |
| shell editing shortcut은 prompt buffer만 바꾸고 overlay/session state를 건드리지 않는다 | `ctrl_u_clears_buffered_input`, `ctrl_w_deletes_previous_buffered_word` |
| resize/focus는 rendering concern이며 committed transcript와 input buffer를 mutate하지 않는다 | `resize_event_leaves_transcript_state_unchanged`, `focus_lost_blocks_draw_until_focus_returns` |
| inline rendering은 prompt cursor와 compact tail을 projection rows 아래에 보존한다 | `inline_render_positions_cursor_on_empty_prompt_line`, `inline_supersession_keeps_buffered_prompt_visible_in_compact_tail` |
| planning editor cursor/dirty/close guard는 UI state로 유지된다 | `planning_draft_editor_ui` cursor/close-risk tests |
| stale/duplicate post-turn background result는 current conversation/projection을 덮지 않는다 | `stale_post_turn_evaluation_background_message_is_ignored`, `duplicate_post_turn_evaluation_for_same_turn_is_ignored` |

## Migration Guard

`TUI-01`은 위 test anchor를 약화시키면 안 된다.

- overlay focus와 prompt input routing을 conversation automation split의 부수 효과로 바꾸지 않는다.
- projection refresh는 UI selection/cursor를 reset하지 않는다.
- prompt lock과 readiness copy는 rendering state로 남기되, 실제 execution 가능 여부는 application/runtime event를 따른다.
- background message가 직접 projection internals를 재계산하는 방향으로 test를 통과시키지 않는다.
