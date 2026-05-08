# Parallel Control Plane Migration Plan

## 문서 목적

이 문서는 `Parallel Control Plane Architecture`를 실제 코드 변경 순서로 옮기기 위한
이행 계획서다. 앞선 문서가 책임 경계를 정했다면, 이 문서는 어떤 타입과 모듈을
어떤 순서로 만들고, 어떤 테스트가 통과해야 다음 단계로 넘어갈 수 있는지 고정한다.

기준 문서:

- [../architecture/parallel-control-plane-architecture.md](../architecture/parallel-control-plane-architecture.md)

이 계획의 핵심 목표는 세 가지다.

```text
1. parallel/supersession 상태 변경은 application single-writer loop로 모은다.
2. dispatch, retry, stale epoch, capacity 판단은 domain decision으로 내린다.
3. TUI는 operator intent와 presentation state만 가진다.
```

## 현재 문제를 구현 관점에서 다시 정의

현재 코드는 이미 여러 좋은 재료를 가지고 있다.

- `src/application/service/parallel_mode/orchestrator_loop.rs`에 dispatch tick과 wake event가 있다.
- `src/domain/parallel_mode/orchestrator.rs`에 runtime event, dispatch command, state machine 일부가 있다.
- `PlanningAuthorityPort`는 slot lease, session detail, distributor queue, dispatch command를 durable projection으로 보관한다.
- TUI는 background message를 받아 supervisor snapshot을 갱신하고, operator에게 상태를 보여준다.

문제는 재료가 없는 것이 아니라, 변경 책임이 아직 한 줄로 고정되지 않은 것이다.

- `src/adapter/inbound/tui/app/parallel_mode.rs`가 enable, readiness refresh, worker spawn, in-flight gate, status copy, dispatch wake를 함께 들고 있다.
- `src/adapter/inbound/tui/app/shell_runtime.rs`가 timer/polling 판단으로 application orchestration을 직접 깨운다.
- `ParallelModeService`는 application facade 역할을 하지만, 일부 decision과 effect 실행 경계가 같은 함수에 섞여 있다.
- domain은 타입과 작은 state machine을 제공하지만, `control-plane aggregate가 command를 받아 decision을 반환한다`는 형태는 아직 약하다.

따라서 다음 구현은 큰 재작성보다 “변경 경로를 하나씩 좁히는 이전”이어야 한다.

## 목표 아키텍처

### 최종 흐름

```text
TUI / CLI / Admin / Telegram
  -> ParallelModeControlPlaneCommand
  -> Application single-writer runtime
  -> repository/load durable control-plane snapshot
  -> domain aggregate decision
  -> repository/save durable change
  -> effect runner
  -> ParallelModeControlPlaneEvent
  -> same single-writer runtime
  -> projection update
  -> inbound adapter renders/returns response
```

### 새로 고정할 개념

| 개념 | 위치 | 역할 |
| --- | --- | --- |
| `ParallelModeControlPlaneCommand` | `src/application/service/parallel_mode` | 외부 진입점이 보내는 use case 의도 |
| `ParallelModeControlPlaneEvent` | `src/application/service/parallel_mode` | worker/effect 완료 후 runtime으로 돌아오는 사실 |
| `ParallelModeControlPlaneAggregate` | `src/domain/parallel_mode` | durable snapshot과 입력을 받아 decision 생성 |
| `ParallelModeControlPlaneDecision` | `src/domain/parallel_mode` | 저장 변경과 실행할 effect를 구조화 |
| `ParallelModeControlPlaneRuntime` | `src/application/service/parallel_mode` | command/event 직렬 처리, runtime store, effect scheduling |
| `ParallelModeControlPlaneProjection` | `Application Projection` | TUI/Admin이 읽는 현재 상태 |
| `ParallelPanelStateController` | `src/adapter/inbound/tui/app/parallel_mode` | overlay, cursor, prompt lock, loading 표시만 관리 |

초기 PR에서 이 이름을 모두 만들 필요는 없다. 다만 새 코드가 다른 이름을 쓰더라도
위 역할과 일대일로 대응해야 한다.

application command/event가 surface metadata, channel, effect id를 담아야 한다면 domain으로
내리지 않는다. domain에는 I/O와 surface 정보를 제거한 순수 입력만 전달한다.
이 문서에서 `Application Projection`은 inbound adapter가 읽는 현재 상태 view를 뜻한다.
구현은 domain projection을 감쌀 수도 있고 durable repository/read model에서 조립할 수도 있지만,
TUI/Admin/CLI가 읽는 이름은 하나로 유지한다.

