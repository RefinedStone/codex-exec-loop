# Parallel Control Plane Architecture

## 목적

이 문서는 parallel/supersession 흐름을 다시 설계하기 위한 기준 문서다. 목표는
코드 변경 전에 `TUI`, `application single-writer gate`, `domain aggregate`,
`repository/store`의 책임을 분명히 고정해, 이후 LLM이나 사람이 수정할 때 같은
정책을 여러 계층에 중복 구현하지 않게 만드는 것이다.

이 문서는 `new/docs` 구조 재설계 문서군의 첫 reference architecture이기도 하다.
parallel control-plane은 thread, worktree, durable command, worker stream, TUI
presentation state, process-lifetime runtime state가 한 흐름에 모이는 가장 까다로운
영역이다. 따라서 여기서 정한 책임 경계는 parallel 전용 예외가 아니라 repository
전체 구조 재설계의 검증 모델로 사용한다.

핵심 결론은 단순하다.

```text
TUI는 의도를 보낸다.
Application control-plane gate는 변경을 한 줄로 직렬화한다.
Domain aggregate는 규칙과 상태 전이를 판단한다.
Repository/store는 현재 상태의 단일 진실을 제공한다.
```

R6 결정으로 현재 구현은 queue-backed actor loop가 아니라
mutex-serialized synchronous facade를 single-writer gate로 둔다. 이 문서에서
`loop`라는 표현은 mailbox actor를 필수로 뜻하지 않고, command/event가 하나의
runtime reducer를 통과한다는 책임 경계를 뜻한다. 별도 actor queue는
ordering/backpressure/shutdown 문제가 현재 regression으로 제어되지 않을 때만 다시
검토한다.

## 문제 정의

현재 parallel 흐름은 여러 책임이 한 흐름 안에서 겹치기 쉽다.

- TUI가 overlay 표시, prompt lock, supervisor refresh, dispatch wake, status copy를 함께 다룬다.
- background thread 결과가 TUI 상태에 직접 가까운 형태로 돌아온다.
- application orchestrator loop가 이미 있지만, 일부 정책 판단은 TUI와 application service에 나뉘어 있다.
- durable state와 process-lifetime state, UI-only state의 경계가 명확하지 않다.
- worktree, slot lease, dispatch command, post-turn continuation이 섞이면 작은 수정도 동시성 버그로 번지기 쉽다.

이 문제의 핵심은 “BLoC가 부족하다”가 아니라 “단일 도메인 개념의 소유권이 불명확하다”이다. 따라서 UI 쪽에 큰 BLoC를 추가하는 것만으로는 해결되지 않는다. 오히려 TUI BLoC와 application orchestrator가 둘 다 orchestration을 하게 되면 두 번째 오케스트레이터가 생긴다.

## 설계 원칙

### 1. Domain Entity Singleton을 만들지 않는다

도메인상 하나뿐인 개념이 필요하더라도 프로세스 전역 singleton 객체로 보관하지 않는다.

나쁜 방향:

```text
global ParallelControlPlane object
  -> 여러 계층이 직접 mutate
  -> workspace 분리, 테스트 reset, 동시성 제어가 흐려짐
```

좋은 방향:

```text
ParallelModeControlPlaneAggregate(workspace_id)
  -> repository.load(workspace_id)
  -> domain method / decision
  -> repository.save(...)
```

즉 “하나뿐인 개념”은 singleton이 아니라 고정 identity를 가진 aggregate root로 모델링한다.

### 2. 모든 변경은 Single-Writer Application Gate를 통과한다

parallel mode는 worktree, slot lease, dispatch command, worker stream, completion refresh가 얽혀 있다. 따라서 mutable control-plane state는 여러 thread에서 직접 바꾸면 안 된다.

application layer에 single-writer gate를 둔다. 현재 구현 이름은
`ParallelModeControlPlaneHandle`과 `ParallelModeControlPlaneRuntime`이며, host는
mutex로 controller를 한 번에 하나의 command/event만 처리하게 한다.

```text
command/event 수신
  -> repository/load
  -> domain decision
  -> repository/save
  -> effect 실행
  -> completion event가 같은 gate로 재진입
```

이 gate는 “정책의 주인”이 아니다. 순서, transaction, port 호출, worker launch 같은
실행 책임을 가진다. durable dispatch command table은 backpressure boundary이고,
runtime store는 in-flight effect, wake coalescing, epoch stale-drop만 process-local로
관리한다.

### 3. Domain이 정책의 주인이다

thread, DB, git, app-server, TUI가 필요 없는 판단은 domain으로 간다.

예시:

- 이 task를 dispatch할 수 있는가
- 현재 capacity에서 몇 개의 task를 시작할 수 있는가
- stale epoch 결과를 버려야 하는가
- slot capacity available 이벤트가 다음 dispatch를 만들어야 하는가
- blocked worktree 상태에서 retry를 보류할 것인가
- failed-start block이 task update 이후 해제 가능한가

