# TUI Shell State Inventory

## 목적

이 문서는 `TUI-00A`의 산출물이다. 기준 architecture는
[../architecture/tui-application-boundary-architecture.md](../architecture/tui-application-boundary-architecture.md)
이며, 목적은 `NativeTuiApp`에 모여 있는 state를 구조 변경 전에 분류하는 것이다.

이 inventory는 현재 구조를 정당화하지 않는다. 다음 slice가 어떤 state를 TUI에
남기고, 어떤 state를 application event/projection으로 이동해야 하는지 결정하기 위한
작업 표다.

## 분류 기준

| 분류 | 의미 |
| --- | --- |
| UI-only | terminal focus, input buffer, overlay step, cursor, selection처럼 TUI 수명에만 묶인다. |
| Application Projection Cache | application service가 계산한 projection/result를 TUI가 표시하기 위해 들고 있다. |
| Runtime Bridge | reducer effect와 background message를 연결하기 위한 임시 실행/correlation state다. 최종 소유자는 application runtime일 수 있다. |
| Service Wiring | TUI entrypoint가 조립한 service/port handle이다. business state가 아니다. |

## NativeTuiApp Field Inventory

| Field | 분류 | 현재 책임 | 이동 방향 |
| --- | --- | --- | --- |
| `shell_overlay` | UI-only | 현재 focus owner와 visible overlay를 고른다 | TUI controller에 유지 |
| `exit_confirmation_state` | UI-only | exit guard 표시와 confirm/cancel 상태 | TUI controller에 유지 |
| `startup_state` | Application Projection Cache | startup diagnostics load result와 loading/error 표시 | projection cache로 유지 |
| `session_state` | Application Projection Cache | session catalog load result와 loading/error 표시 | projection cache로 유지 |
| `parallel_mode_enabled` | Runtime Bridge | TUI가 parallel mode surface를 켰는지 추적 | application command result projection으로 축소 검토 |
| `parallel_mode_readiness_snapshot` | Application Projection Cache | parallel readiness projection 표시 | TUI가 재계산하지 않고 cache만 유지 |
| `parallel_mode_supervisor_snapshot` | Application Projection Cache | supervisor/slot/queue projection 표시 | TUI가 dispatch 가능 여부를 재판단하지 않음 |
| `supersession_mud_ui_state` | UI-only | supersession overlay selection/navigation | TUI presentation state로 유지 |
| `parallel_mode_supervisor_refresh_in_flight` | Runtime Bridge | refresh effect 중복 실행 방지와 spinner 성격 | command correlation은 application runtime store로 이동 검토 |
| `parallel_mode_orchestrator_wake_in_flight` | Runtime Bridge | wake effect 중복 실행 방지 | application control-plane wake coalescing으로 이동 검토 |
| `parallel_mode_orchestrator_tick_in_flight` | Runtime Bridge | tick effect 중복 실행 방지 | application runtime event/correlation으로 이동 검토 |
| `last_parallel_mode_orchestrator_tick_signature` | Runtime Bridge | repeated tick/result 표시와 중복 guard | application event id 또는 projection으로 이동 검토 |
| `parallel_mode_automation_epoch_id` | Runtime Bridge | stale automation completion gate | application control-plane epoch로 이동 검토 |
| `next_parallel_mode_automation_epoch_id` | Runtime Bridge | TUI-local epoch allocation | application runtime allocator로 이동 검토 |
| `last_parallel_mode_automation_trigger` | Runtime Bridge | 마지막 automation trigger 표시/guard | application projection notice로 낮추기 |
| `last_parallel_mode_dispatch_withheld_reason` | Application Projection Cache | dispatch withheld reason 표시 | domain/application decision result를 표시만 함 |
| `conversation_state` | Hybrid | transcript, prompt readiness, stream projection, planning snapshot을 함께 보유 | `TUI-00B`에서 하위 field별로 쪼갬 |
| `selected_session_index` | UI-only | session overlay row selection | TUI controller에 유지 |
| `session_overlay_ui_state` | UI-only | session overlay page/search/navigation | TUI controller에 유지 |
| `auto_follow_overlay_ui_state` | UI-only | auto-follow controls overlay의 local settings view | application policy를 재계산하지 않고 command 입력만 생성 |
| `directions_maintenance_overlay_ui_state` | UI-only | directions maintenance form/step state | save/apply만 application command로 보냄 |
| `planning_init_overlay_ui_state` | UI-only | planning init/review/manual editor step state | workspace bootstrap은 application service 호출 |
| `planning_draft_editor_ui_state` | UI-only | editor buffer, cursor, dirty/close-risk 표시 | save/promote는 application command로 보냄 |
| `task_intake_overlay_ui_state` | UI-only | task intake prompt/preview/confirm overlay state | preview/commit은 planning application service 호출 |
| `pending_task_intake_command` | UI-only | inline shell command에서 task intake overlay로 넘길 임시 input | durable task authority가 아님 |
| `active_session` | Application Projection Cache | current session summary 표시와 thread id context | session service result cache로 유지 |
| `startup_service` | Service Wiring | startup diagnostics service handle | state 아님 |
| `session_service` | Service Wiring | session catalog service handle | state 아님 |
| `conversation_service` | Service Wiring | app-server conversation service handle | state 아님 |
| `parallel_agent_worker_port` | Service Wiring | parallel worker port handle | state 아님 |
| `turn_control_truth` | Application Projection Cache | runtime control capability copy 표시 | conversation service capability projection으로 유지 |
| `parallel_mode_service` | Service Wiring | parallel application service handle | state 아님 |
| `planning` | Service Wiring | planning service bundle handle | state 아님 |
| `active_turn_execution_snapshot_capture` | Runtime Bridge | active turn workspace snapshot 또는 capture failure를 post-turn으로 전달 | application post-turn event input으로 이동 검토 |
| `planning_worker_panel_state` | Application Projection Cache | planning worker/repair/refresh progress 표시 | application event result projection으로 유지 |
| `planning_worker_visibility` | UI-only | panel visibility env/user preference | TUI-only config로 유지 |
| `github_review_poller_service` | Service Wiring | GitHub polling service handle | state 아님 |
| `github_review_polling_state` | Application Projection Cache | polling enabled/loading/result 표시 | projection cache로 유지 |
| `inline_history_render_mode` | UI-only | terminal rendering strategy | TUI-only config로 유지 |
| `history_insert_mode` | UI-only | transcript insertion/rendering strategy | TUI-only config로 유지 |
| `show_startup_ascii_art` | UI-only | terminal decoration flag | TUI-only config로 유지 |
| `tx` | Runtime Bridge | background effect result channel sender | runtime bridge에 남기되 write semantics는 제한 |
| `rx` | Runtime Bridge | background effect result channel receiver | handler는 reducer/application event로만 전달 |

