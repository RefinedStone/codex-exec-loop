# Planning Control Plane Architecture

> 상태: 배경 기준 문서다. 현재 구현 판정과 다음 작업은
> [repository-wide-rebuild-roadmap.md](../plan/repository-wide-rebuild-roadmap.md)를 따른다.
> 이 문서의 계획/완료 표현은 현재 완료 판정이 아니다.

## 목적

이 문서는 planning 영역을 repository-wide rebuild 기준에 맞춰 다시 세우기 위한
architecture 기준선이다. 현재 planning 코드는 이미 `domain`, `application`,
`port`, `adapter`가 존재하지만, authoring, runtime, repair, worker, admin,
task mutation이 서로 다른 표면에서 부분적으로 자라며 같은 판단을 여러 경로에
복제하기 쉬운 형태가 되었다.

목표는 planning을 하나의 control-plane으로 다루는 것이다.

```text
Inbound surface는 planning 의도를 보낸다.
Application planning runtime은 변경 순서를 직렬화한다.
Domain planning aggregate/service는 검증, 분류, 큐 정책을 판단한다.
Repository/store는 durable authority와 projection을 보관한다.
Inbound surface는 Application Projection만 표시한다.
```

이 문서는 현재 구조 설명서가 아니다. `new/docs`의 새 구조 기준이며, 이후
`PLAN-*` implementation slice는 이 문서를 source of truth로 삼는다.

## 문제 정의

planning은 다음 흐름들이 한 bounded context 안에 섞인다.

- planning workspace bootstrap과 draft authoring
- direction authority와 task authority 편집
- user prompt를 task로 바꾸는 runtime intake
- queue ordering과 auto-follow handoff
- post-turn protected-file reconciliation
- hidden planning worker queue refresh와 repair
- admin API/UI, CLI, Telegram control surface
- parallel official completion refresh

현재 구현은 좋은 재료를 이미 갖고 있다.

- `domain/planning`에는 semantic validation과 priority queue service가 있다.
- `application/service/planning/runtime`에는 prompt snapshot과 auto-follow policy가 있다.
- `task_mutation`은 user/worker/system mutation을 같은 task authority commit path로 모은다.
- `PlanningControlService`는 CLI/Telegram이 공유할 command surface를 갖는다.
- `PlanningAdminFacadeService`는 admin-facing projection과 mutation surface를 제공한다.
- outbound port는 workspace file, task repository, planning worker, authority store로 나뉘어 있다.

문제는 “모듈이 없다”가 아니라 “최종 소유자가 하나로 고정되지 않았다”는 점이다.
따라서 다음 변경은 새 거대 facade를 추가하는 것이 아니라, 기존 재료를 하나의
control-plane 언어로 정렬하는 방식이어야 한다.

## 핵심 원칙

### 1. Planning은 Surface가 아니라 Bounded Context다

TUI, admin API, CLI, Telegram은 planning bounded context가 아니다. 이들은
inbound adapter다. 같은 planning 조작은 같은 application command/facade를
호출해야 한다.

```text
TUI :task
Admin task edit
Telegram /reset queue
CLI planning report
  -> PlanningControlPlaneCommand
  -> Planning application runtime/use case
```

surface별 차이는 request parsing, authorization context, response rendering에만 둔다.

### 2. Durable Authority와 Runtime State를 분리한다

planning에는 복구되어야 하는 state와 프로세스 안에서만 의미 있는 state가 섞인다.

| State 종류 | 목표 소유자 | 예시 |
| --- | --- | --- |
| Durable authority | repository/store | direction authority, task authority, queue projection, official completion claim |
| Durable workspace artifact | outbound workspace adapter | draft files, result-output, supporting files |
| Process-lifetime runtime state | application runtime store | worker in-flight, retry attempt, command correlation, stale event gate |
| Domain invariant | domain | semantic validation, queue ordering, proposal classification, task mutation legality |
| UI-only state | inbound controller | editor cursor, overlay step, selected task row, expanded admin section |
| Application Projection | application read model | planning status, queue head, visible tasks, repair state, admin/control summary |

DB에 저장되지 않아도 되는 runtime state를 repository로 숨기지 않는다. 반대로 task
authority, queue projection, official completion claim처럼 재시작 후에도 의미가
있는 값은 UI나 thread local state에 두지 않는다.

### 3. Domain이 Planning Policy의 주인이다

I/O 없이 판단할 수 있는 것은 domain에 둔다.