application gate 안에서 이런 판단이 `if/else`로 커지면 domain이 약해진다.
application gate는 domain decision을 실행하는 coordinator여야 한다.

### 4. TUI는 Presentation State만 가진다

TUI는 operator intent와 render state만 관리한다.

TUI가 가져도 되는 상태:

- supersession overlay가 열렸는가
- board에서 선택된 row/zone
- loading placeholder 표시 여부
- prompt 입력 lock 여부
- 마지막으로 표시한 supervisor snapshot
- 마지막 status line

TUI가 가지면 안 되는 판단:

- slot capacity 계산
- dispatch candidate 선택
- worker launch 여부
- durable dispatch command claim
- stale failed-start block 정책
- distributor retry 정책

### 5. Repository는 반드시 DB일 필요가 없다

repository/store는 “domain object collection처럼 보이는 접근 계층”이다. 구현은 DB일 수도 있고 in-memory map일 수도 있다.

단, 저장 위치는 state 성격에 따라 나눈다.

| State 종류 | 소유 계층 | 예시 |
| --- | --- | --- |
| UI-only state | TUI controller | overlay, cursor, loading display, prompt lock |
| Process-lifetime shared state | application runtime store | wake coalescing, in-flight effect id, poll timer |
| Durable/recoverable state | repository/store | dispatch command, slot lease, session detail, distributor queue |
| Domain invariant state | domain aggregate | eligibility, capacity decision, retry/block reason |

DB에 저장할 필요 없는 shared state는 application runtime store에서 관리한다. in-memory repository는 repository가 소유한 aggregate/read model의 process-local 구현일 때만 사용하고, timer나 effect id처럼 도메인 규칙과 무관한 orchestration state는 repository로 내리지 않는다. UI-only state는 TUI controller에 머물며, 복구가 필요한 state는 반드시 durable store로 간다.

## 레이어 책임

### TUI Adapter

위치:

```text
src/adapter/inbound/tui
```

책임:

- terminal key/background message를 operator intent로 변환
- UI-only state 관리
- read model/snapshot 렌더링
- status copy 표시
- application command enqueue

비책임:

- durable state 변경
- worker launch
- slot capacity 판단
- queue/distributor 정책 판단

### Parallel Panel State Controller

TUI 내부의 얇은 상태 controller다. Flutter BLoC에 가장 가까운 위치지만, business orchestration을 소유하지 않는다.

역할:

- `ParallelPanelUiEvent`를 받는다.
- `ParallelPanelUiState`를 갱신한다.
- 필요한 경우 `ParallelPanelUiEffect::SendCommand(...)`를 반환한다.

예상 책임:

```text
overlay open/close
loading placeholder
selection/cursor
last visible snapshot
last visible status
prompt lock projection
```

### Application Single-Writer Gate

위치:

```text
src/application/service/parallel_mode
```

역할:

- parallel command/event를 한 줄로 직렬 처리
- repository/store에서 현재 상태 load
- domain aggregate에 command 적용
- decision 결과 저장
- port effect 실행
- worker/background completion을 같은 gate의 command/event로 환원

중요한 제약:

- application gate는 domain state를 직접 소유하는 singleton이 아니다.
- application gate는 business rule을 직접 판단하지 않는다.
- application gate는 side effect와 ordering의 주인이다.
- 현재 gate는 mutex facade다. mailbox actor가 아니다.

### Domain Aggregate

위치:

```text
src/domain/parallel_mode
```

후보 이름:

```text
ParallelModeControlPlaneAggregate
ParallelModeControlPlaneCommand
ParallelModeControlPlaneDecision
ParallelModeControlPlaneEffect
ParallelModeControlPlaneEvent
```

역할:

- workspace 단위 control-plane 상태 전이
- invariant 보호
- dispatch/retry/capacity/stale epoch decision
- application이 실행할 effect를 구조화해서 반환

domain은 I/O를 하지 않는다. DB, git, filesystem, TUI, app-server를 모른다.

### Repository / Store

역할:

- aggregate/read model을 load/save
- durable state와 runtime state의 source of truth 제공
- tests에서 in-memory implementation으로 대체 가능해야 함

구현 예시:

```text
SqliteParallelModeControlPlaneRepository
InMemoryParallelModeControlPlaneRepository
```

in-memory repository는 가능하지만 mutation 경로는 single-writer gate로 제한한다.

## Command Flow

### Operator가 `:parallel`을 입력한 경우

