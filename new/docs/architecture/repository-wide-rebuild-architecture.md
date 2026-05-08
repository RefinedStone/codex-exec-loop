# Repository-Wide Rebuild Architecture

## 목적

이 문서는 Akra repository 전체 구조를 대폭 재설계하기 위한 최상위 기준 문서다.
기존 `docs/*`가 현재 동작, 운영, 과거 계획, 점진적 구조 정리를 설명한다면,
`new/docs/*`는 현재 구조를 그대로 보존하기 위한 문서가 아니다. `new/docs/*`는
버그를 반복해서 만드는 구조를 끊고, 이후 코드가 따라야 할 새 기준선을 세우는
문서 체계다.

핵심 목표는 하나다.

```text
모든 코드 영역이 같은 구조 언어를 사용하게 한다.
```

여기서 “모든 코드 영역”은 이 repository 내부의 기능 영역을 뜻한다. TUI,
planning, parallel mode, conversation runtime, admin API, CLI, Telegram bot,
outbound adapter, domain/application/port 계층을 모두 포함한다.

## 기준 문서의 지위

이 문서는 새 구조의 상위 원칙을 정의한다. 실제 구현으로 내려갈 때는 다음 문서를
첫 번째 reference architecture로 삼는다.

- [parallel-control-plane-architecture.md](./parallel-control-plane-architecture.md)
- [../plan/parallel-control-plane-migration-plan.md](../plan/parallel-control-plane-migration-plan.md)
- [store-and-runtime-state-architecture.md](./store-and-runtime-state-architecture.md)

parallel control-plane은 새 구조의 신뢰 기준으로 적합하다.

- 실제 장애가 발생한 영역이다.
- TUI state, application orchestration, domain policy, durable store가 모두 얽혀 있다.
- thread, worktree, worker stream, dispatch command, retry, stale epoch, prompt lock이 함께 작동한다.
- 이 영역에서 통하는 구조는 planning, conversation, admin, CLI에도 일반화할 수 있다.

따라서 parallel 문서는 “parallel만의 특수 문서”가 아니다. 이 repository가 앞으로
따라야 할 구조 원칙을 가장 압축적으로 검증하는 첫 사례다.

## 현재 구조가 만드는 문제

현재 구조의 문제는 파일 크기만이 아니다. 더 큰 문제는 같은 판단이 여러 계층에
나뉘어 들어가는 것이다.

- TUI가 operator intent, presentation state, background worker 결과, application wake를 함께 다룬다.
- application service가 use case orchestration과 domain policy를 함께 키운다.
- domain이 존재하지만 모든 기능 영역에서 decision의 주인이 되지는 못한다.
- repository, runtime store, UI cache, durable projection의 경계가 흐려진다.
- inbound surface마다 비슷한 use case 판단을 다시 구현할 위험이 있다.
- 테스트는 많지만 “어느 계층 계약을 검증하는지”가 흐려져 변경 안정성이 떨어진다.

새 구조는 이 문제를 “조금 더 나은 모듈 분리”로 보지 않는다. 상태 변경 경로,
정책 소유권, side effect 실행 책임을 같은 방식으로 재정의한다.

## 표준 레이어 모델

Akra의 새 구조는 다음 방향을 따른다.

```text
adapter/inbound
  -> application/service
  -> domain

application/service
  -> application/port/outbound
  -> adapter/outbound
```

의존 방향은 기존 hexagonal 원칙과 같지만, 책임 정의는 더 엄격해진다.

| Layer | 책임 | 금지 |
| --- | --- | --- |
| `adapter/inbound/*` | 입력 해석, UI-only state, request/response mapping, rendering | domain policy, durable mutation, worker launch decision |
| `application/service/*` | use case, single-writer runtime, transaction order, port effect orchestration | business invariant 직접 성장, surface별 정책 복제 |
| `domain/*` | aggregate, policy, invariant, pure decision, state transition | I/O, thread, channel, `application/*` 및 `adapter/*` 의존성 전체 |
| `application/port/outbound/*` | 외부 boundary trait과 request/response contract | concrete adapter detail |
| `adapter/outbound/*` | DB/git/github/filesystem/app-server/telegram 구현과 mapping | use case policy, domain rule |

## 표준 상태 분류

모든 기능 영역은 state를 먼저 분류한 뒤 구현 위치를 정한다.

