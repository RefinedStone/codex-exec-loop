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
| Control-plane bypass | 일부 surface가 아직 `ParallelModeService`를 직접 받아 control-plane gate를 우회할 수 있다. |
| TUI boundary | TUI production state에 raw application service handle debt가 남아 있다. |
| Inbound composition | CLI/admin/Telegram/TUI entrypoint가 production outbound adapter wiring을 아직 직접 들고 있다. |
| Tests | source-string guard가 behavior test를 대체하는 곳이 있다. 새 slice마다 behavior test를 우선한다. |

## 실행 Backlog

### R7. Parallel Control-Plane Bypass 제거

상태: `ready`

결정:

- R6에서 queue-backed actor loop 전환은 보류했다.
- 현재 선택은 mutex-serialized synchronous facade다.
- 따라서 다음 문제는 actor loop 부재가 아니라 raw `ParallelModeService` bypass다.

대상:

- `src/application/service/parallel_mode/control_plane/composition.rs`
- `src/application/service/parallel_mode/control_plane/controller.rs`
- `src/adapter/inbound/tui/app/app_runtime.rs`
- `src/adapter/inbound/tui/app/parallel_mode.rs`

해야 할 일:

- production inbound surface가 parallel control-plane handle을 우회하지 못하게 한다.
- manual orchestrator tick도 control-plane command/effect 경로로 들어가게 한다.
- controller가 command 구성 중 low-level queue query를 직접 호출하는 지점을 좁힌다.
- TUI parallel binding에서 raw service accessor를 제거한다.

완료 조건:

- `temporary_parallel_control_surfaces_no_longer_bypass_control_plane_gate`가 통과한다.
- 기존 `parallel_mode` regression이 통과한다.

### R8. TUI Raw Application Service Handle 축소

상태: `ready`

대상:

- `src/adapter/inbound/tui/app.rs`
- `src/adapter/inbound/tui/app/app_runtime.rs`
- `src/adapter/inbound/tui/app/*controller*`

해야 할 일:

- TUI production state가 raw application service를 직접 들고 있는 범위를 좁힌다.
- TUI가 필요한 것은 UI state, projection cache, narrow application handle뿐으로 만든다.
- startup/session/conversation/planning/parallel 경계별로 작은 handle을 구성한다.

완료 조건:

- `temporary_tui_raw_application_services_have_been_wrapped`가 통과한다.
- TUI flow regression이 통과한다.

### R9. Production Composition Wiring 중앙화

상태: `ready`

대상:

- `src/adapter/inbound/cli.rs`
- `src/adapter/inbound/admin_api/mod.rs`
- `src/adapter/inbound/telegram_bot/mod.rs`
- `src/adapter/inbound/tui/app/shell_entrypoint.rs`
- application composition module

해야 할 일:

- production outbound adapter wiring을 inbound entrypoint에서 application composition으로 올린다.
- inbound adapter는 auth/context mapping과 command invocation만 남긴다.
- GitHub polling처럼 TUI edge에 남은 adapter 생성도 application-facing handle로 감싼다.

완료 조건:

- `inbound_adapters_do_not_depend_on_outbound_implementations`의 temporary allowance를 줄이거나 제거한다.
- CLI/admin/Telegram/TUI smoke regression이 통과한다.

### R10. Architecture Guard를 Behavior Regression으로 보강

상태: `ready`

대상:

- `tests/architecture_boundaries.rs`
- parallel/planning/TUI flow tests

해야 할 일:

- source-string guard가 실제 behavior regression 없이 정책만 잡는 곳을 분류한다.
- 새 구조 slice마다 behavior test를 먼저 추가하고 source guard는 보조 안전망으로 낮춘다.
- 남길 source guard는 directory dependency, public field 노출, raw adapter import처럼 정적 검사가 더 정확한 항목으로 제한한다.

완료 조건:

- 남은 source guard마다 behavior test로 대체하지 않는 이유가 test 이름 또는 comment에 드러난다.
- R7-R9에서 제거된 debt는 source guard 삭제만이 아니라 behavior regression으로 확인된다.

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