## Conversation State 하위 감사 필요

`conversation_state`는 이번 inventory에서 intentionally `Hybrid`로 둔다. 내부에는
다음 성격이 섞여 있다.

- UI-only: prompt input readiness copy, transcript viewport/rendering context
- Application Projection Cache: `PlanningRuntimeSnapshot`, runtime status text, app-server stream projection
- Runtime Bridge: auto-follow submission provenance, queued prompt context, post-turn state transition

`TUI-00B`는 `ConversationState`와 `BackgroundMessage`를 함께 감사해 background event가
어느 reducer 또는 application command로 들어가야 하는지 표로 고정했다. 완료 문서는
[tui-background-message-inventory.md](./tui-background-message-inventory.md)이다.

## Immediate Migration Guard

`TUI-00` 동안 지켜야 할 금지 사항:

- inventory 없이 `NativeTuiApp` field를 application service로 이동하지 않는다.
- `Runtime Bridge` field를 없애면서 equivalent application event/correlation test를 만들지 않는 변경을 금지한다.
- `Application Projection Cache` field에서 queue, planning, parallel policy를 재계산하지 않는다.
- `UI-only` field를 durable repository나 domain model에 올리지 않는다.

## 다음 Slice

- `TUI-00B`: `BackgroundMessage`별 target boundary inventory와 최소 shell runtime regression. 완료.
- `TUI-00C`: prompt lock, overlay focus, selection/cursor와 projection update 충돌을 막는 input/rendering regression.
- `TUI-01`: `TUI-00` regression 이후 conversation lifecycle과 automation lifecycle 분리.
