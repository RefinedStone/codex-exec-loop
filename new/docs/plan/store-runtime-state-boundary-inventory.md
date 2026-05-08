# Store Runtime State Boundary Inventory

이 문서는 `STORE-00A`의 산출물이다. 기준 문서는
[store-and-runtime-state-architecture.md](../architecture/store-and-runtime-state-architecture.md)이며,
목표는 durable store, process-lifetime runtime store, workspace artifact, external runtime
artifact의 현재 소유자를 코드 변경 전에 고정하는 것이다.

## 결론

현재 저장 경계의 큰 방향은 맞다.

- accepted planning authority는 SQLite authority store와 `PlanningTaskRepositoryPort`가 가진다.
- parallel lease/session/dispatch/distributor 상태는 durable runtime projection으로 SQLite authority에 저장된다.
- git worktree, branch, PR은 Akra DB에 복제하지 않고 outbound runtime artifact로 관찰한다.
- TUI overlay/selection/loading은 UI-only state다.

다만 `STORE-00` 구현 전에 고쳐야 할 구조적 위험이 있다.

- pool-local mirror 파일 I/O가 `src/application/service/parallel_mode/*/store.rs`에 남아 있다.
- TUI가 parallel wake/epoch/in-flight guard를 process-lifetime runtime state로 직접 들고 있다.
- `PlanningAuthorityPort`가 durable authority metadata, durable runtime projection, claim coordination을 모두 가진다.
- filesystem workspace artifact와 DB authority mirror가 같은 adapter family에 있어 reset/recovery 테스트 이름이 더 명확해야 한다.

따라서 `STORE-00`은 새 repository trait부터 만들지 않는다. 먼저 아래 inventory를 기준으로
adapter 경계와 runtime state migration을 작은 slice로 나눈다.

## Durable State Inventory

| State | 현재 소유자 | 분류 | Recovery / reset contract | 다음 작업 |
| --- | --- | --- | --- | --- |
| `planning_revision`, schema metadata | `SqlitePlanningAuthorityAdapter` metadata | Durable authority metadata | authority snapshot commit과 같은 DB에서 갱신 | 유지. method별 owner를 inventory에 계속 기록 |
| direction authority | `PlanningTaskRepositoryPort` / `planning_directions` | Durable authority | optimistic revision으로 lost update 방지 | 유지. domain policy가 adapter로 내려가지 않는지 guard |
| task authority | `PlanningTaskRepositoryPort` / `planning_tasks` | Durable authority | accepted task state의 source of truth | 유지. queue projection과 같은 revision으로 검증 |
| queue projection | `PlanningTaskRepositoryPort` / `planning_queue_projection` | Durable authority projection | task authority와 같은 snapshot으로 load | 유지. projection rebuild는 application/domain 쪽에서만 수행 |
| shadow documents | SQLite `shadow_documents` | Durable mirror | filesystem workspace와 authority drift 진단 | `STORE-00E`에서 reset/sync 테스트 이름 보강 |
| active documents | `PlanningWorkspacePort` repo-scoped `active_documents` | Durable workspace artifact | active planning workspace snapshot | `STORE-00E`에서 authority와 artifact 구분 문서화 |
| staged drafts | `PlanningWorkspacePort` repo-scoped `staged_drafts` | Durable workspace artifact | promoted 전까지 accepted authority가 아님 | `STORE-00E`에서 draft reset/validation contract 보강 |
| official refresh order/claim | `PlanningAuthorityPort` / `runtime_claims` + metadata | Durable runtime coordination | stale claim recovery가 다음 order를 풀어야 함 | `STORE-00B`에서 regression matrix에 연결 |
| distributor queue claim | `PlanningAuthorityPort` / `runtime_claims` | Durable runtime coordination | 한 queue head를 한 owner만 처리 | `STORE-00B`에서 retry/claim release contract 고정 |
| dispatch commands | `PlanningAuthorityPort` / `runtime_dispatch_commands` | Durable runtime projection | restart 후 pending command를 claim 가능 | `STORE-00B`에서 pending/claimed/completed matrix 고정 |
| slot leases | `PlanningAuthorityPort` / `runtime_slot_leases` | Durable runtime projection | worktree probe와 함께 active slot 복구 | `STORE-00C`에서 mirror I/O boundary 분리 |
| invalid slot leases | `PlanningAuthorityPort` / `runtime_invalid_slot_leases` | Durable runtime projection | reconcile이 깨진 lease를 표시/정리 | `STORE-00B`에서 load snapshot matrix에 포함 |
| session details | `PlanningAuthorityPort` / `runtime_session_details` | Durable runtime projection | lease보다 오래 사는 UI/recovery detail | `STORE-00C`에서 mirror I/O boundary 분리 |
| task dispatch blocks | `PlanningAuthorityPort` / `runtime_task_dispatch_blocks` | Durable runtime projection | disposable pool reset 뒤에도 failed-start block 보존 | 유지. existing reset test가 있음 |
| distributor queue records | `PlanningAuthorityPort` / `runtime_distributor_queue` | Durable runtime projection | PR/integration retry 상태 보존 | `STORE-00C`에서 mirror I/O boundary 분리 |
| runtime events | `ParallelModeRuntimeEventLogPort` / `runtime_events` | Durable audit projection | recent event feed로 recovery/UI 진단 | `STORE-00B`에서 bounded feed contract 고정 |
| `.leases/*.json` | application parallel pool mirror helper | Durable mirror | authority store의 보조 관찰 파일 | `STORE-00C`에서 outbound filesystem/runtime port 뒤로 이동 |
| `.agent-sessions/*.json` | application session detail mirror helper | Durable mirror | authority store의 보조 관찰 파일 | `STORE-00C`에서 outbound filesystem/runtime port 뒤로 이동 |
| `.distributor-queue/*.json` | application distributor mirror helper | Durable mirror | authority store의 보조 관찰 파일 | `STORE-00C`에서 outbound filesystem/runtime port 뒤로 이동 |
| git worktree / branch | `ParallelModeRuntimePort` + git adapter | External runtime artifact | probe 후 lease/projection과 reconcile | `STORE-00D`에서 probe 결과와 durable projection join contract 고정 |
| GitHub PR | GitHub automation/review ports | External runtime artifact | PR number/url만 durable projection에 보관 | `STORE-00D`에서 PR artifact mapping을 distributor audit에 연결 |
| app-server session | app-server runtime/session ports | External runtime artifact | thread/session snapshot은 app-server에서 관찰 | `STORE-00D` 범위 밖. conversation architecture에서 다룸 |