Domain 소유:

- semantic validation issue 생성
- queue ordering과 `next_task` 선택
- proposed/ready/in_progress/skipped task classification
- direction state가 task 실행 가능성에 미치는 영향
- dependency/blocker resolution
- task mutation input의 legality
- proposal classification과 promotion 가능성
- repeated queue head 또는 stale proposal을 어떻게 표시할지에 대한 순수 decision

Application은 domain decision을 실행하고 저장 순서를 보장한다. application 안의
`if/else`가 policy로 커지면 다음 slice에서 domain decision으로 내려야 한다.

### 4. Application은 Single-Writer Runtime과 Use Case 순서를 가진다

planning은 parallel mode만큼 thread가 많지는 않지만, hidden worker, admin write,
runtime intake, post-turn reconciliation, official completion refresh가 같은
authority를 갱신한다. 그러므로 write path는 application runtime/use case가 한 줄로
순서화해야 한다.

Application 소유:

- command/event 처리 순서
- repository load/save transaction boundary
- optimistic revision conflict retry
- prompt assembly
- hidden worker launch와 retry orchestration
- workspace sync와 protected-file restoration
- official completion refresh claim acquire/release
- projection invalidation과 rebuild
- outbound port 호출과 error mapping

### 5. Inbound는 Projection만 읽는다

TUI/admin/CLI/Telegram은 planning internals를 직접 읽지 않는다. 이들은 application이
제공하는 projection을 표시하거나, command 결과를 surface별 copy로 렌더링한다.

금지:

- inbound가 `PriorityQueueService`를 직접 호출해 queue head를 다시 계산
- inbound가 task authority document를 직접 수정
- inbound가 hidden worker retry 여부 판단
- inbound가 validation/repair policy를 재구현

허용:

- keyboard/form/chat command를 application command로 mapping
- UI-only selection/cursor/editor buffer state 관리
- application projection을 표, popup, text reply, JSON response로 변환

## 목표 모듈 경계

### Domain

위치:

```text
src/domain/planning
```

목표 역할:

- `PlanningControlPlaneAggregate` 또는 동등한 domain service vocabulary 제공
- accepted direction/task authority를 입력으로 받아 validation, queue, mutation decision 반환
- task mutation legality와 audit metadata rule을 순수 함수로 고정
- queue/proposal classification을 모든 surface가 공유하는 projection vocabulary로 반환

현재 유지할 수 있는 재료:

- `PlanningSemanticValidationService`
- `PriorityQueueService`
- `TaskAuthorityDocument`, `DirectionCatalogDocument`
- `PlanningValidationReport`

다음 slice에서 추출할 후보:

- task mutation validation helper
- proposal promotion 가능 여부
- queue-idle decision
- repeated queue head block decision

### Application Runtime / Facade

위치:

```text
src/application/service/planning
```

목표 역할:

- `PlanningControlPlaneCommand`를 surface 공통 진입점으로 정의
- command/event를 받아 use case 순서를 직렬화
- durable authority와 workspace artifact를 port로 읽고 쓴다
- domain decision을 저장 가능한 change와 실행할 effect로 변환
- hidden worker 완료/실패를 application event로 되돌린다
- Application Projection을 생성한다

초기 facade 후보:

```text
PlanningControlPlaneService
PlanningApplicationProjectionService
PlanningCommandRuntime
PlanningCommand
PlanningEvent
PlanningEffect
```

기존 `PlanningServices`, `PlanningAdminFacadeService`, `PlanningControlService`는 바로
삭제하지 않는다. 먼저 이들이 새 command/projection surface를 호출하도록 얇게 감싼다.

### Application Projection

Planning Application Projection은 모든 inbound surface가 읽는 공통 read model이다.
admin처럼 rich view가 필요한 surface도 같은 projection에서 확장된 section을 읽어야 한다.

최소 필드:

```text
workspace_id
workspace_state
authority_revision
validation_report
queue_head
visible_tasks
proposed_tasks
skipped_tasks
queue_idle_policy
repair_state
worker_state
last_operation
operator_notices
```

원칙:

- projection은 display-ready일 수 있지만 policy를 새로 판단하지 않는다.
- queue head와 task lists는 같은 authority revision에서 온다.
- admin/control/TUI projection 이름은 다를 수 있어도 source는 하나여야 한다.
- projection rebuild는 application service 책임이며 inbound adapter가 직접 조립하지 않는다.

### Repository / Store