| State 종류 | 소유자 | 예시 |
| --- | --- | --- |
| UI-only state | inbound controller | overlay, cursor, selected row, local loading display |
| Process-lifetime runtime state | application runtime store | in-flight effect id, wake coalescing, poll timer, epoch gate |
| Durable/recoverable state | repository/store | queue item, command, lease, session detail, task authority |
| Domain invariant state | domain aggregate/value | eligibility, capacity, retry policy, validation result |
| Application Projection | application read model | TUI/admin/CLI가 읽는 현재 상태 view |

중요한 규칙:

- UI-only state는 repository로 내리지 않는다.
- process-lifetime runtime state는 durable store로 위장하지 않는다.
- durable/recoverable state는 thread나 TUI가 직접 변경하지 않는다.
- domain invariant는 application `if/else`로 흩어지지 않는다.
- inbound adapter는 Application Projection을 읽고 표시한다.

## Bounded Context 기준

Akra의 bounded context는 UI surface가 아니라 제품 기능 단위로 나눈다.

우선 context:

- `parallel_mode`
- `planning`
- `conversation`
- `session_browser`
- `startup`
- `github_review`

TUI, CLI, admin API, Telegram bot은 bounded context가 아니다. 이들은 inbound
adapter다. 같은 planning use case가 TUI와 admin API에서 필요하면, 두 surface는
각자 request/input mapping만 하고 application command 또는 facade를 공유해야 한다.

## 표준 흐름

### 사용자 입력 흐름

```text
inbound event/request
  -> inbound controller maps intent
  -> application command
  -> application runtime/use case
  -> repository/load if needed
  -> domain decision
  -> repository/save if needed
  -> outbound effect if needed
  -> Application Projection update
  -> inbound renders from projection or returns response
```

### background/effect 완료 흐름

```text
effect runner or worker thread
  -> application event
  -> same application runtime/use case
  -> stale/owner/epoch gate
  -> domain decision
  -> compensating save or next effect
  -> Application Projection update
```

thread, worker, timer는 직접 UI나 durable state를 바꾸지 않는다. 성공과 실패 모두
application event로 돌아와야 한다.

## 기능 영역별 적용 방향

### Parallel Mode

parallel mode는 새 구조의 첫 reference다.

- `ParallelModeControlPlaneRuntime`은 application single-writer loop 역할을 한다.
- dispatch, capacity, stale epoch, retry 판단은 domain decision으로 내려간다.
- TUI parallel panel은 presentation controller로 축소한다.
- dispatch command, slot lease, session detail, distributor queue는 durable store로 유지한다.
- wake coalescing, in-flight effect, epoch gate는 application runtime store가 가진다.

### Planning

planning은 다음 순위의 구조 재정렬 대상이다.

- authoring, runtime, repair, worker, admin, task mutation의 경계를 명확히 한다.
- semantic validation, queue ordering, proposal classification은 domain에 둔다.
- prompt assembly, hidden worker retry, workspace sync는 application에 둔다.
- TUI/admin/CLI는 planning internals를 직접 호출하지 않고 application facade/command를 공유한다.
- file workspace와 SQLite authority mapping은 outbound adapter에 둔다.

### TUI Shell And Conversation

TUI는 화면의 중심이지만 정책의 중심이 아니다.

- shell input은 intent로 변환한다.
- conversation lifecycle과 automation lifecycle을 분리한다.
- background message는 Application Projection 갱신과 status 표시로 제한한다.
- prompt lock, overlay, selection, cursor는 UI-only state로 유지한다.
- continuation, auto-follow, planning handoff policy는 application/domain으로 이동한다.

### Admin, CLI, Telegram

이들은 별도 business logic 소유자가 아니다.
상세 기준은 [inbound-surface-unification-architecture.md](./inbound-surface-unification-architecture.md)를 따른다.

- 각 surface는 request parsing, auth/session/context mapping, response rendering을 맡는다.
- 같은 기능은 같은 application command/use case를 호출한다.
- surface별 copy나 rendering은 adapter에 남기되 policy는 복제하지 않는다.

### Outbound Adapters

outbound adapter는 infrastructure detail을 숨기는 구현체다.

- DB row shape, filesystem path shape, git command, GitHub API, app-server protocol mapping은 adapter에 둔다.
- use case 순서와 retry policy는 application/domain에 둔다.
- 새 외부 boundary가 실제로 필요할 때만 application port를 추가한다.

## 우선순위

### P0. Parallel Control-Plane 신뢰 강화