## Process-Lifetime Runtime State Inventory

이 값들은 durable store로 내리면 안 된다. 재시작 후 durable projection과 probe로 다시 만들거나,
현재 프로세스의 event loop 수명 안에서만 의미가 있어야 한다.

| State | 현재 위치 | 목표 소유자 | 위험 | 다음 작업 |
| --- | --- | --- | --- | --- |
| parallel supervisor refresh in-flight bit | TUI `NativeTuiApp` | application control-plane runtime 또는 thin inbound bridge | TUI가 runtime concurrency guard를 직접 소유 | `STORE-00D` 또는 TUI 후속에서 runtime store로 이동 |
| orchestrator wake in-flight bit | TUI `NativeTuiApp` | application control-plane runtime | wake coalescing이 UI state와 섞임 | `STORE-00D`에서 command/event boundary 정리 |
| orchestrator tick in-flight bit | TUI `NativeTuiApp` | application control-plane runtime | single-writer loop와 inbound guard가 중복 | `STORE-00D`에서 control-plane effect state로 이동 |
| automation epoch id | TUI `NativeTuiApp`, domain event reducer 일부 | application runtime state + domain stale decision | stale event gate가 UI lifetime에 묶임 | `STORE-00D`에서 epoch owner를 application으로 고정 |
| next effect sequence | `ParallelModeControlPlaneEffectId` producer | application runtime state | durable projection처럼 저장하면 안 됨 | 유지. reset/restart 시 새 sequence 가능해야 함 |
| wake poll timestamp | TUI shell runtime | inbound scheduler UI/runtime bridge | poll timer는 durable state가 아님 | 유지하되 repository로 내리지 않음 |
| GitHub poll in-flight / next poll | TUI GitHub polling controller | inbound controller | external poll scheduler state | `TEST-00`에서 UI/runtime test taxonomy로 분류 |
| worker thread handle/channel sender | TUI/application worker launch path | application runtime/effect runner | durable store로 복구 불가 | `STORE-00D`에서 completion event만 durable projection과 연결 |
| dispatch owner token string | application service, saved into claim row | process-generated coordination token | token 생성은 runtime, claim row는 durable | 유지. claim row만 durable coordination |
| planning worker owner token string | planning worker orchestration | process-generated coordination token | official refresh claim과 혼동 가능 | 유지. claim acquire/release는 repository contract |
| `OnceLock` default direction definition | planning admin documents | process-local config memoization | domain state가 아니라 static template cache | 유지. reset 대상 아님 |
| Noop repository global maps | `#[cfg(test)]` fake ports | test fake only | production path로 승격되면 안 됨 | `TEST-00`에서 fake reset guideline 연결 |