```text
TUI key/input
  -> ParallelPanelUiEvent::ParallelCommandEntered
  -> ParallelPanelStateController updates UI loading state
  -> ParallelPanelUiEffect::SendCommand(EnableOrRefreshParallelMode)
  -> Application single-writer gate
  -> repository.load(workspace_id)
  -> aggregate.decide(command, input_snapshot)
  -> repository.save(next_state)
  -> effect runner executes readiness/reconcile/refresh
  -> completion event returns to gate
  -> supervisor snapshot refreshed in repository/read model
  -> TUI observes change and renders snapshot
```

### Worker completion이 다음 dispatch를 유발하는 경우

```text
worker thread completion
  -> completion event
  -> Application single-writer gate
  -> repository/load durable queue + leases
  -> aggregate decides whether slot capacity can dispatch next task
  -> repository/save command/outcome
  -> effect runner launches worker if decision says so
  -> TUI receives snapshot/status only
```

TUI는 이 흐름에서 “다음 dispatch가 가능한가”를 판단하지 않는다.

## 금지 패턴

### TUI Policy Branch

```text
TUI checks idle slot count
TUI decides dispatch candidate
TUI starts worker
```

금지한다. TUI는 intent만 보낸다.

### Application Policy Blob

```text
orchestrator_loop {
  if pending && idle_slots > 0 && !blocked && epoch_ok && ...
}
```

이 형태가 커지면 domain이 약해진다. 판단은 aggregate/domain decision으로 이동한다.

### Global Domain Singleton

```text
static ParallelModeControlPlaneAggregate
```

금지한다. workspace-scoped aggregate를 repository에서 load/save한다.

### Repository 우회 Mutation

store나 runtime projection을 여러 계층에서 직접 mutate하면 single source of truth가 깨진다. 변경은 command processor를 통해서만 수행한다.

### Controller 간 직접 의존

TUI controller나 BLoC가 다른 controller/BLoC를 직접 구독해 business flow를 이어붙이면 coupling이 커진다. 공유 상태는 repository stream/read model 또는 application command/event로 연결한다.

## 후속 구현 단계

### Phase 1. 문서 확정

- 이 문서를 기준으로 parallel control-plane의 계층 책임을 합의한다.
- 기존 `docs/design` 문서와 연결할지는 별도 PR에서 결정한다.

### Phase 2. Domain Decision 도입

- 기존 `ParallelModeOrchestratorStateMachine` 주변에 control-plane decision 타입을 추가한다.
- 먼저 I/O 없는 판단만 옮긴다.
- aggregate unit test를 작성한다.

### Phase 3. Application Gate 정리

- `orchestrator_loop.rs`와 control-plane host를 command processor 관점으로 정리한다.
- policy `if/else`를 domain decision 호출로 대체한다.
- durable command claim/update와 worker launch는 application에 남긴다.

### Phase 4. TUI State 축소

- `parallel_mode.rs`에서 UI-only state와 application command dispatch를 분리한다.
- `ParallelPanelStateController`는 rendering/prompt/overlay state만 가진다.
- background completion은 typed event로만 들어오게 한다.

### Phase 5. Runtime Store 분류

- DB 복구가 필요한 상태와 process-lifetime 상태를 명확히 나눈다.
- process-lifetime 상태가 필요하면 application runtime store로 만들고 single-writer gate만 mutate하게 한다.

## 테스트 전략

문서 이후 구현 PR은 다음 테스트를 요구한다.

- domain aggregate decision unit tests
- stale epoch/workspace drop tests
- slot capacity continuation tests
- wake coalescing tests
- command serialization tests
- existing parallel dispatch/recovery flow regression tests
- TUI panel state controller tests

테스트 원칙:

```text
domain decision은 thread/DB 없이 테스트한다.
application gate는 fake repository/port로 직렬 처리 순서를 테스트한다.
TUI는 service 없이 UI state와 effect emission만 테스트한다.
```

## 참고 기준

- Flutter app architecture는 repository를 single source of truth로 보고, app-wide lifecycle state도 repository가 관리할 수 있다고 설명한다.
- Bloc architecture는 UI와 business logic/data layer를 분리하고, Bloc이 injected repository를 통해 정보를 받도록 권장한다.
- DDD aggregate root는 invariant의 단일 진입점이며, repository는 aggregate root 단위로 정의한다.
- application service는 use case orchestration과 side effect를 조율하지만 business rule의 주인이 아니다.

참고 링크:

- Flutter App Architecture: https://docs.flutter.dev/app-architecture/guide
- Flutter Architecture Concepts: https://docs.flutter.dev/app-architecture/concepts
- Bloc Architecture: https://bloclibrary.dev/architecture/
- Microsoft DDD Application/Domain Guidance: https://learn.microsoft.com/en-us/dotnet/architecture/microservices/microservice-ddd-cqrs-patterns/ddd-oriented-microservice
- Microsoft Repository per Aggregate Guidance: https://learn.microsoft.com/en-us/dotnet/architecture/microservices/microservice-ddd-cqrs-patterns/infrastructure-persistence-layer-design