- existing regression을 먼저 고정한다.
- blocked worktree가 남은 capacity를 막지 않는지 검증한다.
- task가 많을 때 dispatch가 하나만 진행되는 회귀를 막는다.
- parallel architecture 문서를 repo-wide 기준의 reference로 유지한다.

### P1. Planning 구조 재정렬

- planning authoring/runtime/repair/worker/admin/task mutation 경계를 문서화한다.
- planning Application Projection과 durable authority state의 차이를 분명히 한다.
- TUI/admin/CLI가 공유할 application facade/command 표면을 정의한다.

### P2. TUI Shell And Conversation 구조 재정렬

- TUI controller와 application runtime boundary를 정한다.
- conversation lifecycle, post-turn automation, planning/parallel handoff를 분리한다.
- UI-only state와 application state를 표로 고정한다.

### P3. Inbound Surface 통일

- CLI, admin API, Telegram이 application use case를 복제하지 않도록 한다.
- surface별 request/response mapping 규칙을 통일한다.
- 세부 기준 문서는 [inbound-surface-unification-architecture.md](./inbound-surface-unification-architecture.md)이다.

### P4. Outbound And Store 경계 통일

- repository/store와 runtime store를
  [store-and-runtime-state-architecture.md](./store-and-runtime-state-architecture.md) 기준으로 분리한다.
- adapter mapping과 application orchestration을 분리한다.
- SQLite/file/git/GitHub/app-server 구현체가 policy를 갖지 않게 한다.

### P5. Test And Docs 체계 통일

- domain decision tests, application ordering tests, adapter mapping tests를 분리한다.
- `new/docs`의 architecture와 plan 문서를 새 구조의 기준으로 유지한다.
- 기존 `docs/*`는 현재 동작과 운영/역사 문서로 남기고, 새 구조 약속은 `new/docs/*`에 둔다.

## 금지 패턴

### Inbound Policy Branch

```text
TUI/Admin/CLI checks business state
TUI/Admin/CLI decides next action
TUI/Admin/CLI mutates durable state
```

금지한다. inbound는 intent를 만들고 projection을 표시한다.

### Application Policy Blob

```text
service {
  if ready && pending && !blocked && epoch_ok && ...
}
```

비즈니스 규칙이나 invariant 판단이 포함되면 domain decision으로 이동한다.

### Runtime Store As Repository

process-lifetime state를 durable repository처럼 다루지 않는다. poll timer, in-flight
effect id, wake coalescing은 복구 대상이 아니다.

### Repository As Runtime Store

durable store 안에 thread/timer/effect 상태를 넣지 않는다. 복구 가능한 state와
실행 중인 process state가 섞이면 재시작과 테스트가 모두 어려워진다.

### Surface-Specific Use Case Duplication

TUI용 planning enable, admin용 planning enable, CLI용 planning enable을 따로 만들지
않는다. surface는 달라도 application use case는 하나여야 한다.

### Global Domain Singleton

단 하나의 개념이 필요하더라도 global mutable singleton으로 두지 않는다. 고정
identity를 가진 aggregate를 repository/store에서 load/save한다.

## 구현 문서 작성 순서

이 문서 이후 `new/docs`의 다음 작업은 아래 순서로 진행한다.

1. `new/docs/plan/repository-wide-rebuild-roadmap.md`
2. `new/docs/architecture/planning-control-plane-architecture.md`
3. `new/docs/architecture/tui-application-boundary-architecture.md`
4. `new/docs/architecture/inbound-surface-unification-architecture.md`
5. `new/docs/architecture/store-and-runtime-state-architecture.md`

각 문서는 새 구조를 구현하기 위한 기준이어야 한다. 현재 구조를 있는 그대로
해설하는 문서가 되면 안 된다.

## 완료 기준

repo-wide rebuild architecture는 다음 조건을 만족할 때 신뢰할 수 있다.

- 모든 기능 변경이 어느 layer에 들어가야 하는지 문서만 보고 판단 가능하다.
- parallel, planning, TUI, admin, CLI, Telegram이 같은 application/domain 언어를 쓴다.
- background thread와 worker failure가 직접 UI/durable state를 mutate하지 않는다.
- domain decision은 I/O 없이 테스트 가능하다.
- application runtime은 command/event ordering과 side effect 실행을 테스트한다.
- adapter tests는 mapping과 rendering만 검증한다.
- `new/docs`가 새 구조의 source of truth 역할을 한다.
