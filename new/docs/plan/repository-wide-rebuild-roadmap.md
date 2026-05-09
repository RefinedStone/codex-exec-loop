# Repository-Wide Rebuild Roadmap

## 문서 지위

이 문서는 `new/docs` 재건 작업의 단일 실행 문서다. 아래 두 문서는 별도 기준
문서로 유지하고, 이 문서에서 압축하지 않는다.

- [../architecture/parallel-control-plane-architecture.md](../architecture/parallel-control-plane-architecture.md)
- [parallel-control-plane-migration-plan.md](./parallel-control-plane-migration-plan.md)

그 외 `new/docs` 문서는 설계 배경 또는 과거 inventory로만 취급한다. 구현 상태와
다음 작업 판단은 이 문서를 기준으로 한다. 과거 문서의 `done` 표기는
`prerelease`에 어떤 slice가 병합됐다는 관리 기록일 뿐이며, repository-wide rebuild
완료 선언으로 보지 않는다.

## 판정 기준

상태는 세 가지만 쓴다.

| 상태 | 의미 |
| --- | --- |
| `implemented` | 코드가 기준을 만족하고, regression이 계층 계약을 막는다 |
| `partial` | 일부 구현됐지만 TUI/application/domain/store 경계에 잔여 작업이 있다 |
| `not-started` | 문서 또는 inventory만 있고 실제 구조 이동이 없다 |

`implemented`로 바꾸려면 아래 조건을 모두 만족해야 한다.

- 구현 PR이 `prerelease`에 병합됐다.
- 테스트가 behavior를 막는다. source string guard만으로는 부족하다.
- inbound adapter에 domain policy, durable mutation, worker launch decision이 남지 않는다.
- application service는 ordering, transaction, port effect orchestration을 맡고,
  invariant 판단은 domain decision으로 이동했다.
- durable/recoverable state, process-lifetime runtime state, UI-only state가 서로
  다른 owner를 가진다.
- 문서의 field/file inventory가 현재 코드와 drift되지 않는다.

## 현재 판정

전체 판정: `partial`

1차 문서화와 regression seed는 진행됐지만, repository-wide rebuild는 완료되지
않았다. 특히 TUI runtime bridge와 post-turn/stream lifecycle에 application workflow가
남아 있다. 따라서 다음 작업은 새 원칙을 더 쓰는 것이 아니라, 실제 코드 경계를
줄이는 migration slice여야 한다.

| 영역 | 상태 | 구현된 것 | 남은 문제 |
| --- | --- | --- | --- |
| Parallel control-plane | `partial` | `projection_ready`, refresh/reconcile, dispatch readiness, stale epoch 판단이 TUI 밖으로 이동했다. TUI는 raw control-plane service 대신 handle을 보유한다. | queue-backed event loop가 아니라 mutex-serialized facade다. `turn_submission_runtime`과 post-turn bridge에 parallel handoff/slot lease logic이 남아 있다. |
| Planning | `partial` | planning projection, control facade, domain policy seed가 있다. task mutation, promotion, queue follow, repair guard 일부가 domain/application 경계로 이동했다. | TUI와 post-turn executor가 아직 `PlanningServices`와 planning workflow를 직접 조합한다. facade가 모든 inbound/runtime bridge를 덮지는 못한다. |
| TUI boundary | `partial` | shell state inventory, background message inventory, conversation/automation vocabulary split이 있다. | `NativeTuiApp`이 service wiring과 runtime bridge를 크게 보유한다. `QueueAutoPrompt` mutation, active turn snapshot capture, slot lease request 생성이 TUI 쪽에 남아 있다. |
| Inbound surfaces | `partial` | CLI/admin/Telegram 일부 command vocabulary가 shared facade를 사용한다. admin route pair와 TUI parser drift를 막는 regression이 있다. | 모든 surface가 같은 application command/use case를 쓰는지는 기능별로 재검증이 필요하다. parallel live control-plane host 공유는 아직 아니다. |
| Store/runtime state | `partial` | SQLite authority, runtime projection, pool-local mirror I/O, process runtime state inventory가 있다. mirror I/O는 outbound runtime port로 이동했다. | inventory와 현재 코드가 일부 drift됐다. process-lifetime state와 durable/recoverable state의 recovery 요구가 기능별로 다시 판정되어야 한다. |
| Tests/docs | `partial` | test taxonomy와 여러 regression anchor가 있다. | 문서가 많고 완료 표기가 과하다. source-level guard가 실제 behavior regression을 대체하는 곳이 있다. |