## Adapter Mapping Versus Application Policy Audit

| Boundary | Adapter가 가져도 되는 것 | Application/domain에 있어야 하는 것 | 현재 판정 |
| --- | --- | --- | --- |
| SQLite authority adapter | schema, row mapping, transaction, optimistic revision, idempotent upsert | queue ordering, dispatch eligibility, retry policy | 대체로 적절. claim stale TTL은 저장 coordination 성격이라 허용 |
| Planning task repository port | direction/task snapshot load/commit/clear | task mutation legality, validation issue 생성 | 적절 |
| Planning workspace port | path normalization, file read/write, draft staging | workspace validation, promotion 가능 여부 | 적절. repo-scoped artifact와 accepted authority 용어를 테스트에 더 드러내야 함 |
| Parallel runtime port | git/fs/gh command primitive, path probe, timestamp | cleanup/retry/distributor 순서, capacity decision | 적절 |
| Application parallel mirror helpers | 현재 fs mirror write/read를 직접 수행 | mirror write primitive는 outbound로 내려야 함 | 구조 위험. `STORE-00C`에서 이동 대상 |
| TUI parallel controller | operator intent, UI-only display, thin command dispatch | in-flight effect ownership, epoch/wake coalescing | 구조 위험. `STORE-00D`에서 application runtime으로 이동 |
| Admin/CLI/Telegram inbound | request parsing, response rendering | store classification, reset policy | 현재 `INBOUND-00` 후속과 연결 |

## 테스트 Anchor 현황

이미 있는 anchor:

- `runtime_reset_preserves_latest_failed_start_dispatch_block_per_task`
- `runtime_dispatch_command_enqueue_claim_and_update_round_trips`
- `runtime_dispatch_command_cancel_marks_only_non_terminal_commands`
- `runtime_task_cleanup_removes_deleted_task_projections_only`
- `runtime_projection_snapshot_groups_current_rows_and_recent_events`
- `official_refresh_claim_orders_are_enforced_by_authority_store`
- `active_workspace_artifact_removal_preserves_task_authority_snapshot`
- `staged_draft_rows_do_not_mutate_active_workspace_or_task_authority_snapshot`
- `runtime_event_log_port_reads_recent_projection_events`
- `runtime_event_log_port_filters_events_after_sequence`
- `runtime_projection_loads_recent_runtime_event_feed_newest_first`
- `dispatch_orchestrator_loop_claims_one_durable_command_across_two_ticks`
- `build_dispatch_plan_uses_remaining_idle_capacity_when_other_worktrees_are_blocked`
- `build_supervisor_snapshot_reads_store_backed_runtime_projections_after_mirror_loss`
- `acquire_slot_lease_persists_metadata_and_marks_slot_leased`
- `acquire_slot_lease_rolls_back_authority_when_mirror_write_fails`
- `pool_ignores_stale_legacy_lease_mirror_after_store_removal`

보강할 anchor:

- pool-local mirror write가 outbound boundary로 이동한 뒤 authority-first write order가 유지되는지
- TUI in-flight/epoch state가 durable store에 저장되지 않고 application runtime event로만 흐르는지

## STORE-00 분할

`STORE-00` parent는 모든 하위 작업이 끝날 때까지 `ready`로 둔다.

- `STORE-00A`: 이 inventory 문서로 durable/process runtime state와 adapter policy audit를 고정한다. 완료.
- `STORE-00B`: SQLite runtime projection regression matrix를 보강한다. dispatch command, runtime event feed,
  claim/recovery, task cleanup projection을 같은 DB read boundary로 검증한다. 완료.
- `STORE-00C`: pool-local mirror I/O를 application service helper에서 outbound filesystem/runtime boundary로 이동한다.
  authority-first write order와 mirror-loss recovery 테스트를 유지한다. ready.
- `STORE-00D`: parallel wake/epoch/in-flight process state를 TUI-owned state에서 application control-plane runtime
  state로 이동하는 최소 slice를 설계/구현한다. `PAR-03`, `PAR-04`, `TUI-01` regression을 같이 확인해야 하므로
  `STORE-00B/C` 이후 진행한다.
- `STORE-00E`: planning workspace artifact reset/sync contract를 보강한다. filesystem artifact, repo-scoped
  active documents, shadow documents, accepted DB authority가 서로 다른 reset boundary를 갖는지 고정한다. 완료.

## 다음 Worker 시작 기준

다음 worker는 새 repository trait를 만들지 말고 `STORE-00C`를 잡는다. `STORE-00D`는
TUI/application runtime migration이라 `STORE-00C`의 mirror I/O boundary가 정리된 뒤에 진행한다.
