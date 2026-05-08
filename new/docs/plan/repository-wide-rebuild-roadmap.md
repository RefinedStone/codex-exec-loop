# Repository-Wide Rebuild Roadmap

## 목적

이 문서는 `new/docs`의 대규모 구조 재설계를 worker가 바로 집을 수 있는 작업
단위로 쪼갠다. 상위 기준은 다음 문서다.

- [../architecture/repository-wide-rebuild-architecture.md](../architecture/repository-wide-rebuild-architecture.md)
- [../architecture/parallel-control-plane-architecture.md](../architecture/parallel-control-plane-architecture.md)
- [parallel-control-plane-migration-plan.md](./parallel-control-plane-migration-plan.md)

이 roadmap은 현재 구조를 보존하기 위한 backlog가 아니다. 목적은 버그를 만드는
구조를 새 기준선으로 교체하는 것이다.

## Worker 운영 규칙

모든 worker는 아래 규칙을 따른다.

- 하나의 worker는 하나의 slice만 소유한다.
- 하나의 slice는 하나의 branch와 하나의 PR로 끝낸다.
- 기본 base는 `origin/prerelease`다.
- `akra-agent/slot-*` worktree는 runtime이 관리하므로 수동 정리하지 않는다.
- 코드 변경 slice는 선행 regression 또는 architecture 문서가 없으면 시작하지 않는다.
- 문서 slice는 구현 결정을 남기되, 현재 구조를 있는 그대로 해설하는 문서가 되면 안 된다.
- 구현 slice는 domain/application/adapter 책임을 바꾸는 경우 `new/docs` 문서를 함께 갱신한다.
- slice가 서로 같은 파일을 수정해야 하면 먼저 이 roadmap의 ownership을 갱신한다.

## Slice 상태 규칙

각 slice는 다음 상태 중 하나로 관리한다.

| 상태 | 의미 |
| --- | --- |
| `ready` | 바로 작업 가능 |
| `blocked` | 선행 slice 필요 |
| `active` | worker가 branch를 잡고 진행 중 |
| `done` | `prerelease`에 반영됨 |

이 문서의 초기 상태는 worker 배정 전 기준이다. worker가 실제로 착수하면 해당
slice의 상태를 `active`로 바꾸는 문서 PR을 별도로 만들 필요는 없다. 병렬 runtime이
이미 branch/worktree 상태를 갖기 때문이다. 다만 사람이 수동으로 여러 worker를
배정할 때는 이 문서를 갱신해도 된다.

## 우선순위 요약

| Priority | Slice | 상태 | 목적 |
| --- | --- | --- | --- |
| P0 | `PAR-00` | done | parallel regression contract 고정 |
| P0 | `PAR-01` | done | parallel domain decision seed |
| P0 | `PAR-02` | done | parallel application runtime facade |
| P0 | `PAR-03` | done | parallel TUI controller split |
| P0 | `PAR-04` | done | parallel worker event path |
| P1 | `DOC-PLAN-00` | done | planning control-plane architecture 작성 |
| P1 | `PLAN-00` | done | planning regression/audit contract |
| P1 | `PLAN-01` | done | planning application facade 표준화 |
| P1 | `PLAN-02` | ready | planning domain decision/projection 정리 |
| P2 | `DOC-TUI-00` | ready | TUI/application boundary architecture 작성 |
| P2 | `TUI-00` | blocked | TUI shell state inventory와 regression |
| P2 | `TUI-01` | blocked | conversation lifecycle와 automation lifecycle 분리 |
| P3 | `DOC-INBOUND-00` | ready | inbound surface unification architecture 작성 |
| P3 | `INBOUND-00` | blocked | CLI/admin/Telegram command surface 통일 |
| P4 | `DOC-STORE-00` | ready | store/runtime-state architecture 작성 |
| P4 | `STORE-00` | blocked | durable store와 runtime store 경계 정리 |
| P5 | `TEST-00` | blocked | test/doc contract taxonomy 정리 |

## P0. Parallel Control-Plane Slices

parallel은 repository-wide rebuild의 reference architecture다. 여기서 실패하면 다른
영역으로 일반화하지 않는다.

