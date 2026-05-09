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

| 영역 | 현재 구현된 것 | 아직 안 된 것 |
| --- | --- | --- |
| Parallel control-plane | TUI에서 `projection_ready`, refresh/reconcile, dispatch readiness, stale epoch 판단을 대부분 제거했다. | post-turn bridge에 auto-follow ownership이 남아 있다. control-plane은 queue actor가 아니라 mutex facade다. |
| TUI boundary | shell state 일부와 parallel panel은 controller/projection으로 나뉘었다. | `NativeTuiApp`이 service wiring과 runtime bridge를 많이 들고 있다. `QueueAutoPrompt` ownership이 TUI에 남아 있다. |
| Planning | projection/facade/domain policy seed가 있다. | post-turn executor와 TUI runtime bridge가 planning workflow를 직접 조합한다. 모든 route가 하나의 application command로 통일되지는 않았다. |
| Inbound surfaces | CLI/admin/Telegram 일부 vocabulary가 공유된다. | route별로 같은 기능이 같은 application request/result를 쓰는지 재감사가 필요하다. |
| Store/runtime | SQLite authority, runtime projection, mirror I/O boundary 일부가 정리됐다. | process-lifetime state와 durable recovery requirement를 기능별로 다시 판정해야 한다. |
| Tests | regression anchor가 여럿 있다. | source-string guard가 behavior test를 대체하는 곳이 있다. 새 slice마다 behavior test를 우선한다. |

## 실행 Backlog

### R2. Post-Turn Automation Effect Ownership 정리

상태: `ready`

대상:

- `src/adapter/inbound/tui/app/post_turn_automation.rs`
- `src/adapter/inbound/tui/app/conversation_runtime.rs`
- `src/application/service/post_turn_decision.rs`
- 필요 시 `src/application/service/parallel_mode/control_plane/*`

문제:

- TUI target이 `ConversationRuntimeEffect::QueueAutoPrompt`를 직접 검사하고 제거한다.
- pending task-intake path도 generic auto prompt를 TUI에서 suppress한다.
- post-turn continuation은 control-plane으로 올라갔지만 effect vector ownership은 TUI adapter에 있다.

해야 할 일:

- post-turn result를 application-level outcome으로 낮춘다.
- TUI는 outcome을 local effect/rendering으로만 매핑한다.
- auto prompt consume 여부와 parallel dispatch 기록을 application command/outcome으로 표현한다.

완료 조건:

- `post_turn_automation.rs`가 `QueueAutoPrompt` variant를 직접 retain/filter하지 않는다.
- parallel continuation과 task-intake suppression이 같은 application outcome vocabulary를 사용한다.
- duplicate submit 방지 regression이 통과한다.

검증:

```bash
cargo test conversation_runtime
cargo test post_turn
cargo test shell_runtime
```

### R3. NativeTuiApp Service Wiring 축소

상태: `ready`

대상:

- `src/adapter/inbound/tui/app.rs`
- `src/adapter/inbound/tui/app/app_runtime.rs`
- TUI runtime bridge modules

문제:

- `NativeTuiApp`이 `PlanningServices`, `ParallelModeService`, `ConversationService` 등 raw service를 직접 들고 있다.
- TUI가 application composition root처럼 동작한다.

해야 할 일:

- raw service field를 application-facing handle/facade로 감싼다.
- projection cache와 service wiring field를 분리한다.
- test helper가 raw service에 기대는 부분은 facade/test double 계약으로 바꾼다.

완료 조건:

- TUI app field는 UI-only state, projection cache, application handle로만 분류된다.
- raw planning/parallel service 접근이 production TUI path에 남지 않는다.
- shell runtime/rendering regression이 통과한다.

검증:

```bash
cargo test shell_runtime
cargo test shell_rendering
cargo test planning
```

### R4. Planning Runtime Bridge를 Application Facade로 통합

상태: `ready`

대상:

- `src/adapter/inbound/tui/app/turn_submission_runtime/post_turn_execution*`
- `src/application/service/planning/*`
- `src/adapter/inbound/admin_api/*`
- `src/adapter/inbound/cli.rs`
- `src/adapter/inbound/telegram_bot/*`

문제:

- planning projection/domain policy seed는 있지만 post-turn/TUI/admin/CLI route가 모두
  같은 command surface를 쓰는지 확실하지 않다.
- post-turn executor가 planning workflow를 직접 조합하는 부분이 남아 있다.

해야 할 일:

- planning reset/status/queue/task mutation/post-turn refresh route를 application request/result로 정리한다.
- route별 direct service call이 adapter mapping인지 duplicated policy인지 판정한다.
- duplicated policy는 shared facade로 이동한다.

완료 조건:

- 같은 planning 기능은 surface와 무관하게 같은 application request/result vocabulary를 사용한다.
- adapter에는 parsing/auth/context/rendering만 남는다.
- planning-related behavior regression이 통과한다.

검증:

```bash
cargo test planning
cargo test cli
cargo test admin_api
cargo test telegram
```

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

- R2
- R3

현재 판단:

- 지금 구조는 queue-backed actor loop가 아니라 mutex-serialized synchronous facade다.
- 동기 facade는 당장 유지한다. 먼저 TUI/application 경계를 줄인다.

해야 할 일:

- R2-R3 이후에도 background completion ordering, backpressure, shutdown, stale event
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
