# Planning Control Plane Migration Plan

## 목적

이 문서는 `PLAN-00`의 산출물이다. 목표는 planning 구조를 바꾸기 전에 현재
user-visible contract와 fan-in 지점을 고정해, 이후 `PLAN-01`과 `PLAN-02`가
새 facade나 domain decision을 추가하더라도 기존 동작을 조용히 깨지 못하게 하는 것이다.

기준 architecture는
`new/docs/architecture/planning-control-plane-architecture.md`이다. 이 문서는 현재
구조를 정당화하는 설명서가 아니라, 다음 slice가 어떤 계약을 보존하면서 구조를
이동해야 하는지 정하는 작업 계획서다.

## Regression Contract

| 계약 | 고정 위치 | 보호하는 동작 |
| --- | --- | --- |
| queue ordering / proposal classification | `src/domain/planning/queue/tests.rs` | proposed task가 높은 priority여도 executable queue head가 되지 않고, active/proposed lane이 분리된다. |
| authoring close-risk | `src/adapter/inbound/tui/app/planning_draft_editor_ui/tests.rs` | dirty buffer 또는 invalid staged draft는 첫 close에서 확인을 요구하고, save 결과는 dirty flag와 validation source를 함께 갱신한다. |
| repair / reconciliation | `src/application/service/planning/repair/reconciliation/tests.rs` | post-turn reconciliation은 active `result-output.md`만 restore 대상으로 보며, repair prompt는 accepted DB authority와 task command payload를 기준으로 한다. |
| admin projection | `src/application/service/planning/admin/projection.rs` | admin queue preview는 domain projection 순서를 재계산하지 않고 표시용 DTO로만 낮춘다. |
| TUI projection | `src/adapter/inbound/tui/app/planning/status_projection.rs` | TUI queue framing은 structured queue projection의 active/proposed/skipped lane을 우선한다. |
| task mutation audit | `src/application/service/planning/task_mutation/tests.rs` | preview/commit, worker/user audit attribution, no-op update, terminal status guard가 하나의 mutation path를 통과한다. |

`PLAN-01`은 이 테스트 이름을 내부 helper 단위로 약화시키면 안 된다. 테스트 이름은
사용자 또는 operator가 관찰하는 contract를 설명해야 한다.

## Fan-In Audit

| Flow | 현재 진입점 | 직접 읽는 planning state | 다음 slice의 목표 |
| --- | --- | --- | --- |
| TUI runtime status / queue framing | `NativeTuiApp` planning controller, `status_projection.rs` | `PlanningRuntimeSnapshot`, `PriorityQueueProjection` | `PlanningApplicationProjection`을 읽는 adapter mapping으로 낮춘다. |
| TUI draft editor | `planning_draft_editor_ui.rs`, `controller/editor.rs` | `PlanningDraftEditorSession`, validation report | close-risk는 TUI controller에 남기고, draft save/promote는 application command로 묶는다. |
| Admin overview / draft session | `PlanningAdminFacadeService` | doctor report, runtime snapshot, task/direction docs, queue preview | rich admin view도 공통 projection source에서 section을 확장한다. |
| CLI planning commands | `src/adapter/inbound/cli.rs`, `cli/reports.rs` | `PlanningServices`, doctor/reset/tool response | CLI는 request parsing과 JSON/text rendering만 맡고 command facade를 호출한다. |
| Telegram control | `telegram_bot/message.rs` | `PlanningControlCommand`, compact control reply | Telegram parser는 command enum만 만들고, status/queue/reset은 shared command path를 쓴다. |
| Hidden planning worker | `worker/orchestration.rs`, `task_mutation.rs` | worker response, task command extraction, repository snapshot | worker success/failure를 application event로 되돌리고, mutation legality는 domain decision으로 이동한다. |
| Post-turn reconciliation | `repair/reconciliation.rs`, TUI post-turn runtime | changed planning paths, execution snapshot | protected-file restore와 repair request 생성은 application event/effect 경계로 표준화한다. |
| Queue/proposal policy | `domain/planning/queue.rs` | direction/task authority document | queue ordering, proposal classification, skipped reason은 domain-owned decision으로 유지한다. |

## State Ownership Baseline

| State | 현재 기준 | 이동 원칙 |
| --- | --- | --- |
| direction/task authority | `PlanningTaskRepositoryPort` | durable authority다. adapter나 TUI state로 복제하지 않는다. |
| queue projection | domain 계산 + repository snapshot | domain decision 결과이며, inbound가 재계산하지 않는다. |
| worker retry / in-flight | application runtime state | durable repository처럼 숨기지 않는다. |
| draft editor cursor / close guard | TUI UI state | application service로 올리지 않는다. |
| admin expanded row/filter | admin adapter state | planning policy가 아니다. |
| validation report | domain validation result + application projection | surface별로 severity나 issue count를 다시 판단하지 않는다. |

## PLAN-01 작업 단위

`PLAN-01`은 바로 대규모 교체를 하지 않는다. 아래 순서로 PR을 더 잘게 나눈다.

1. `PlanningApplicationProjection` 최소 타입을 추가한다. 완료: `PLAN-01A`
2. runtime snapshot, admin overview, control status가 같은 projection source를 읽는
   compatibility mapper를 만든다. 완료: control status/queue `PLAN-01C`
3. TUI/admin/CLI/Telegram 호출부를 한 번에 바꾸지 않고, 먼저 read-only status/queue
   command부터 facade 뒤로 보낸다. 진행: TUI status projection `PLAN-01B`, control
   surface status/queue `PLAN-01C`, CLI status/queue command `PLAN-01D`
4. mutation이나 worker orchestration은 `PLAN-01`에서 정책을 바꾸지 않는다. 필요한
   경우 `PLAN-02`로 미룬다.

완료 조건:

- `/status`, `/queue`, admin overview, TUI status가 같은 authority revision의 queue
  facts를 보게 된다.
- adapter는 projection rendering만 검증하고 queue/proposal policy를 직접 호출하지 않는다.
- 기존 `PLAN-00` regression test가 그대로 통과한다.

## PLAN-02 작업 단위

`PLAN-02`는 application helper에 남은 순수 판단을 domain으로 이동한다.

- task mutation legality
- proposal promotion 가능 여부
- queue-idle / repeated-head decision
- repair eligibility

완료 조건:

- domain decision은 I/O 없이 테스트된다.
- application service는 load/save/effect ordering만 테스트한다.
- worker output은 accepted authority가 아니라 untrusted candidate로만 들어온다.

## 금지 패턴

- facade 표준화 전에 surface별로 새 planning policy를 추가하지 않는다.
- runtime-local state를 SQLite authority row처럼 저장하지 않는다.
- admin/TUI가 `PriorityQueueService`를 직접 호출해 queue head를 다시 만들지 않는다.
- worker response를 validation과 optimistic commit 없이 authority로 반영하지 않는다.
- close-risk, cursor, selected row 같은 UI-only state를 domain/application state로 올리지 않는다.

## 검증 명령

`PLAN-00` slice는 아래 명령을 통과해야 한다.

```bash
cargo test domain::planning::queue
cargo test planning_draft_editor_ui
cargo test application::service::planning::repair::reconciliation
cargo test application::service::planning::admin::projection
cargo test adapter::inbound::tui::app::planning::status_projection
cargo test application::service::planning::task_mutation
```

문서 변경은 `git diff --check`로 whitespace regression을 확인한다.
