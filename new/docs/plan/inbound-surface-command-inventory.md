# Inbound Surface Command Inventory

## 목적

이 문서는 `INBOUND-00A`의 산출물이다. 기준 architecture는
[../architecture/inbound-surface-unification-architecture.md](../architecture/inbound-surface-unification-architecture.md)
이며, 목적은 production code를 움직이기 전에 현재 inbound command surface와 regression
anchor를 고정하는 것이다.

`INBOUND-00`의 구현은 한 번에 TUI, CLI, admin API, Telegram을 모두 바꾸지 않는다. 이
inventory는 어떤 surface가 어떤 application command/use case를 호출해야 하는지 먼저
정리하고, 다음 slice가 중복 policy를 제거할 때 사용할 안전 장치를 기록한다.

## Command Boundary Inventory

| Surface | Command 또는 event | 현재 application boundary | Regression anchor |
| --- | --- | --- | --- |
| CLI | `akra status`, `akra queue` | `PlanningControlCommand::{Status, Queue}` -> `PlanningControlService` | `status_and_queue_commands_use_planning_control_surface` |
| CLI | `akra reset <queue|directions|all>` | `PlanningResetTarget` -> planning workspace reset service | `reset_command_spelling_maps_to_shared_application_target` |
| CLI | `akra planning-tool <contract|run>` | `PlanningTaskToolRequest`/`PlanningTaskToolResponse` -> planning task tool use case | `planning_tool_contract_is_json_and_worker_oriented` |
| CLI | `akra parallel-tick` | `ParallelModeService::process_distributor_queue` | 후속 `INBOUND-00E`에서 control-plane runtime command로 정렬 |
| Telegram | `/status`, `/queue`, `/plan [status]` | `PlanningControlCommand::{Status, Queue}` -> `PlanningControlService` | `parse_message_maps_supported_planning_commands_to_shared_control_enum`, `runner_executes_planning_command_for_allowed_chat` |
| Telegram | `/reset queue`, `/reset_queue`, `/reset_directions`, `/reset_all` | `PlanningControlCommand::Reset(PlanningResetTarget)` -> `PlanningControlService` | `parse_message_maps_supported_planning_commands_to_shared_control_enum` |
| Telegram | `/whoami` | Telegram adapter local command | `help_reply_mentions_whoami_without_allowlist` |
| Admin HTML | direction/task/draft/reset forms | `PlanningAdmin*Request`, `PlanningResetTarget` -> `PlanningAdminFacadeService` | `reset_form_and_json_spelling_maps_to_shared_application_target`, form/template tests |
| Admin JSON | planning summary/runtime/draft/task/reset API | same facade/request DTO family as HTML where the operation matches | admin API/page parser tests; 후속 `INBOUND-00C`에서 route pair별 anchor 보강 |
| Admin Akra dashboard | read-only planning/parallel dashboard | `PlanningAdminFacadeService`, `ParallelModeService` projection | `akra_graphic_dashboard_keeps_admin_and_snapshot_surfaces` |
| TUI planning shell/overlay | `:planning`, `:task`, `:reset`, editor/overlay actions | `PlanningServices`, planning controller request mapping | 후속 `INBOUND-00D`에서 CLI/admin command vocabulary와 정렬 |
| TUI parallel shell/panel | `:parallel`, post-turn wake, supervisor refresh | `ParallelModeService`, domain state machine, panel controller | parallel/TUI regression anchors; 후속 `INBOUND-00E`에서 CLI/admin tick vocabulary와 정렬 |
| TUI conversation | prompt submit, stream, post-turn automation | `ConversationRuntimeEvent`, `ConversationLifecycleEvent` | TUI-01 regression matrix |

## Regression Guard

`INBOUND-00A`는 새 application facade를 만들지 않는다. 대신 surface spelling이
application command enum으로 내려가는 최소 regression을 추가한다.

- CLI reset spelling은 `PlanningResetTarget`으로만 내려간다.
- Admin reset spelling은 HTML form과 JSON API가 공유하는 `parse_reset_target`에서
  `PlanningResetTarget`으로만 내려간다.
- Telegram planning command spelling은 `PlanningControlCommand`로만 내려간다.

이 guard는 다음 변경을 금지한다.

- CLI/admin/Telegram 전용 reset enum 추가
- reset target free-form string을 application mutation path로 전달
- Telegram parser가 planning status/queue/reset을 자체 response로 처리
- admin HTML과 JSON reset spelling이 서로 다른 parser를 쓰는 변경

## 다음 Slice

- `INBOUND-00B`: CLI와 Telegram의 planning control command context와 response contract를
  같은 request/result vocabulary로 정렬한다. 완료.
- `INBOUND-00C`: admin HTML/JSON route pair가 같은 mutation request DTO와 facade method를
  통과하는지 route pair별 regression을 보강한다.
- `INBOUND-00D`: TUI planning shell command와 CLI/admin control vocabulary의 차이를 줄인다.
- `INBOUND-00E`: parallel TUI/admin/CLI entrypoint를 control-plane runtime command/event
  vocabulary로 정렬한다.

## INBOUND-00B 완료 근거

- `PlanningControlRequest`와 `PlanningControlResponse`를 planning control application
  surface에 추가했다.
- `PlanningControlSurface::workspace_dir`가 response context의 단일 source가 되도록 했다.
- CLI `status`/`queue`와 Telegram planning command execution이 모두
  `PlanningControlService::execute_request`를 통과한다.
- `execute_request_returns_shared_response_context` regression으로 response가 shared reply와
  workspace context를 함께 반환하는 계약을 고정했다.
