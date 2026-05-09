# Repository-Wide Rebuild Roadmap

## 문서 지위

`new/docs`에서 현재 읽어야 하는 문서는 세 개뿐이다.

- [../architecture/parallel-control-plane-architecture.md](../architecture/parallel-control-plane-architecture.md)
- [parallel-control-plane-migration-plan.md](./parallel-control-plane-migration-plan.md)
- 이 문서

그 외 `new/docs/architecture/*`, `new/docs/plan/*` 문서는 제거했다. 완료된
inventory, 중복 architecture, 과거 migration memo는 Git history로 충분하다. 현재
구현 판정과 다음 작업은 이 문서만 따른다.

## 고정 원칙

레이어 책임은 아래로 고정한다.

| Layer | 해야 할 일 | 하면 안 되는 일 |
| --- | --- | --- |
| `adapter/inbound/*` | 입력 해석, auth/context mapping, UI-only state, rendering | domain policy, durable mutation, worker launch decision |
| `application/service/*` | command/use case, ordering, transaction, port effect orchestration | surface별 정책 복제, invariant를 큰 `if/else`로 계속 키우기 |
| `domain/*` | invariant, pure decision, state transition rule | I/O, thread/channel, adapter/application 의존 |
| `application/port/outbound/*` | 외부 boundary trait과 request/result | concrete DB/git/filesystem detail |
| `adapter/outbound/*` | DB/git/filesystem/GitHub/app-server mapping | use case policy, domain rule |

state owner는 먼저 분류한 뒤 이동한다.

| State | Owner |
| --- | --- |
| overlay, cursor, selection, local editor buffer | TUI |
| visible projection cache | inbound adapter cache |
| in-flight effect id, wake coalescing, poll timer, epoch gate | application runtime |
| task authority, dispatch command, lease, session detail, distributor queue | durable store |
| eligibility, capacity, retry, validation, stale event decision | domain |

## 현재 판정

전체 상태: `partial`

진행된 것은 다시 문서화하지 않는다. 현재 남은 문제만 관리한다.

| 영역 | 남은 문제 |
| --- | --- |
| Parallel control-plane | control-plane은 아직 queue actor가 아니라 mutex facade다. |
| TUI boundary | TUI production state에 raw application service handle debt가 남아 있다. |
| Inbound composition | CLI/admin/Telegram/TUI entrypoint가 production outbound adapter wiring을 아직 직접 들고 있다. |
| Store/runtime | process-lifetime state와 durable recovery requirement를 기능별로 다시 판정해야 한다. |
| Tests | source-string guard가 behavior test를 대체하는 곳이 있다. 새 slice마다 behavior test를 우선한다. |

## 실행 Backlog

### R5. Store/Runtime Recovery Boundary 재판정

상태: `ready`

대상:

- `src/application/port/outbound/*`
- `src/adapter/outbound/db/*`
- `src/adapter/outbound/filesystem/*`
- `src/adapter/outbound/git/*`
- parallel/planning runtime store modules

문제:

- 어떤 state가 process-lifetime이어도 되는지, 어떤 state가 recoverable이어야 하는지
  기능별로 다시 판정해야 한다.
- 과거 inventory 문서는 제거했으므로 source code와 behavior test가 authority여야 한다.

해야 할 일:

- dispatch command, lease, session detail, distributor queue, planning authority, active turn
  correlation을 recovery requirement 기준으로 분류한다.
- durable state mutation이 adapter/TUI/thread에서 직접 일어나지 않는지 확인한다.
- 필요한 경우 port boundary를 추가하거나 기존 port request/result를 좁힌다.

완료 조건:

- recoverable state는 durable store 또는 outbound boundary 뒤에 있다.
- process-lifetime state는 restart loss가 허용되는 이유가 test 또는 code comment로 설명된다.
- recovery/mirror-loss/stale completion regression이 통과한다.

검증:

```bash
cargo test adapter::outbound::db::sqlite_planning_authority_adapter
cargo test parallel_mode
cargo test planning
```

### R6. Control-Plane Event Loop 전환 여부 결정

상태: `blocked`

선행:

- R5

현재 판단:

- 지금 구조는 queue-backed actor loop가 아니라 mutex-serialized synchronous facade다.
- 동기 facade는 당장 유지한다. 먼저 durable/process-lifetime state 경계를 확정한다.

해야 할 일:

- R3 이후에도 background completion ordering, backpressure, shutdown, stale event
  문제가 남는지 측정한다.
- 문제가 남으면 queue-backed single consumer loop 설계를 별도 slice로 작성한다.
- 문제가 충분히 제어되면 mutex facade를 명시적 설계 선택으로 문서화한다.

완료 조건:

- actor loop 필요 여부가 구현 근거와 regression으로 결정된다.
- 선택한 구조가 `parallel-control-plane-architecture.md`와 충돌하면 해당 parallel 기준 문서를 갱신한다.

## 문서 운영 규칙

- 새 `new/docs` 문서를 추가하지 않는다.
- 새 작업은 이 문서의 R-slice를 갱신한다.
- 완료된 내용은 이 문서에서 제거한다.
- 아직 구현되지 않은 내용만 남긴다.
- `parallel-control-plane-architecture.md`와 `parallel-control-plane-migration-plan.md`는 별도 기준 문서로 유지한다.

## Worker 보고 형식

```text
Slice:
Branch:
Implemented:
Still partial:
Changed files:
Verification:
Follow-up:
```

`Still partial`이 비어 있지 않으면 해당 영역을 완료로 보지 않는다.
