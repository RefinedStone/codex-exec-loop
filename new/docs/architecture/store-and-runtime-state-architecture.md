# Store And Runtime State Architecture

## 목적

이 문서는 repository-wide rebuild에서 모든 저장 상태를 같은 언어로 분류하기 위한
기준선이다. 지금 구조는 planning authority, filesystem workspace, parallel slot
lease, runtime projection, TUI cache, 테스트 fake가 모두 “state”라는 이름으로
묶이기 쉽다. 이 경계가 흐리면 재시작 복구, retry, stale completion, test reset이
계층마다 다르게 동작한다.

핵심 결론은 다음과 같다.

```text
복구되어야 하는 값은 durable store가 가진다.
프로세스 안에서만 의미 있는 값은 application runtime store가 가진다.
화면 조작 값은 inbound controller가 가진다.
판단 규칙은 domain이 가진다.
저장소 구현 detail은 outbound adapter가 숨긴다.
```

이 문서는 현재 구현의 모든 파일을 그대로 설명하는 문서가 아니다. 이후 `STORE-*`
slice가 무엇을 durable authority로 승격하고, 무엇을 runtime store에 남기며, 무엇을
UI-only state로 격리해야 하는지 판단하는 reference architecture다.

## 문제 정의

Akra에는 서로 다른 성격의 상태가 동시에 존재한다.

- planning direction/task authority와 queue projection
- active/candidate/draft planning workspace 파일
- official completion refresh order와 claim
- parallel dispatch command, slot lease, session detail, distributor queue
- worker in-flight handle, wake coalescing, retry timer, stale event gate
- TUI overlay, cursor, selected row, loading 표시
- git worktree, branch, PR, app-server session 같은 외부 runtime artifact

문제는 이 상태들이 모두 같은 저장소에 있어야 한다는 뜻이 아니다. 오히려 반대다.
각 값은 “재시작 후에도 의미가 있는가”, “여러 실행 주체가 동시에 접근하는가”,
“domain invariant를 구성하는가”, “단순 presentation인가”에 따라 소유 계층이
달라져야 한다.

## 표준 상태 분류

| State 종류 | 소유 계층 | 복구성 | 예시 |
| --- | --- | --- | --- |
| Durable authority | repository/store | 재시작 후 source of truth | direction authority, task authority, queue projection |
| Durable runtime projection | repository/store | 재시작 후 recovery/read model | dispatch command, slot lease, session detail, distributor queue, task dispatch block |
| Durable workspace artifact | outbound workspace adapter | 파일 또는 repo-scoped store에서 복구 | result-output, draft files, supporting files |
| External runtime artifact | outbound adapter | 외부 시스템에서 관찰/정리 | git worktree, branch, PR, app-server session |
| Process-lifetime runtime state | application runtime store | 재시작 시 버림, durable/probe로 재구성 | wake coalescing, local in-flight handle, poll timer, command correlation |
| Domain invariant state | domain aggregate/value | load한 authority에서 계산 또는 값으로 보호 | eligibility, capacity, validation, queue ordering |
| UI-only state | inbound controller | 화면 수명 | overlay, cursor, selection, local loading |
| Application Projection | application read model | authority/projection에서 재생성 가능 | TUI/admin/CLI summary, visible task list, runtime board |

중요한 구분은 `Durable runtime projection`과 `Process-lifetime runtime state`다. 둘 다
runtime이라는 단어가 붙지만 성격이 다르다.

- 여러 프로세스, worker, 재시작 복구가 같은 값을 봐야 하면 durable runtime projection이다.
- 현재 프로세스의 event loop를 편하게 만들기 위한 값이면 process-lifetime runtime state다.

예를 들어 parallel slot lease는 재시작 뒤 pool reconciliation이 봐야 하므로 durable
runtime projection이다. 반면 특정 tick을 이미 wake queue에 넣었는지 표시하는 flag는
재시작 후 다시 계산하면 되므로 application runtime store에 둔다.

## Repository / Store와 Runtime Store의 차이

Repository/store는 domain object collection 또는 read model collection처럼 보이는
접근 계층이다. 구현은 SQLite, filesystem, git-backed authority, in-memory fake가 될 수
있지만 계약은 같다.

Repository/store 책임:

- workspace 또는 aggregate identity 기준으로 snapshot을 load/save한다.
- optimistic revision, claim, transaction, idempotent upsert 같은 저장 계약을 제공한다.
- durable authority와 durable runtime projection의 단일 진실을 보관한다.
- application service가 SQLite row, file path, JSON payload를 직접 알지 못하게 한다.

Application runtime store 책임:

- single-writer loop 내부의 process-local 상태를 가진다.
- wake coalescing, local in-flight effect id, timer, command correlation을 관리한다.
- durable store를 대신하지 않는다.
- 재시작 시 durable snapshot과 runtime probe로 다시 만들 수 있어야 한다.

금지되는 혼합:

- timer/effect id를 repository trait 뒤에 숨겨 durable state처럼 취급한다.
- task authority, slot lease, distributor queue를 thread local map에만 둔다.
- TUI loading flag를 SQLite에 저장한다.
- repository adapter 안에서 dispatch/retry/capacity policy를 판단한다.

## 현재 저장 경계 분류

### SQLite Planning Authority

현재 `SqlitePlanningAuthorityAdapter`는 단순 DB adapter가 아니라 planning authority와
parallel runtime projection의 durable boundary다. 다음 상태는 durable store로 본다.

| 상태 | 현재 boundary | 분류 | 이유 |
| --- | --- | --- | --- |
| `planning_directions` | `PlanningTaskRepositoryPort` | Durable authority | direction 문서가 accepted planning 기준이다. |
| `planning_tasks` | `PlanningTaskRepositoryPort` | Durable authority | task status, metadata, relation이 source of truth다. |
| `planning_queue_projection` | `PlanningTaskRepositoryPort` | Durable authority projection | task authority와 같은 revision으로 읽혀야 한다. |
| `runtime_claims` official refresh | `PlanningAuthorityPort` | Durable runtime coordination | 여러 worker/프로세스가 같은 순서를 지켜야 한다. |
| `runtime_dispatch_commands` | `PlanningAuthorityPort` | Durable runtime projection | blocked worktree나 restart 후 dispatch 재개 판단에 필요하다. |
| `runtime_slot_leases` | `PlanningAuthorityPort` | Durable runtime projection | slot/worktree 소유권과 cleanup recovery의 기준이다. |
| `runtime_session_details` | `PlanningAuthorityPort` | Durable runtime projection | lease보다 오래 사는 operator-facing 실행 결과다. |
| `runtime_task_dispatch_blocks` | `PlanningAuthorityPort` | Durable runtime projection | disposable pool reset 뒤에도 task-level block이 남아야 한다. |
| `runtime_distributor_queue` | `PlanningAuthorityPort` | Durable runtime projection | PR 생성, integration, retry 상태가 재시작 뒤 이어져야 한다. |
| `runtime_events` | `ParallelModeRuntimeEventLogPort` | Durable audit projection | 최근 전이를 recovery와 UI 진단에서 공유한다. |
| `shadow_documents` | authority adapter | Durable mirror | 파일 workspace와 authority store drift를 진단하는 mirror다. |
| `staged_drafts` / `active_documents` | `PlanningWorkspacePort` repo-scoped 구현 | Durable workspace artifact | repo-scoped workspace의 파일 표현이다. |

주의할 점은 runtime projection이 domain singleton이 아니라는 점이다. application은
workspace identity로 snapshot을 load하고, domain decision을 적용한 뒤, repository에
저장한다. SQLite row가 존재한다고 해서 application loop가 생략되면 안 된다.

### Filesystem Planning Workspace

`PlanningWorkspacePort`는 active/candidate/draft planning 파일을 다룬다. 일반 filesystem
workspace에서는 파일이 실제 저장소이고, repo-scoped workspace에서는 SQLite adapter가 파일
표현을 대신할 수 있다.

분류 기준:

- 사람이 읽고 편집하는 산출물은 durable workspace artifact다.
- accepted direction/task authority는 파일 mirror가 아니라 DB authority가 기준이다.
- draft는 promoted되기 전까지 accepted authority가 아니다.
- validation은 workspace artifact와 authority snapshot을 함께 읽을 수 있지만, policy는 domain이 판단한다.

따라서 `result-output.md`, supporting file, draft file은 repository aggregate 자체가 아니다.
이들은 application use case가 domain validation 또는 prompt assembly에 넣기 위해 읽는 durable
artifact다.

### Git Worktree And Branch State

parallel mode의 worktree, branch, commit, PR은 Akra DB 안에 모두 복제할 대상이 아니다.
이들은 external runtime artifact이며 `ParallelModeRuntimePort`, GitHub adapter, git
outbound adapter를 통해 관찰하고 조작한다.

저장 기준:

- worktree path, branch name, commit sha, PR number처럼 recovery와 operator display에 필요한
  참조값은 durable runtime projection에 저장한다.
- 실제 worktree 디렉터리, branch, remote PR의 존재 여부는 outbound adapter가 probe한다.
- probe 결과를 보고 cleanup/retry 여부를 정하는 순서는 application/domain decision으로 둔다.
- git command 결과 parsing과 filesystem primitive는 outbound adapter 내부에 둔다.

## In-Memory 저장이 허용되는 경우

in-memory map은 금지 대상이 아니다. 다만 무엇을 흉내 내는지 명확해야 한다.

허용:

- 테스트에서 repository/store port를 대체하는 fake
- single-writer application loop 내부의 process-lifetime runtime store
- projection rebuild 비용을 줄이는 short-lived cache
- 한 요청 안에서만 쓰는 memoization

조건:

- production durable source of truth를 대체하지 않는다.
- workspace/test reset boundary가 명확해야 한다.
- 여러 thread가 직접 mutate하지 않고 single-writer loop 또는 lock 범위가 명확해야 한다.
- 재시작 후 사라져도 durable store와 runtime probe로 복구 가능해야 한다.