기존 port를 당장 갈아엎지 않는다.

유지할 durable boundary:

- `PlanningTaskRepositoryPort`
- `PlanningWorkspacePort`
- `PlanningAuthorityPort`
- `PlanningWorkerPort`

정리 방향:

- task/direction authority와 queue projection은 `PlanningTaskRepositoryPort`가 authoritative하게 다룬다.
- workspace draft/result/supporting files는 `PlanningWorkspacePort`가 다룬다.
- official completion claim, parallel-linked runtime projection은 당분간 `PlanningAuthorityPort`에 남긴다.
- 새 repository trait은 중복 load/save가 실제로 줄어드는 slice에서만 만든다.

## Command / Event 언어

초기 command 후보:

```text
InitializeWorkspace
LoadProjection
PrepareTaskIntake
CommitTaskIntake
PreviewTaskMutation
CommitTaskMutation
PromoteProposal
RefreshQueueFromWorker
RecordOfficialCompletion
RunRepair
ResetWorkspace
SyncDraft
UpdateDirection
```

초기 event 후보:

```text
ProjectionLoaded
AuthorityCommitted
AuthorityCommitConflict
WorkerCompleted
WorkerFailed
RepairRequested
RepairCompleted
WorkspaceReset
StaleEventDropped
```

초기 effect 후보:

```text
RunPlanningWorker
WriteWorkspaceDraft
RestoreProtectedFile
AcquireOfficialCompletionClaim
ReleaseOfficialCompletionClaim
EmitOperatorNotice
```

command/event/effect 이름은 구현 중 바뀔 수 있다. 단, 모든 surface가 같은 의미를
공유하고, worker/thread 결과가 application event로 돌아오는 구조는 유지해야 한다.

## 주요 흐름

### Runtime Task Intake

```text
TUI/Admin/CLI input
  -> PlanningControlPlaneCommand::PrepareTaskIntake
  -> application loads direction/task authority
  -> domain validates generated draft and mutation legality
  -> application returns preview projection
  -> inbound confirms
  -> PlanningControlPlaneCommand::CommitTaskIntake
  -> application commits task authority with observed revision
  -> projection rebuild
```

중요 계약:

- preview와 commit은 같은 mutation service/domain decision을 사용한다.
- revision conflict는 application이 재시도하거나 conflict result로 반환한다.
- inbound는 task id collision, direction fallback, queue recalculation을 직접 처리하지 않는다.

### Queue Refresh Worker

```text
post-turn evaluation or queue-idle trigger
  -> PlanningControlPlaneCommand::RefreshQueueFromWorker
  -> application assembles worker prompt
  -> effect runner calls PlanningWorkerPort
  -> worker response becomes PlanningEvent::WorkerCompleted or WorkerFailed
  -> application extracts structured task commands
  -> domain validates mutation/proposal classification
  -> repository commit
  -> projection rebuild
```

중요 계약:

- prompt assembly와 hidden worker retry는 application 소유다.
- worker output은 바로 authority를 덮어쓰지 않는다.
- structured command extraction 이후에도 domain validation과 optimistic commit을 거친다.
- worker failure는 status copy가 아니라 application event로 보상 처리된다.

### Post-Turn Reconciliation

```text
turn starts
  -> application captures PlanningExecutionSnapshot
turn completes
  -> application receives changed planning file paths
  -> application asks reconciliation service
  -> protected workspace file restore if needed
  -> repair request or projection notice
```

중요 계약:

- conversation runtime은 planning protected-file policy를 직접 판단하지 않는다.
- reconciliation은 file path normalization과 restore ordering을 application service에서 수행한다.
- repair request는 projection에 드러나며, inbound는 repair prompt를 직접 만들지 않는다.

### Admin / CLI / Telegram Control

```text
surface request
  -> surface auth/context mapping
  -> PlanningControlPlaneCommand or PlanningControlService compatibility wrapper
  -> application projection or mutation result
  -> surface-specific rendering
```

중요 계약:

- `/queue`, admin queue panel, CLI report는 같은 projection source를 읽는다.
- reset은 target enum으로만 들어온다.
- destructive command parsing과 authorization은 adapter 책임이지만, reset 범위와 post-reset health는 application 책임이다.

## 상태 소유권 표