### PAR-00. Regression Contract Lock

상태: `done`

목적:

- 구조 변경 전에 현재 parallel failure mode를 테스트로 고정한다.
- “worktree 3개 중 2개가 blocked여도 남은 capacity가 진행되어야 한다”는 계약을 잠근다.
- “task가 많을 때 dispatch가 하나만 진행되는 회귀”를 막는다.

소유 범위:

- `src/application/service/parallel_mode/tests/orchestrator_loop.rs`
- `src/application/service/parallel_mode/tests/pool/*`
- 필요한 경우 `src/adapter/inbound/tui/app/shell_runtime/tests/*`

산출물:

- blocked worktree + idle slot dispatch regression test
- capacity available event continuation regression test
- repeated `:parallel` duplicate worker guard test

금지:

- runtime 구조 변경
- TUI controller 분리
- domain decision 추출

검증:

- `cargo test parallel_mode`
- 실패 재현 테스트 이름만 봐도 operator-visible 문제가 드러나야 한다.

### PAR-01. Domain Decision Seed

상태: `done`

선행:

- `PAR-00`

목적:

- dispatch, capacity, stale epoch, failed-start unblock 판단을 domain decision으로 내린다.
- application service의 policy `if/else` 증가를 멈춘다.

소유 범위:

- `src/domain/parallel_mode/*`
- `src/application/service/parallel_mode/orchestration.rs`
- 관련 application tests

산출물:

- I/O 없는 decision 타입
- DB/thread/filesystem 없이 실행되는 domain tests
- application service에서 중복 판단 제거

금지:

- worker thread launch 위치 변경
- TUI state 변경
- repository schema 변경

검증:

- `cargo test domain::parallel_mode`
- `cargo test parallel_mode`

### PAR-02. Application Runtime Facade

상태: `done`

선행:

- `PAR-01`

목적:

- `ParallelModeControlPlaneRuntime` 또는 동등한 application runtime facade를 만든다.
- 외부 진입점이 runtime command/event로 들어오게 한다.

소유 범위:

- `src/application/service/parallel_mode/control_plane/*`
- `src/application/service/parallel_mode/orchestrator_loop.rs`
- `src/application/service/parallel_mode/tests/orchestrator_loop.rs`

산출물:

- `Enable`, `Disable`, `RefreshSupervisor`, `WakeOrchestrator`, `WorkerCompleted`, `EffectCompleted` command/event 표면
- process-lifetime runtime store
- fake repository/port 기반 ordering tests

금지:

- runtime store를 SQLite에 저장
- TUI background message가 durable state를 직접 mutate

검증:

- command serialization/order tests
- wake coalescing tests
- stale epoch tests

### PAR-03. TUI Controller Split

상태: `done`

선행:

- `PAR-02`

목적:

- `parallel_mode.rs`에서 presentation state와 application command dispatch를 분리한다.
- TUI는 `ParallelPanelUiEvent -> ParallelPanelUiState + Effect`만 담당한다.

소유 범위:

- `src/adapter/inbound/tui/app/parallel_mode.rs`
- `src/adapter/inbound/tui/app/parallel_mode/*`
- `src/adapter/inbound/tui/app/shell_runtime/tests/*`

산출물:

- `ParallelPanelStateController`
- controller-only tests
- TUI에서 durable command claim/dispatch 판단 제거

금지:

- application runtime policy를 TUI controller로 복사
- controller 간 직접 business flow 구독

검증:

- controller unit tests
- shell runtime input tests
- visible rendering이 바뀌면 snapshot tests

### PAR-04. Worker Event Path

상태: `done`

선행:

- `PAR-02`
- `PAR-03`

목적:

- worker success/failure가 TUI state를 직접 고치지 않고 application runtime event로 돌아오게 한다.
- worker launch failure, stream failure, stale completion, capacity available event를 runtime에서 처리한다.

소유 범위:

- `src/application/service/parallel_mode/orchestrator_loop.rs`
- `src/adapter/inbound/tui/app/turn_submission_runtime/*`
- `src/adapter/inbound/tui/app/parallel_mode.rs`

산출물:

- `WorkerCompleted`, `WorkerLaunchFailed`, `WorkerStreamFailed` event path
- lease/session detail/dispatch command 보상 갱신
- capacity continuation regression 통과

금지:

- failure를 TUI status copy로만 소비
- thread가 repository나 UI state를 직접 mutate

검증:

- completion-to-dispatch continuation tests
- worker failure compensation tests
- official completion refresh ordering tests

## P1. Planning Slices

planning은 parallel 다음의 구조 재정렬 대상이다. 목표는 planning authoring,
runtime, repair, worker, admin, task mutation이 같은 구조 언어를 쓰게 하는 것이다.

### DOC-PLAN-00. Planning Control-Plane Architecture

상태: `done`

목적:

- planning 전체의 새 architecture 문서를 작성한다.
- authoring/runtime/repair/worker/admin/task mutation 책임을 재정의한다.

소유 범위:

- `new/docs/architecture/planning-control-plane-architecture.md`

필수 내용:

- Planning Application Projection 정의
- durable task authority와 process-lifetime runtime state 분리
- TUI/admin/CLI가 공유할 application facade/command 원칙
- semantic validation, queue ordering, proposal classification의 domain 소유권
- prompt assembly, hidden worker retry, workspace sync의 application 소유권

검증:

- `repository-wide-rebuild-architecture.md`와 모순이 없어야 한다.
- 현재 구조 해설이 아니라 새 구조 기준이어야 한다.

### PLAN-00. Planning Regression And Audit Contract

상태: `done`

선행:

- `DOC-PLAN-00`

목적:

- planning 리팩터링 전에 현재 user-visible contract와 failure mode를 테스트로 고정한다.
- authoring/runtime/repair/worker/admin/task mutation이 여는 파일 fan-in을 기록한다.

소유 범위:

- `src/application/service/planning/tests` 또는 현재 planning test 위치
- 필요한 경우 `new/docs/plan/planning-control-plane-migration-plan.md`

산출물:

- queue ordering/proposal classification regression
- planning authoring close-risk regression
- repair/reconciliation regression
- admin/TUI shared projection audit

금지:

- architecture 없이 facade 변경
- 테스트 이름이 내부 helper만 설명하는 형태

검증:

- planning 관련 unit/integration tests
- audit 문서가 다음 implementation slice를 지정해야 한다.

### PLAN-01. Planning Application Facade Standardization

상태: `done`

선행:

- `PLAN-00`

목적:

- TUI/admin/CLI가 planning internals를 직접 호출하지 않도록 application facade/command 표면을 정리한다.

소유 범위:

- `src/application/service/planning/*`
- `src/adapter/inbound/tui/app/planning/*`
- `src/adapter/inbound/admin_api/*`
- `src/adapter/inbound/cli.rs`

산출물:

- inbound-neutral planning command/use case 표면
- internal module 직접 import 감소
- surface별 mapping과 rendering만 adapter에 남김

금지:

- TUI 전용 planning use case와 admin 전용 planning use case를 분리 생성
- planning policy를 adapter로 복사

검증:

- planning facade tests
- TUI/admin/CLI behavior regression

완료 근거:

- `PlanningApplicationProjection`이 queue lane, runtime status, source signature의 공통 read model이 되었다.
- `/status`, `/queue`, Telegram, admin overview, Akra dashboard, TUI status/queue 표시 경로가 projection 또는 planning control facade를 통과한다.
- inbound adapter의 read-only status/queue 경로에서 `PriorityQueueService`와 `queue_projection()` 직접 호출을 제거했다.
- mutation, worker orchestration, repair eligibility 판단은 정책 변경 없이 `PLAN-02`로 넘긴다.

### PLAN-02. Planning Domain Decision And Projection

상태: `ready`

선행:

- `PLAN-01`

목적:

- semantic validation, queue/proposal decision, projection summary를 domain 중심으로 정리한다.
- application은 prompt assembly와 side effect ordering에 집중한다.

소유 범위:

- `src/domain/planning/*`
- `src/application/service/planning/runtime/*`
- `src/application/service/planning/task_mutation/*`

산출물:

- I/O 없는 planning decision tests
- Application Projection assembly path
- task mutation validation 중복 제거

작업 단위:

- `PLAN-02A`: task mutation update legality 중 terminal status 재분류 금지와
  description update ownership을 domain `PlanningTaskMutationPolicy`로 이동한다. 완료.
- `PLAN-02B`: task authority link/priority invariant를 domain semantic validation으로
  일원화하고, task mutation application validation의 중복 helper를 제거한다. 완료.
- `PLAN-02C`: proposal promotion 가능 여부를 queue projection 기반 domain
  `PlanningProposalPromotionPolicy`로 이동하고, application은 authority load/commit만 맡는다. 완료.

금지:

- domain에서 workspace file, DB, adapter type import
- prompt text assembly를 domain으로 이동

검증:

- `cargo test domain::planning`
- planning runtime/task mutation tests

## P2. TUI Shell And Conversation Slices

TUI는 policy owner가 아니라 inbound adapter다. Shell, conversation, automation,
planning handoff, parallel handoff가 섞이는 부분을 줄인다.

### DOC-TUI-00. TUI Application Boundary Architecture

상태: `ready`

목적:

- TUI가 가진 state를 UI-only state, Application Projection cache, background message mapping으로 분류한다.
- conversation lifecycle과 automation lifecycle의 경계를 정한다.

소유 범위:

- `new/docs/architecture/tui-application-boundary-architecture.md`

필수 내용:

- controller/reducer/presentation/runtime bridge 역할
- prompt lock, overlay, selection, cursor의 UI-only 소유권
- post-turn automation, auto-follow, planning/parallel handoff의 application/domain 소유권
- background message가 직접 mutation하지 않는 규칙

검증:

- repository-wide rebuild architecture와 같은 용어를 사용해야 한다.

### TUI-00. Shell State Inventory And Regression

상태: `blocked`

선행:

- `DOC-TUI-00`

목적:

- TUI state를 분류하고, 구조 변경 전 rendering/input contract를 고정한다.

소유 범위:

- `src/adapter/inbound/tui/app/shell_runtime/tests/*`
- `src/adapter/inbound/tui/app/shell_rendering*_tests.rs`
- 필요한 경우 TUI state inventory 문서

산출물:

- UI-only state inventory
- Application Projection cache inventory
- shell input/rendering regression tests

금지:

- test 없이 controller split 시작

검증:

- shell runtime tests
- shell rendering snapshot tests

### TUI-01. Conversation And Automation Split

상태: `blocked`

선행:

- `TUI-00`

목적:

- conversation lifecycle과 post-turn automation lifecycle을 분리한다.
- auto-follow, planning handoff, parallel handoff policy를 TUI에서 밀어낸다.

소유 범위:

- `src/adapter/inbound/tui/app/conversation*`
- `src/adapter/inbound/tui/app/turn_submission_runtime/*`
- `src/application/service/*`

산출물:

- conversation state와 automation state 분리
- application event path
- TUI가 projection 표시만 담당하는 handoff path

금지:

- TUI background message에서 domain policy 판단

검증:

- conversation runtime tests
- post-turn execution tests
- affected TUI input/rendering tests

## P3. Inbound Surface Slices

Inbound surface는 business logic owner가 아니다. TUI, CLI, admin, Telegram은 같은
application command/use case를 공유해야 한다.

### DOC-INBOUND-00. Inbound Surface Unification Architecture

상태: `ready`

목적:

- TUI/CLI/admin/Telegram request mapping과 response rendering 규칙을 통일한다.

소유 범위:

- `new/docs/architecture/inbound-surface-unification-architecture.md`

필수 내용:

- surface별 책임
- shared application command 호출 원칙
- auth/session/context mapping 위치
- copy/rendering과 policy의 분리

검증:

- TUI/admin/CLI/Telegram을 bounded context로 취급하지 않아야 한다.

### INBOUND-00. Shared Command Surface

상태: `blocked`

선행:

- `DOC-INBOUND-00`
- 관련 context architecture 문서

목적:

- planning/parallel command를 inbound-neutral surface로 공유한다.

소유 범위:

- `src/adapter/inbound/cli.rs`
- `src/adapter/inbound/admin_api/*`
- `src/adapter/inbound/telegram_bot/*`
- shared application command/facade modules

산출물:

- duplicated policy 제거
- request/response DTO mapping 유지
- common application call path

금지:

- admin API용 domain rule, CLI용 domain rule을 따로 만들기

검증:

- CLI tests
- admin API tests
- Telegram parser/runner tests
- affected planning/parallel tests

## P4. Store And Runtime State Slices

Store와 runtime state 경계가 흐리면 재시작, retry, stale completion, test reset이 모두
깨진다.

### DOC-STORE-00. Store And Runtime State Architecture

상태: `ready`

목적:

- durable/recoverable state, process-lifetime runtime state, UI-only state를 repo 전체 기준으로 정의한다.

소유 범위:

- `new/docs/architecture/store-and-runtime-state-architecture.md`

필수 내용:

- repository/store와 runtime store의 차이
- SQLite authority, file workspace, lease/session detail, runtime projection 분류
- in-memory map이 허용되는 경우
- recovery와 test reset 기준

검증:

- parallel architecture의 state table과 모순이 없어야 한다.

### STORE-00. Durable Store Boundary Audit

상태: `blocked`

선행:

- `DOC-STORE-00`

목적:

- SQLite authority, filesystem workspace, git worktree, runtime event projection의 책임을 재분류한다.

소유 범위:

- `src/application/port/outbound/*`
- `src/adapter/outbound/db/*`
- `src/adapter/outbound/filesystem/*`
- `src/adapter/outbound/git/*`

산출물:

- durable state inventory
- process-lifetime runtime state inventory
- adapter mapping versus application policy audit

금지:

- audit 없이 repository trait 추가
- timer/effect id를 durable store로 이동

검증:

- SQLite runtime projection tests
- filesystem adapter tests
- affected planning/parallel recovery tests

## P5. Test And Docs Slices

테스트와 문서도 새 구조를 따라야 한다. 테스트가 계층 계약을 설명하지 못하면
worker가 변경 범위를 안전하게 잡을 수 없다.

### TEST-00. Contract Taxonomy

상태: `blocked`

선행:

- `PAR-00`
- `DOC-PLAN-00`
- `DOC-TUI-00`
- `DOC-STORE-00`

목적:

- 테스트를 domain decision, application ordering, adapter mapping/rendering, integration journey로 분류한다.
- `new/docs` 문서와 테스트 위치가 서로 추적 가능하게 한다.

소유 범위:

- `new/docs/plan/repository-wide-test-contract-taxonomy.md`
- 관련 test module README 또는 module-level comments where useful

산출물:

- test contract taxonomy
- 각 major context별 필수 regression 목록
- worker가 새 slice를 시작할 때 참고할 검증 matrix

금지:

- 테스트 파일 이동만 하는 기계적 PR
- behavior 변경 없이 snapshot 대량 갱신

검증:

- 문서 링크 확인
- 기존 test command 목록이 재현 가능해야 한다.

## 병렬 작업 가능 조합

바로 병렬로 시작 가능한 조합:

- `PAR-00`
- `DOC-PLAN-00`
- `DOC-TUI-00`
- `DOC-INBOUND-00`
- `DOC-STORE-00`

서로 같은 production file을 건드리지 않는 조합:

- `PAR-01`과 `DOC-PLAN-00`
- `PLAN-00`과 `DOC-TUI-00`
- `DOC-INBOUND-00`과 `DOC-STORE-00`

동시에 진행하면 안 되는 조합:

- `PLAN-01`과 `INBOUND-00`
- `PAR-04`와 `TUI-01`
- `STORE-00`과 parallel/planning repository 구현 slice

## Worker 완료 보고 형식

각 worker는 최종 보고에 다음을 포함한다.

```text
Slice:
Branch:
Changed files:
Key decisions:
Verification:
Blocked follow-up:
```

문서 slice는 “다음 worker가 바로 구현 가능한가”를 기준으로 완료한다.
구현 slice는 “어느 계층 계약을 바꿨는가”와 “어떤 regression이 막는가”를 기준으로
완료한다.