## 해야 할 작업

우선순위는 TUI에서 application workflow를 빼는 순서다. event loop 전환은 지금
필수 작업이 아니다. 동기 facade를 유지하더라도 아래 작업은 필요하다.

### R1. Turn Submission Runtime Bridge 축소

상태: `ready`

목표:

- `src/adapter/inbound/tui/app/turn_submission_runtime.rs`
- `src/adapter/inbound/tui/app/turn_submission_runtime/*`

현재 문제:

- TUI가 `ParallelModeSlotLeaseRequest`를 만들고 task/agent slug를 생성한다.
- TUI stream bridge가 `ParallelModeTurnService`를 직접 조합해 slot lifecycle을
  따라간다.
- active turn execution snapshot capture가 post-turn reconciliation input이지만
  TUI field에 남아 있다.

해야 할 일:

- stream launch 준비를 application request/result로 이동한다.
- slot lease request 생성과 slug normalization을 application/domain helper로
  단일화한다.
- TUI는 `StartStream` effect를 application bridge에 넘기고, returned launch
  projection과 background message만 처리한다.

완료 조건:

- TUI 파일에 slot lease slug 생성 helper가 없다.
- TUI가 parallel slot lifecycle state transition service를 직접 호출하지 않는다.
- stream start, launch failure, terminal failure, official completion ordering regression이 통과한다.

권장 검증:

```bash
cargo test turn_submission_runtime
cargo test parallel_mode
cargo test shell_runtime
```

### R2. Post-Turn Automation Effect Ownership 정리

상태: `ready`

목표:

- `src/adapter/inbound/tui/app/post_turn_automation.rs`
- `src/adapter/inbound/tui/app/conversation_runtime.rs`
- `src/application/service/post_turn_decision.rs`
- 필요 시 `src/application/service/parallel_mode/control_plane/*`

현재 문제:

- TUI target이 `ConversationRuntimeEffect::QueueAutoPrompt`를 직접 검사하고 제거한다.
- pending task-intake path도 generic auto prompt를 TUI에서 suppress한다.
- post-turn continuation은 control-plane으로 올라갔지만 effect vector ownership은
  여전히 TUI adapter에 있다.

해야 할 일:

- post-turn result를 application-level outcome으로 낮추고, TUI는 outcome을 effect로
  매핑만 한다.
- auto prompt consume 여부와 parallel dispatch 기록을 application command/outcome으로 표현한다.
- task-intake flush와 auto prompt suppression ordering을 behavior test로 고정한다.

완료 조건:

- `post_turn_automation.rs`가 `QueueAutoPrompt` variant를 직접 retain/filter하지 않는다.
- parallel continuation과 task-intake suppression이 같은 application outcome vocabulary를 사용한다.
- duplicate submit 방지 regression이 통과한다.

권장 검증:

```bash
cargo test conversation_runtime
cargo test post_turn
cargo test shell_runtime
```

### R3. NativeTuiApp Service Wiring 축소

상태: `ready`

목표:

- `src/adapter/inbound/tui/app.rs`
- `src/adapter/inbound/tui/app/app_runtime.rs`
- TUI runtime bridge modules

현재 문제:

- `NativeTuiApp`이 `PlanningServices`, `ParallelModeService`, `ConversationService`
  등 여러 application service를 직접 들고 있다.
- TUI가 service composition root처럼 동작해 기능별 boundary가 흐려진다.

해야 할 일:

- TUI가 직접 들 필요가 없는 service를 application-facing handle/facade로 감싼다.
- projection cache와 service wiring field를 분리한다.
- 기존 test helper가 raw service에 기대는 부분을 facade/test double로 바꾼다.

완료 조건:

- `NativeTuiApp` field inventory가 현재 코드와 맞다.
- TUI app field는 UI-only state, projection cache, application handle로만 분류된다.
- raw planning/parallel service 접근이 테스트 전용으로도 새 helper 계약을 통과한다.

권장 검증:

```bash
cargo test shell_runtime
cargo test shell_rendering
cargo test planning
```

### R4. Store/Runtime Inventory Drift 제거

상태: `ready`

목표:

- [store-runtime-state-boundary-inventory.md](./store-runtime-state-boundary-inventory.md)
- [tui-shell-state-inventory.md](./tui-shell-state-inventory.md)
- 관련 runtime/store modules