금지:

- accepted task authority를 process-global singleton map으로 유지
- slot lease나 distributor queue를 DB 없이 local map에만 저장
- fake repository의 전역 map을 production code path로 승격
- UI-only state를 shared in-memory repository로 이동

## Recovery 기준

재시작 또는 worker failure 뒤에는 다음 순서로 복구한다.

```text
application starts
  -> durable authority/projection load
  -> external runtime artifact probe
  -> domain recovery decision
  -> durable projection repair if needed
  -> application runtime store rebuild
  -> Application Projection publish
```

Recovery 규칙:

- process-lifetime runtime state는 복구 대상이 아니라 재구성 대상이다.
- durable claim은 stale policy가 있어야 하며, owner token만 믿고 영구 대기하면 안 된다.
- slot lease는 worktree probe와 함께 검증한다.
- session detail은 UI/recovery read model이므로 lease cleanup과 같은 transaction 순서로 갱신한다.
- queue projection은 task authority revision과 어긋나지 않아야 한다.
- stale completion event는 epoch, revision, lease/session identity로 버릴 수 있어야 한다.

## Test Reset 기준

테스트 reset은 “모든 state를 지운다”가 아니라 계층별 reset을 명시한다.

| Reset 대상 | 지워야 하는 것 | 지우면 안 되는 것 |
| --- | --- | --- |
| UI test | controller UI-only state | durable authority fixture |
| Application runtime test | in-flight handle, wake flag, timer | repository fixture |
| Repository test | SQLite rows, in-memory fake map | external git/app-server artifact |
| Workspace adapter test | temp workspace files, staged draft | accepted DB authority unless test owns it |
| Parallel recovery test | runtime projection fixture와 fake probe state | domain policy helper |

새 테스트를 추가할 때는 어떤 계층 계약을 검증하는지 이름에 드러나야 한다. 예를 들어
`load_runtime_projections_keeps_slot_and_queue_same_snapshot`은 repository/read model 계약이고,
`wake_coalescing_does_not_persist_after_restart`는 application runtime store 계약이다.

## Application Projection 원칙

Application Projection은 inbound surface가 읽는 공통 read model이다. Projection은 durable
authority와 durable runtime projection에서 만들 수 있어야 하며, inbound adapter가 정책을 다시
계산하지 않게 한다.

Projection 규칙:

- projection은 display-ready일 수 있지만 새로운 business policy를 판단하지 않는다.
- projection 생성은 application service 책임이다.
- projection cache가 필요하면 cache invalidation owner를 명시한다.
- TUI/admin/CLI/Telegram은 projection을 surface별 response로 mapping만 한다.
- projection을 저장할지 말지는 복구 필요성과 비용에 따라 결정한다.

## Domain 순수성 보존

저장 상태를 명확히 나눠도 domain이 약해지면 안 된다. Domain은 repository 구현체를 소유하지 않고,
application이 load한 aggregate/value를 받아 순수 decision을 반환한다.

Domain 소유:

- queue ordering과 dispatch eligibility
- stale epoch, retry 가능성, blocked reason 해석
- planning validation, proposal classification, mutation legality
- reset 후 어떤 durable projection을 제거하거나 보존해야 하는지에 대한 순수 decision

Application 소유:

- 어느 repository snapshot을 어떤 순서로 읽을지
- transaction/claim/retry boundary
- domain decision을 저장 가능한 mutation과 outbound effect로 바꾸는 일
- external runtime artifact probe와 cleanup 실행

Outbound adapter 소유:

- SQLite row shape와 migration
- filesystem path normalization
- git command, GitHub API, app-server protocol mapping
- port request/response로의 변환

## STORE-00 구현 분할 기준

다음 구현 slice는 이 문서를 기준으로 inventory를 먼저 만든다.

1. `PlanningTaskRepositoryPort`, `PlanningAuthorityPort`, `PlanningWorkspacePort`,
   `ParallelModeRuntimePort`가 다루는 상태를 위 표의 분류로 tagging한다.
2. durable authority와 durable runtime projection이 같은 trait에 섞인 곳은 즉시 분리하지 말고,
   method별 classification과 migration risk를 먼저 기록한다.
3. process-lifetime runtime state 후보를 application service 내부에서 찾는다. timer, effect id,
   wake coalescing, local worker handle은 repository로 내리지 않는다.
4. reset/recovery 테스트가 없는 durable runtime projection에는 regression anchor를 추가한다.
5. 새 repository trait은 중복 load/save나 transaction boundary가 실제로 단순해지는 slice에서만 추가한다.

완료 조건:

- durable state inventory와 process-lifetime runtime state inventory가 문서화된다.
- SQLite authority, filesystem workspace, git worktree, runtime event projection의 책임이 분류된다.
- recovery와 test reset 기준이 각 inventory 항목에 연결된다.
- parallel architecture의 state table과 모순이 없어야 한다.

구체 inventory와 후속 분할은
[store-runtime-state-boundary-inventory.md](../plan/store-runtime-state-boundary-inventory.md)를 따른다.