## 상태 소유권 이전표

| 상태 | 현재 흔한 위치 | 목표 소유자 | 비고 |
| --- | --- | --- | --- |
| parallel mode enabled 여부 | TUI app state | application runtime store | 자동 dispatch에 영향을 주므로 UI-only가 아니다. TUI는 projection만 가진다. |
| readiness snapshot | TUI refresh path/application service | Application Projection | 계산은 service/effect, 표시 모델은 projection으로 제공한다. |
| supervisor snapshot | TUI cache/application service | Application Projection | durable repository/read model에서 조립하되 TUI cache는 마지막 렌더링 값이어야 한다. |
| in-flight refresh/wake/tick flags | `ParallelModeControlPlaneRuntime` | application runtime store | `STORE-00D`에서 이동 완료. 중복 worker spawn 방지는 runtime effect state가 판단한다. |
| last tick signature | `ParallelModeControlPlaneRuntime` | application runtime store | `STORE-00D`에서 이동 완료. wake coalescing과 같은 orchestration state다. |
| overlay open/close | TUI app state | TUI controller | 순수 presentation state다. |
| board selection/cursor | TUI app state | TUI controller | domain/application으로 올리지 않는다. |
| prompt lock 표시 | TUI app state | TUI controller | lock 원인은 Application Projection에서 읽되, 표시 상태는 TUI가 가진다. |
| dispatch command | SQLite authority runtime projection | durable repository/store | 기존 `PlanningAuthorityPort` 경로를 유지한다. |
| slot lease | lease file + SQLite projection | durable repository/store | load/save 경로는 single-writer loop에서만 호출한다. |
| session detail | runtime projection | durable repository/store | worker stream event가 직접 UI를 고치지 않게 한다. |
| wake coalescing | `ParallelModeControlPlaneRuntime`, durable dispatch command poll bridge | application runtime store | process-lifetime state이므로 DB에 저장하지 않는다. durable dispatch command는 별도 authority projection이다. |
| effect id/epoch | `ParallelModeControlPlaneRuntime` | application runtime store | `STORE-00D`에서 TUI field owner를 제거했다. stale completion drop의 기준이다. |

## 경계 규칙

### Domain

domain은 다음을 해도 된다.

- capacity와 candidate eligibility 판단
- stale epoch drop 판단
- mode off에서 pending command를 cancel해야 하는지 판단
- failed-start block이 해제 가능한지 판단
- queue head와 lease 상태를 보고 다음 dispatch decision 생성
- supervisor/readiness/projection에 필요한 순수 변환

domain은 다음을 하면 안 된다.

- thread spawn
- `mpsc` channel 소유
- DB/file/git/GitHub 호출
- `Ratatui`, `Crossterm`, `BackgroundMessage` import
- global singleton 유지

### Application Runtime

application runtime은 다음을 소유한다.

- command/event inbox
- single-writer loop
- process-lifetime runtime store
- durable repository load/save 순서
- effect runner 호출
- worker completion event 재주입
- projection invalidation

application runtime은 business rule을 직접 키우면 안 된다. `if pending && idle > 0`
같은 분기가 커지면 domain decision으로 내려야 한다.

### TUI Controller

TUI controller는 다음만 소유한다.

- overlay 열림/닫힘
- selection/cursor
- loading stage 표시
- 마지막으로 렌더링한 projection
- prompt lock의 표시 상태
- key event를 command/effect로 변환

TUI controller는 다음 함수를 직접 호출하지 않는 방향으로 줄인다.

- `claim_next_dispatch_command`
- `build_dispatch_plan`
- `run_dispatch_orchestrator_tick`
- worker launch 함수
- durable command enqueue/update 함수

## 구현 단계

### Phase 0. 현재 동작 고정

목적:

- 리팩터링 전에 현재 parallel mode의 실패 조건을 테스트 이름으로 고정한다.
- 이 단계에서는 구조를 바꾸지 않는다.

작업:

- 현재 dispatch/wake 흐름의 regression test 이름을 정리한다.
- “3개 slot 중 2개가 blocked여도 남은 1개가 진행된다” 계약을 테스트로 고정한다.
- “task가 많을 때 capacity available event가 다음 dispatch를 깨운다” 계약을 테스트로 고정한다.
- `:parallel` 반복 입력이 중복 worker를 만들지 않는지 확인한다.

완료 조건:

- 기존 동작을 설명하는 실패 재현 테스트 또는 회귀 테스트가 있다.
- 테스트가 실패한다면 어떤 계층의 책임 문제인지 문서에 메모한다.

### Phase 1. Domain Decision Seed

목적:

- application service에 섞인 순수 판단을 domain decision 함수로 옮긴다.
- I/O 없는 타입만 먼저 만든다.

후보 타입:

```rust
ParallelModeControlPlaneInput
ParallelModeControlPlaneDecision
ParallelModeDispatchEligibility
ParallelModeDispatchCapacity
ParallelModeWakeDecision
ParallelModeStaleEventDecision
```

첫 추출 후보:

- `ParallelModeRuntimeEvent`가 dispatch command를 enqueue해야 하는지
- actionable queue head가 없을 때 command를 만들지 말아야 하는지
- idle slot 수와 excluded task 목록으로 candidate를 몇 개까지 선택할지
- failed-start block이 task update 이후 해제 가능한지
- stale epoch completion을 drop할지

완료 조건:

- 새 domain test는 DB, thread, filesystem 없이 실행된다.
- application service는 같은 판단을 중복으로 보유하지 않는다.
- TUI 변경 없이도 기존 parallel tests가 통과한다.

### Phase 2. Application Runtime Store 도입

목적:

- `parallel_mode_enabled`, in-flight flag, wake coalescing, effect id를 TUI에서 application runtime으로 옮길 준비를 한다.
- 처음에는 기존 `ParallelModeService` facade를 감싼 얇은 runtime으로 시작한다.

후보 모듈:

```text
src/application/service/parallel_mode/control_plane.rs
src/application/service/parallel_mode/control_plane/runtime.rs
src/application/service/parallel_mode/control_plane/commands.rs
src/application/service/parallel_mode/control_plane/projection.rs
```

초기 책임:

- `Enable`
- `Disable`
- `RefreshSupervisor`
- `WakeOrchestrator`
- `WorkerCompleted`
- `EffectCompleted`

초기에는 내부에서 기존 service 함수를 호출해도 된다. 중요한 것은 외부 진입점이
runtime command 하나로 들어오게 만드는 것이다.

완료 조건:

- TUI가 command를 보낼 application API가 생긴다.
- runtime store는 DB에 저장하지 않는다.
- effect completion은 같은 runtime으로 event를 다시 보낸다.
- runtime tests는 fake port/repository로 직렬 처리 순서를 검증한다.

### Phase 3. TUI Parallel Panel Controller 분리

목적:

- `src/adapter/inbound/tui/app/parallel_mode.rs`에서 presentation state와 command dispatch를 분리한다.
- Flutter BLoC와 비슷한 역할은 TUI inbound adapter 내부에만 둔다.

후보 모듈:

```text
src/adapter/inbound/tui/app/parallel_mode/controller.rs
src/adapter/inbound/tui/app/parallel_mode/state.rs
src/adapter/inbound/tui/app/parallel_mode/event.rs
src/adapter/inbound/tui/app/parallel_mode/effect.rs
```

controller 입력:

```text
ParallelPanelUiEvent::CommandEntered
ParallelPanelUiEvent::ProjectionUpdated
ParallelPanelUiEvent::OverlayOpened
ParallelPanelUiEvent::OverlayClosed
ParallelPanelUiEvent::SelectionMoved
```

controller 출력:

```text
ParallelPanelUiEffect::SendCommand(...)
ParallelPanelUiEffect::SetStatusCopy(...)
ParallelPanelUiEffect::RequestRedraw
```

완료 조건:

- controller test는 `ParallelModeService` 없이 실행된다.
- controller는 durable state를 mutate하지 않는다.
- `parallel_mode.rs`는 wiring과 background message mapping 위주로 줄어든다.

### Phase 4. Background Worker Event 재배선

목적:

- worker thread 완료가 TUI state를 직접 고치지 않고 application runtime event로 돌아오게 한다.
- TUI의 `BackgroundMessage`는 projection 갱신과 status 표시만 담당하게 한다.

변경 방향:

```text
worker thread
  -> ParallelModeControlPlaneEvent::WorkerCompleted(...)
     or ParallelModeControlPlaneEvent::WorkerLaunchFailed(...)
     or ParallelModeControlPlaneEvent::WorkerStreamFailed(...)
  -> application runtime
  -> domain decision
  -> repository/projection update
  -> TUI receives ProjectionUpdated
```

주의:

- 실제 worker launch는 여전히 application effect runner의 책임이다.
- worker stream 자체는 별도 thread에서 돌 수 있다.
- thread가 직접 바꿀 수 있는 것은 channel에 event를 보내는 일뿐이다.
- thread 생성 실패, worker port I/O 실패, stream 중단, effect runner 내부 오류도
  `ParallelModeControlPlaneEvent`로 runtime에 되돌린다.
- runtime은 실패 event를 보고 lease/session detail/dispatch command 보상 갱신을 수행한다.
  실패를 TUI status copy로만 소비하면 durable state가 다음 dispatch를 막는 원인을 잃는다.

완료 조건:

- stale epoch completion은 application runtime에서 drop된다.
- TUI에는 stale drop 판단이 없다.
- worker launch/stream failure가 application runtime에서 보상 처리된다.
- capacity available event가 남은 queue dispatch를 다시 깨운다.
- blocked slot이 있어도 idle slot이 있으면 dispatch가 계속된다.

### Phase 5. Durable Store 경계 정리

목적:

- 기존 `PlanningAuthorityPort`를 무리하게 새 repository로 갈아엎지 않는다.
- 다만 application runtime에서 보는 저장소 역할을 명확히 한다.

초기 방침:

- dispatch command, slot lease, session detail, distributor queue는 기존 authority runtime projection을 계속 쓴다.
- 새 `ParallelModeControlPlaneRepository` trait은 실제 중복을 줄일 때만 만든다.
- repository를 만들 경우에도 구현체는 `PlanningAuthorityPort`를 감싼 adapter가 된다.

완료 조건:

- durable/recoverable state와 process-lifetime state가 같은 struct에 섞이지 않는다.
- in-memory 구현은 test 또는 process-local read model 용도에 한정한다.
- timer, effect id, wake coalescing은 repository로 내리지 않는다.

### Phase 6. Adapter Surface 통합

목적:

- TUI 외 inbound surface도 같은 application command를 사용할 수 있게 한다.
- CLI, admin API, Telegram이 parallel mode를 제어할 때 TUI 전용 service path를 복제하지 않는다.

작업:

- `EnableParallelMode`, `DisableParallelMode`, `RefreshParallelSupervisor`, `DispatchPendingQueue` 같은 command를 inbound-neutral DTO로 노출한다.
- TUI는 key/input을 command로 바꾸는 adapter가 된다.
- admin/CLI는 같은 command를 request/response로 감싼다.

완료 조건:

- application command handler는 TUI 타입을 import하지 않는다.
- TUI 외 surface에서 parallel mode 상태를 조회해도 같은 projection을 읽는다.
- 새 inbound가 추가되어도 domain/application decision을 복사하지 않는다.

## PR 슬라이스 제안

### PR 0. Regression Contract Lock

목적:

- 구조 변경 전에 Phase 0의 현재 동작 고정 테스트를 먼저 추가한다.
- 특히 이번 버그 계열인 “task가 많아도 하나만 진행됨”과 “blocked worktree가 남은 capacity를 막음”을 회귀 테스트로 고정한다.

소유 파일:

- `src/application/service/parallel_mode/tests/orchestrator_loop.rs`
- `src/application/service/parallel_mode/tests/pool/*`
- 필요한 경우 `src/adapter/inbound/tui/app/shell_runtime/tests/*`

검증:

- blocked worktree + idle slot dispatch regression
- capacity available event continuation regression
- repeated `:parallel` duplicate worker guard

완료 조건:

- 이 PR이 없으면 PR 1 이후 구조 변경을 시작하지 않는다.
- 테스트 이름만 봐도 어떤 operator-visible 실패를 막는지 알 수 있어야 한다.

### PR 1. Domain Decision Extraction

소유 파일:

- `src/domain/parallel_mode/orchestrator.rs`
- `src/domain/parallel_mode/tests.rs`
- 필요한 경우 `src/domain/parallel_mode/control_plane.rs`

검증:

- `cargo test domain::parallel_mode`
- 관련 application parallel tests

### PR 2. Runtime Command Facade

소유 파일:

- `src/application/service/parallel_mode/control_plane/*`
- `src/application/service/parallel_mode/orchestrator_loop.rs`
- `src/application/service/parallel_mode/tests/orchestrator_loop.rs`

검증:

- command serialization/order tests
- wake coalescing tests
- stale epoch tests

### PR 3. TUI Controller Split

소유 파일:

- `src/adapter/inbound/tui/app/parallel_mode.rs`
- `src/adapter/inbound/tui/app/parallel_mode/*`
- `src/adapter/inbound/tui/app/shell_runtime/tests/*`

검증:

- controller unit tests
- shell runtime input tests
- TUI rendering snapshot tests only if visible copy/layout changes

### PR 4. Worker Completion Event Path

소유 파일:

- `src/application/service/parallel_mode/orchestrator_loop.rs`
- `src/adapter/inbound/tui/app/turn_submission_runtime/*`
- `src/adapter/inbound/tui/app/parallel_mode.rs`

검증:

- completion-to-dispatch continuation tests
- 2 blocked worktrees + 1 active slot regression
- official completion refresh ordering tests

### PR 5. Store Boundary Cleanup

소유 파일:

- `src/application/port/outbound/planning_authority_port.rs`
- `src/adapter/outbound/db/sqlite_planning_authority_adapter/*`
- `src/application/service/parallel_mode/*`

검증:

- SQLite runtime projection tests
- in-memory/fake repository tests
- reset/cleanup/recovery tests

## 테스트 계약

필수 domain tests:

- actionable queue head가 없으면 dispatch command를 만들지 않는다.
- mode disabled event는 pending command를 진행시키지 않는다.
- idle capacity보다 많은 task가 있어도 capacity만큼만 candidate가 선택된다.
- leased/queued task는 candidate에서 제외된다.
- failed-start block은 task update 전까지 유지된다.
- stale epoch event는 effect를 만들지 않는다.

필수 application tests:

- command는 입력 순서대로 처리된다.
- worker completion event는 runtime inbox로 재진입한다.
- in-flight effect가 있으면 중복 refresh/tick을 만들지 않는다.
- effect completion 뒤 projection invalidation이 발생한다.
- 2개 blocked worktree가 있어도 idle slot이 있으면 다음 task가 dispatch된다.
- capacity available event가 durable command를 깨운다.

필수 TUI tests:

- `:parallel` 입력은 application command effect만 만든다.
- `:parallel off`는 local rendering state를 닫고 application disable command를 보낸다.
- loading/prompt lock은 projection 상태에 맞춰 표시된다.
- selection/cursor 변경은 service를 호출하지 않는다.
- stale/drop/capacity 판단이 TUI test에 등장하지 않는다.

## 코드 리뷰 체크리스트

리뷰어는 parallel control-plane PR에서 다음을 먼저 확인한다.

- `src/domain`이 `application`, `adapter`, `std::thread`, `mpsc`, `ratatui`, `crossterm`을 import하지 않는다.
- TUI가 `claim_next_dispatch_command` 또는 durable queue mutation을 직접 호출하지 않는다.
- application runtime store와 repository/store가 같은 struct로 합쳐지지 않는다.
- 새 in-memory map이 생겼다면 test fake인지, process-lifetime runtime store인지, repository 구현체인지 이름으로 구분된다.
- background worker는 state를 직접 mutate하지 않고 event만 보낸다.
- 새 command/event/projection 타입은 workspace identity를 명시적으로 포함한다.
- stale epoch 또는 owner token check가 사라지지 않았다.

## 금지할 빠른 길

다음 방식은 구현이 빨라 보여도 장기적으로 같은 버그를 반복한다.

- TUI에 큰 BLoC를 만들고 application orchestrator와 나란히 orchestration을 수행한다.
- `ParallelModeService`에 더 큰 `if/else`를 추가해 domain extraction을 미룬다.
- 전역 singleton aggregate를 두고 여러 thread가 lock으로 mutate한다.
- process-lifetime runtime state를 SQLite repository에 넣어 durable state처럼 다룬다.
- dispatch command와 worker launch를 TUI background message에서 직접 이어붙인다.
- controller가 다른 controller를 직접 구독해 business flow를 만든다.

## 완료 정의

이 마이그레이션은 다음 조건을 만족하면 완료로 본다.

- parallel/supersession 상태 변경 진입점이 application command로 통일된다.
- TUI는 operator intent, rendering state, projection 표시만 담당한다.
- domain decision tests가 dispatch/capacity/retry/stale 정책을 DB 없이 설명한다.
- application runtime tests가 command 직렬화와 worker completion 재진입을 설명한다.
- blocked worktree가 있어도 남은 capacity가 계속 사용된다.
- task가 많을 때 dispatch가 하나만 진행되는 회귀가 테스트로 막힌다.
- CLI/admin/Telegram이 같은 application service를 재사용할 수 있는 표면이 생긴다.