현재 문제:

- inventory 문서가 현재 코드와 일부 맞지 않는다.
- 과거 field 이름이나 이동 완료 문구가 남아 있어 worker가 잘못된 기준으로 작업할 수 있다.

해야 할 일:

- inventory를 현재 코드 기준으로 갱신한다.
- 각 state를 `UI-only`, `Application Projection Cache`, `Process Runtime`,
  `Durable/Recoverable`, `Service Wiring` 중 하나로 다시 판정한다.
- drift를 발견하면 구현 변경 slice와 문서-only slice를 분리한다.

완료 조건:

- inventory에 존재하지 않는 field명이 남지 않는다.
- `done` 대신 현재 owner와 다음 owner만 적는다.
- 구현 변경이 필요한 항목은 이 문서의 R-slice로 연결된다.

권장 검증:

```bash
rg "parallel_mode_enabled|parallel_mode_control_plane_runtime|Hybrid" new/docs
git diff --check
```

### R5. Inbound Surface 실제 공통 경로 감사

상태: `ready`

목표:

- `src/adapter/inbound/cli.rs`
- `src/adapter/inbound/admin_api/*`
- `src/adapter/inbound/telegram_bot/*`
- shared application facade modules

현재 문제:

- 일부 command vocabulary는 정렬됐지만, 모든 mutation/read path가 같은 application
  service를 쓰는지는 기능별로 다시 봐야 한다.
- admin/CLI/Telegram의 rendering 차이와 policy 차이가 문서상 구분보다 흐릴 수 있다.

해야 할 일:

- planning reset/status/queue/task mutation과 parallel status/tick path를 route별로 표로 만든다.
- direct service call이 adapter mapping인지, duplicated policy인지 판정한다.
- duplicated policy는 shared application command/facade로 이동한다.

완료 조건:

- surface별로 같은 기능이 같은 application request/result vocabulary를 사용한다.
- adapter에는 parsing/auth/context/rendering만 남는다.
- admin API용 rule, CLI용 rule, Telegram용 rule이 따로 존재하지 않는다.

권장 검증:

```bash
cargo test cli
cargo test admin_api
cargo test telegram
cargo test planning
```

### R6. Control-Plane Event Loop 전환 여부 결정

상태: `blocked`

선행:

- R1
- R2
- R3

현재 판단:

- 지금 구조는 queue-backed actor loop가 아니라 mutex-serialized synchronous facade다.
- 동기 facade는 당장 유지해도 된다. 먼저 TUI/application 경계를 줄여야 한다.

해야 할 일:

- R1-R3 이후에도 background completion ordering, backpressure, shutdown, stale event
  문제가 남는지 측정한다.
- 문제가 남으면 queue-backed single consumer loop 설계를 별도 slice로 작성한다.
- 문제가 충분히 제어되면 mutex facade를 명시적 설계 선택으로 문서화한다.

완료 조건:

- actor loop가 필요한지 여부가 구현 근거와 regression으로 결정된다.
- 선택한 구조가 `parallel-control-plane-architecture.md`와 충돌하면, 충돌 내용을
  해당 parallel 기준 문서에 별도로 갱신한다.

## 문서 정리 규칙

새 문서를 추가하지 않는다. 새 작업은 이 문서의 R-slice를 갱신한다.

기존 문서는 다음처럼만 사용한다.

| 문서 | 지위 |
| --- | --- |
| `parallel-control-plane-architecture.md` | 유지되는 기준 문서 |
| `parallel-control-plane-migration-plan.md` | 유지되는 기준 문서 |
| `repository-wide-rebuild-roadmap.md` | 현재 실행 기준 |
| 나머지 `new/docs/architecture/*` | 배경 설명. 구현 상태 authority 아님 |
| 나머지 `new/docs/plan/*` | 과거 inventory 또는 regression 메모. `done` authority 아님 |

기존 문서를 수정할 때는 장황한 상태 표를 늘리지 않는다. 필요한 경우 이 문서의
R-slice 하나만 갱신한다.

## Worker 보고 형식

모든 구현 slice는 최종 보고에 아래 항목만 적는다.

```text
Slice:
Branch:
Implemented:
Still partial:
Changed files:
Verification:
Follow-up:
```

`Still partial`이 비어 있지 않으면 이 roadmap의 해당 영역 상태를 `implemented`로
바꾸지 않는다.