| 상태/판단 | 목표 소유자 | 비고 |
| --- | --- | --- |
| direction/task authority document | durable repository | accepted planning truth |
| queue projection | domain + repository | domain이 계산하고 repository가 revision과 함께 저장 |
| validation report | domain decision / application projection | inbound가 재계산하지 않음 |
| task mutation audit attribution | domain value + application request | source/turn/thread provenance를 surface별로 재해석하지 않음 |
| worker prompt | application | worker adapter는 prompt를 재조립하지 않음 |
| worker response stream handling | outbound adapter | application에는 축약 response/event만 반환 |
| worker retry / repair attempt count | application runtime store | DB state로 위장하지 않음 |
| official completion refresh order | durable authority store | multi-worker ordering을 복구 가능하게 유지 |
| editor cursor/selected file | TUI controller | UI-only |
| admin expanded row/filter | admin adapter | UI-only |
| Telegram chat allowlist | Telegram adapter | planning policy가 아님 |

## 구현 순서

### Step 1. Planning Regression And Audit Contract

`PLAN-00`은 구조 변경 전 현재 계약을 고정한다.

- queue ordering/proposal classification regression
- task mutation preview/commit conflict regression
- authoring draft promote regression
- repair/reconciliation regression
- admin/TUI/Telegram projection fan-in audit

### Step 2. Application Projection 표준화

기존 admin/control/runtime projection을 하나의 planning projection vocabulary로 묶는다.

- 최소 projection 타입 추가
- TUI/admin/control mapping test
- 기존 surface는 compatibility wrapper로 유지

### Step 3. Command Surface 표준화

surface별 직접 service call을 command/facade 호출로 줄인다.

- task intake
- task mutation
- reset/status/queue
- worker refresh

### Step 4. Domain Decision 강화

application helper에 남은 순수 판단을 domain으로 내린다.

- mutation legality
- proposal classification
- queue-idle/repeated-head decision
- repair eligibility

### Step 5. Worker/Event Path 정리

planning worker success/failure를 application event로 되돌린다.

- stale event drop
- retry attempt store
- repair request projection
- official completion ordering

## 테스트 기준

| Test 종류 | 위치 | 검증 대상 |
| --- | --- | --- |
| Domain decision test | `src/domain/planning` | validation, queue, mutation legality, classification |
| Application ordering test | `src/application/service/planning` | command/event order, revision conflict, worker retry |
| Adapter mapping test | `src/adapter/inbound/*` | parsing, auth/context mapping, rendering |
| Store contract test | outbound adapter tests | revision, atomic commit, claim/release |
| Regression flow test | TUI/admin/Telegram tests | user-visible contract |

테스트 이름은 내부 helper가 아니라 계약을 설명해야 한다.

좋은 예:

```text
task_mutation_retry_preserves_preview_timestamp_after_revision_conflict
admin_and_telegram_queue_read_same_projection_source
worker_failure_returns_repair_event_without_committing_candidate_authority
```

나쁜 예:

```text
service_helper_works
format_queue_line
handle_button
```

## 금지 패턴

### Surface-Specific Planning Policy

```text
admin reads task authority
admin recomputes queue
admin decides task can run
admin writes task authority
```

이 흐름은 planning policy를 admin surface에 복제한다. admin은 command를 보내고
projection을 표시해야 한다.

### Worker Output Direct Commit

```text
worker response
  -> write task authority
```

worker output은 untrusted candidate다. structured extraction, domain validation,
optimistic revision commit을 지나야 accepted authority가 된다.

### Runtime State Hidden In Durable Store

```text
retry timer / in-flight worker flag
  -> sqlite row
```

재시작 후 복구해야 하는 state가 아니라면 application runtime store에 둔다. 반대로
official completion claim처럼 재시작 후 ordering이 필요한 값은 durable store에 둔다.

### Projection Rebuilt In Inbound

```text
TUI loads authority files
TUI calls PriorityQueueService
TUI builds queue panel
```

TUI는 projection을 표시한다. projection source는 application이 소유한다.

## 완료 기준

planning 구조 리팩터링은 다음 조건을 만족해야 완료로 볼 수 있다.

- 모든 inbound surface가 planning policy를 직접 판단하지 않는다.
- planning Application Projection이 TUI/admin/CLI/Telegram의 공통 source가 된다.
- task authority mutation은 하나의 application path를 통과한다.
- semantic validation, queue ordering, proposal classification은 domain-owned decision이다.
- worker success/failure는 application event로 돌아와 보상 처리된다.
- durable authority state와 process-lifetime runtime state가 섞이지 않는다.
- regression tests가 domain/application/adapter/store 계약별로 분류된다.
