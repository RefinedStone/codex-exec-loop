# 런타임 아키텍처

[English](../../reference/architecture.md)

이 문서는 구현된 런타임의 canonical architecture와 authority reference입니다.

아래 화살표는 **컴파일 시점의 소스 의존성**입니다. `A -> B`는 A가 B 소유 계약을 import한다는
뜻이며 command의 실행 순서를 뜻하지 않습니다.

```text
adapter/inbound/tui -> core + application services/contracts/projections + composition wiring + domain
adapter/inbound/{cli,admin_api,telegram_bot} -> application -> domain
application -> outbound ports
adapter/outbound -> application ports + domain
composition -> core + application + adapter/outbound
```

TUI 행은 현재 남아 있는 과도기 의존성을 숨기지 않고 표현합니다. Application service/projection과
composition wiring 의존성 제거는 뒤의 state/effect 경계 작업이며, 이 의존성이 adapter에 Core 내부
mutable state 접근 권한을 주지는 않습니다.

Client command loop의 **런타임 실행 흐름**은 의도적으로 왕복합니다.

```text
TUI intent
  -> core reducer
  -> CoreEffect
  -> composition/CoreEffectRunner
  -> application use case / outbound port
  -> CoreInput completion
  -> core reducer
  -> snapshot/event
  -> TUI projection
```

이 왕복은 소스 의존성을 뒤집지 않습니다. Effect/completion 계약은 Core가 소유하고 composition이
application service로 해석합니다.

## 계층 소유권

| 계층 | 소유 | 소유하면 안 되는 것 |
| --- | --- | --- |
| `adapter/inbound` | 입력 mapping, rendering, local focus/editor/selection state | domain 정책, durable task truth, dispatch 정책 |
| `core` | framework와 독립된 client runtime: command/event/effect/completion 흐름, process-local client state, projection, snapshot | business/domain 권한, TUI/HTTP/Telegram type, application service, 구체 DB/Git/filesystem adapter |
| `application/service` | use-case orchestration, ordering gate, transaction, control-plane handle | widget, terminal event, transport DTO |
| `application/port` | application service가 요구하는 outbound contract | 구체 integration 상세 |
| `domain` | 순수 invariant, validation, decision, state transition | async runtime, IO, logging, UI, DB, filesystem, Git 호출 |
| `adapter/outbound` | app-server, DB, filesystem, Git, GitHub, Telegram integration | business policy |
| `composition` | dependency 생성과 concrete wiring | domain decision |

Mapping은 adapter에, policy는 domain 또는 application service에 둡니다. 실제 outbound boundary가
있을 때만 port를 추가합니다.

## Core runtime

`src/core`는 business hexagon 바깥의 inbound client 경계에 놓인 framework 독립 **Client Runtime**입니다.
Native client의 장기 수명 상태 coordinator이며 별도의 business layer도 아니고 application/domain
layer를 대체하지도 않습니다. CLI, Admin, Telegram은 같은 application service를 사용하면서도 이
process-local TUI runtime을 채택할 필요가 없습니다.

- `AppCommand` 또는 `CoreInput`: 사용자·lifecycle·tick·completion intent
- `Effect`: core 바깥에서 실행할 작업
- `Completion`: 같은 input queue로 돌아오는 effect 결과
- `AppEvent`: 외부에 유용한 전이
- `AppSnapshot`/projection: adapter가 읽는 view model

Mutable client-runtime state는 `CoreRuntime`만 구동합니다. Adapter는 typed input을 dispatch하고
owned snapshot/projection만 읽어야 하며 `CoreController`, `AppState`, `TurnStreamState`를 직접
생성하거나 변경하면 안 됩니다. Effect executor는 작업을 수행하고 completion을 반환할 수 있지만
runtime state의 소유자도 writer도 아닙니다.

시작, session load, conversation 선택, turn 제출, stream reduction, 완료, post-turn 평가가 이 흐름을
사용합니다. Parallel mutation은 application 소유이며 `ParallelModeControlPlaneHandle`로 진입합니다.
Core는 projection을 복사할 수 있지만 두 번째 parallel runtime을 소유하면 안 됩니다.

`AppState`는 전체 read model을 하나의 `Arc<AppSnapshot>` copy-on-write 권위로 보관합니다.
`CoreDispatchOutcome`과 generic `SnapshotChanged` event는 해당 전이의 정확히 같은 snapshot
allocation을 공유합니다. 따라서 unchanged, stale, admission-only, turn-stream input은 load된 전체
conversation/session catalog가 아니라 `Arc`만 복제합니다. 상태 변경이 허용되면
`Arc::make_mut`로 새 revision을 만들기 때문에 이전 outcome을 보관한 caller는 그 시점의 상태를
그대로 관찰합니다. 전체 owned read가 필요한 흐름에는 명시적 `snapshot()` pull을 유지합니다.
Snapshot pointer identity와 `AppState` revision은 dispatch identity가 아닙니다. Controller 내부
상태나 turn stream만 바뀐 전이는 같은 `AppSnapshot`을 공유하면서도 순서가 있는 event를 낼 수
있으므로 adapter는 모든 outcome event를 계속 순서대로 적용해야 합니다.

Post-turn 평가는 planning-worker panel을 바꾸기 전에 Core로 진입합니다. Command는 explicit
완료·확정된 최신 terminal이고 active turn, 이미 적용된 평가, 같은 in-flight 평가가 없을 때만
허용됩니다. 오래되거나 잘못되거나 중복된 start는 event/effect를 내지 않으며 completion은 정확한
in-flight thread/turn lease와 일치해야 합니다. Core는 마지막 exact accepted
`PlanningWorkerPanelState`를 history seed로 보관하고 conversation lifecycle이 무효화되거나
교체되면 기본값으로 초기화합니다. Adapter request의 동명 field는 contract 호환을 위한 중립
placeholder일 뿐 history authority가 아닙니다. Core는 admission 뒤 자체 seed를 복제해 settlement
pause면 전체 상태를 보존하고, 아니면 protected planning file 변경 시 `RepairRunning`, 빈 queue와
stop policy면 전체 상태 보존, 나머지는 `RefreshRunning`을 선택합니다. Core는 effect request를
이 accepted 상태로 덮어쓰고 비동기 effect보다 먼저 같은 상태를
`PostTurnEvaluationStarted`로 내보냅니다. Active correlation, nested execution identity, turn
authority 검증을 모두 통과한 completion만 다음 history seed를 교체할 수 있으며 stale,
duplicate, identity-mismatched, lifecycle-pruned completion은 seed를 바꾸지 못합니다. TUI는
started/completed projection을 단순 대입할 뿐 display copy를 다음 평가의 seed로 쓰거나 raw
application/planning handle을 보관하거나 planning workspace/runtime use case를 직접 호출하지
않습니다.

Native TUI의 prompt-log privacy maintenance도 이 effect 경로가 소유합니다. Production composition은
typed maintenance port를 `StartupService`에 주입하지만 app을 build하는 동안 SQLite purge/clear를
실행하지 않습니다. Startup command는 단조 증가 generation과 요청된 정확한 workspace를 캡처하고,
기존 Core startup worker가 같은 workspace에서 maintenance를 먼저 실행한 뒤 startup check를
순서대로 실행합니다. Capture가 켜져 있으면 retention purge, 꺼져 있으면 전체 clear를 선택합니다.
Maintenance error 또는 panic은 원문을 포함하지 않는 고정된 non-fatal startup warning이 됩니다.
Startup provider panic은 같은 correlation을 가진 redacted failure completion 하나로 돌아옵니다.
민감한 Core worker 범위는 process 전체에 한 번 설치되는 delegating panic hook을 공유합니다.
표시된 worker panic은 고정된 redacted 관측만 남기고, 일반 panic은 기존 hook으로 계속 전달합니다.
Core는 활성 generation/workspace 쌍만 받아들이므로 stale A→B→A 결과와 duplicate completion이 최신
요청을 완료할 수 없습니다. Maintenance나 startup probe가 막혀 있어도 app construction,
`prepare_runtime`, 첫 draw는 계속 가능합니다. CLI, Admin, Telegram 등 TUI 밖 composition의 기존
동기 maintenance semantics는 그대로 유지합니다.

Turn submission admission은 core가 소유하는 single-flight 권한입니다. TUI는 prompt intent 단계에서
editor를 지우거나 transcript history를 추가하지 않고, core가 accepted admission을 반환한 뒤에만
로컬 projection을 확정합니다. 활성 submission이 있으면 core는 명시적 rejection event를 내보내고
worker effect를 만들지 않으므로 adapter와 core가 가짜 `starting turn` 상태로 갈라지지 않습니다.

Runtime stop admission도 core가 소유합니다. `:stop`은 TUI에서 auto-follow와 parallel automation을
즉시 중지한 뒤 correlation 없는 command를 보냅니다. Core가 단조 증가 stop generation을 발급하고,
활성 turn submission이 있으면 그 identity에 귀속하며, 요청 하나만 승인하고 stale 또는 중복
completion을 버립니다. Composition은 기존 `request_stop_all_sessions` 신호를 직렬로 broadcast합니다.
`TurnStarted` 전에 성공한 요청은 정확히 correlation된 첫 start 뒤 한 번만 재동기화합니다. Provider
호출 오류는 admission을 다시 열지만 fail-closed interrupt stream notice는 정확한 retry, terminal,
conversation 전이 전까지 gate를 열지 않습니다. Outbound 신호는 의도적으로 global이므로 Core
correlation은 admission과 lifecycle을 제어하지만 `:stop`을 받는 runtime session 범위를 좁히지는
않습니다.
Provider 실행은 command dispatch 밖에서 수행합니다. 진행 중인 attempt를 turn 또는 conversation
전이가 무효화하면 Core는 worker permit을 무효화하되 정확한 completion까지 stop lease를 유지합니다.
그 settlement 전에는 새 turn admission을 거부하고 새 conversation load를 지연하여, 늦은 global
broadcast가 stop 요청 뒤 승인된 새 작업을 중단하지 못하게 합니다. Provider panic도 같은 correlation의
실패 completion으로 환원되어 lease를 영구 점유하지 않습니다.

Manual prompt preparation도 같은 admission 규칙을 사용합니다. TUI는 correlation 없는 intent를
보내고 core가 승인한 뒤에만 editor, delivery, parallel-mode 문맥을 결합합니다. Core가 correlation을
발급하고 preparation 하나만 허용하며 stale 또는 중복 completion을 버립니다. TUI는 승인된
workspace와 변경되지 않은 draft를 확인한 뒤 prepared result를 적용합니다.
취소는 background worker permit을 무효화하지만 physical single-flight lease는 정확한 settlement까지
유지합니다. Preparation은 blocking read 뒤와 bootstrap stage, promote, task-intake commit 직전에
permit을 다시 확인하므로 무효화된 A→B→A 요청이 새 mutation과 겹치지 않습니다. Worker panic은
정확히 correlation된 rejected completion으로 환원됩니다.

활성 turn steer도 같은 권한 경계를 사용합니다. Core는 정확히 일치하는 활성
submission/thread/turn identity만 승인하고, 해당 submission에 귀속된 correlation을 발급해 provider
worker 하나만 시작하며 stale completion을 버립니다. TUI에는 confirmation modal과 성공 시 변경되지
않은 draft만 지우기 위한 editor revision만 남습니다. 이미 승인된 steer는 turn terminal event만으로
무효화하지 않지만 conversation identity가 바뀌면 무효화합니다.

Approval decision도 core 소유 single-flight 권한입니다. Core는 활성 turn의 현재 pending approval과
일치하는 decision만 승인하고 submitting/submitted 상태를 소유하며 중복 provider 제출을 막습니다.
Composition은 provider 호출을 실행하고 correlation이 유지된 completion을 반환합니다. TUI는 approval
modal projection과 재시도 상태 문구만 소유합니다.

Approval review stream update는 Core 소유 직렬 effect queue를 통해 영속화합니다. 각 write는 정확한
turn, workspace, thread, review와 correlation되며, composition이 기존 idempotent Review Center write를
TUI thread 밖에서 실행합니다. 지연된 실패는 해당 conversation identity가 여전히 현재일 때만
표시되므로 이전 write의 오류가 새 conversation에 추가되지 않습니다.

GitHub review setup과 polling도 core-correlated 작업입니다. 첫 frame이 실제 전달되기 전 TUI는 환경
값을 `PendingFirstFrame`으로 순수하게 파싱할 뿐 Git, credential, process, network discovery를
실행하지 않습니다. Draw와 draw 이후 size 검증이 모두 성공한 뒤에만 해당 frame epoch의
`SetupGithubReviewPolling`을 정확히 한 번 보냅니다. Core는 단조 증가 setup generation, 정확한
workspace correlation, 동일 요청 coalescing, target 승인, 성공한 poll cursor, stale/duplicate
completion 규칙을 소유합니다. 새 workspace setup은 A→B→A race를 포함해 이전 service, target,
cursor, 진행 중 poll을 모두 무효화합니다. Composition은 TUI thread 밖에서 branch discovery와
service 구성을 실행하고 정확히 승인된 setup correlation 하나의 service만 보관하므로 다른
generation의 service로 poll할 수 없습니다. TUI에는 poll 간격과 `PendingFirstFrame` /
`Discovering` / `Active` / `Disabled` / `SetupError` 표현만 남습니다.

Parallel peek load는 core-correlated read입니다. Core가 요청 thread에 단조 증가 generation을
부여하고 최신 요청으로 이전 요청을 대체하며, 일치하지 않거나 중복된 completion은 TUI에 도달하기
전에 버립니다. TUI는 agent 선택과 loading/status/scroll 표현을 소유하고 다른 shell overlay가
대체하면 preview를 지웁니다. Peek 결과는 interactive conversation을 절대 교체하지 않습니다.

Review Center read도 같은 latest-wins 경계를 따릅니다. Core가 workspace와 선택적 active thread를
correlation으로 묶고 effect runner에서 authority read를 시작하며 stale 또는 중복 completion을
버립니다. Section별 실패는 core 소유 snapshot의 data로 남으므로 한 source의 실패가 다른 section을
숨기지 않습니다. TUI는 overlay lifecycle과 표시용 thread 문맥만 소유하고 현재 workspace 또는
thread identity가 바뀌면 새 load를 요청합니다.

Session rename도 core-correlated single-flight 작업입니다. TUI는 Core가 exact correlation과 함께
accepted admission을 반환한 뒤에만 pending 상태로 진입합니다. Active rename, catalog load,
conversation load 충돌은 typed rejection으로 반환되어 provider effect를 시작하지 않고 editor draft를
보존합니다. Provider의 성공 응답을 core가 수락하면 일치하는 catalog row, 불러온 conversation title,
stream identity를 함께 갱신합니다. 충돌하는 catalog load와 같은 thread의 conversation load는 버리지
않고 rename 완료 뒤로 지연하므로 예전 read가 이전 title을 복원할 수 없습니다. Core는 자신의 active
full correlation과 일치하지 않는 provider completion을 버립니다. TUI는 Core가 이미 수락한
completion의 semantic projection을 항상 적용하고, exact local admission correlation은 editor draft,
pending feedback, status, selected row의 표시 정산만 제어합니다.

## 상태 권한

| 상태 | 권한 소유자 |
| --- | --- |
| cursor, modal, overlay, editor buffer, selected row | inbound adapter |
| session/conversation lifecycle, in-flight effect, stream reduction | core |
| parallel wake/effect ordering, stale-completion guard | application control-plane |
| task/direction/queue authority, lease, session record, delivery claim | SQLite-backed store |
| eligibility, capacity, retry, validation, stale-event decision | domain |
| 전달된 frame, viewport/back-buffer 신뢰, host-scrollback receipt, terminal 복구 | terminal transaction/adapter |

Invariant에 영향을 주거나 재시작 후에도 남아야 하는 상태는 TUI에만 둘 수 없습니다. Rendering이나
focus만을 위한 상태는 domain authority로 올리지 않습니다. Adapter-local presentation state는 Core가
이미 소유한 semantic lifecycle 또는 in-flight operation 권한을 중복해서는 안 됩니다. Terminal
delivery state는 host terminal이 실제로 수락한 결과이므로 client state transition 성공만으로
추론하면 안 됩니다.

## Planning 경계

```text
inbound adapter
  -> application/service/planning
  -> application/port
  -> adapter/outbound/{db,filesystem,app_server}
```

Private SQLite store가 권한을 가집니다. Planning workspace file은 운영자가 편집하는 상세 문서,
prompt, staged draft, export, 복구 근거입니다. 승인 mutation은 revision-aware validation을 통과하고,
hidden worker output은 SQL이나 보호된 planning file을 직접 쓰지 않습니다.

Planning setup, Planning Doctor, reset 실패 복구는 기존 Core planning-runtime refresh effect 하나를
공유합니다. Application inspection use case 하나가 workspace 존재·부재 모두에서 aggregate workspace
record를 정확히 한 번 읽고 runtime projection과 Core 소유 doctor snapshot을 함께 반환하므로 TUI는
workspace를 두 번째로 검사하지 않습니다. Core는 planning-runtime coordinator 하나에서 generation과
active correlation을 소유합니다. 정확한 correlated effect completion만 coordinator를 완료할 수 있고,
post-turn이나 generic projection writer는 init, doctor, reset-recovery 결과를 대신할 수 없습니다.
같은 workspace의 더 최신 writer는 진행 중 read를 명시적으로 supersede하고 replacement inspection을
예약하므로, 이전 성공 read가 writer projection을 덮거나 operation을 완료할 수 없습니다.
TUI는 정확한 operation, operation 시작 시점의 presentation revision,
`Idle | Loading | Ready | Failed` 상태만 보관합니다. Replacement inspection 재바인드는
correlation만 바꾸고 최초 revision을 보존합니다. Stale, duplicate, ABA, 닫힌 overlay, workspace
drift, 더 최신 UI intent completion은 setup 분기나 status를 바꿀 수 없습니다. Inspection 실패는
workspace 부재와 구분되고, reset 복구는 reset error와 inspection error를 모두 보존합니다.

TUI reset, simple draft stage/load/promotion, planning/directions editor stage/save/promotion은
Core 소유 planning-workspace operation coordinator 하나를 공유합니다. Core는 정확한 workspace와
operation kind, 해당되는 reset target, simple draft/session identity, editor-stage target 또는
editor-mutation identity를 포함한 단조 증가 correlation을 발급합니다. Editor mutation identity는
action, planning/directions target, draft, 전체 source editor session, session-local buffer
revision을 포함합니다. Editor-stage target은 planning manual, 정확한 direction id를 가진 direction
detail, queue-idle prompt를 구분합니다. 완전히 같은 operation을 반복하면 worker를 추가로 시작하지
않고 coalesce하며 다른 operation은 busy로 응답합니다. Composition은 TUI input thread
밖에서 provider 작업을 실행하고 panic도 정확히 같은
correlated error completion으로 변환합니다. 이 worker들은 공용 panic-redaction 경계를 사용하므로
provider panic payload를 stderr에 노출하지 않습니다. Coordinator는 정확한 completion만 완료합니다. Core는
correlation과 다른 reset target, editor-stage target/session 또는 빈 draft, editor load session,
promotion draft를 거절합니다. Runner는 provider I/O 전에 operation, workspace, action, target,
draft, source session 또는 buffer revision이 request와 다른 editor mutation을 거절한 뒤 기존 save
또는 promote use case 중 정확히 하나만 호출합니다. Stale, duplicate, workspace drift, target
drift, wrong-kind, ABA completion은 결과를 표시할 수 없습니다.

수락된 simple stage는 stage generation에서 session identity를 만들고, 수락된 editor load는
load-generation identity로 교체합니다. TUI는 review/editor state와 이 identity를 함께 보관하므로
늦은 load가 더 최신 editor를 바꾸거나 늦은 promotion이 더 최신 overlay를 닫을 수 없습니다. Editor
file body는 typed request/completion payload로만 전달되고 debug output에서는 가려지며 correlation에는
들어가지 않습니다. Planning manual staging은 기존 planning `Loading` step을 사용하고, directions
staging은 summary, selection, pending direction을 보존하는 별도 `EditorLoading` step을 사용합니다.
현재 presentation과 정확히 일치하는 completion만 editor를 엽니다. 실패하면 planning manual은
detail selection, direction detail은 confirmation, queue-idle은 directions overview로 돌아갑니다.
Coalesced retry는 현재 presentation revision에 다시 연결됩니다. Overlay close, workspace drift,
approval 전환은 background operation을 완료하되 더 최신 UI 뒤에서 editor를 열지 않습니다. Exact reset 또는 promotion completion의 workspace가 현재 workspace라면 A→B→A
회귀 뒤에도 TUI는 항상 post-turn continuation을 멈추고 Core runtime projection을 다시 읽습니다.
별도 presentation revision은 status와 overlay 변경만 제한하므로 더 최신 UI intent의 문구는
유지하면서 authority/runtime reconciliation은 계속 수행합니다. Overlay를 닫거나 바꿔도 파괴 작업을
취소하지 않습니다. Editor buffer revision은 session마다 0에서 시작하고 실제 body mutation 뒤에만
증가합니다. Save 중에도 편집할 수 있으며 continuation pause나 runtime authority refresh를 실행하지
않습니다. 정확한 completion은 validation을 갱신하고 dirty를 해제하지만, 이전 revision completion은
validation을 갱신할 수 있어도 최신 body와 dirty를 보존합니다. Promote 중에도 편집할 수 있습니다.
현재 workspace의 정확한 promote completion은 presentation이 바뀌었더라도 항상 continuation을
멈추고 runtime authority를 다시 읽습니다. 양수 성공이며 현재 target, session, buffer revision과
정확히 일치할 때만 planning을 닫거나 directions maintenance를 overview로 돌립니다. 0건 성공은
저장된 draft를 reconcile한 뒤 editor를 열어 두고, 오류나 stale presentation/session/revision
completion은 editor state를 닫거나 교체하지 않습니다. Planning/directions save/promote 진입점
4개는 모두 typed Core mutation command를 dispatch합니다. Production TUI는 planning workspace
use-case handle을 직접 노출하지 않으며 Rust-aware architecture guard는 direct method, UFCS, path,
function pointer, macro 호출 형태를 모두 거절합니다. 이는 coordinator와 worker path 하나이며
deferred queue, actor, cancellation abstraction이 아닙니다.

TUI Queue overlay를 열면 먼저 shell chrome을 반영한 뒤 core load command를 dispatch합니다. Core는
workspace와 active thread identity에 단조 증가 generation을 부여하고 최신 요청으로 이전 요청을
대체하며 stale 또는 중복 completion을 버립니다. Composition은 coherent application read를 실행하고
runtime projection, planning revision, task, typed failure를 core 소유 snapshot으로 매핑합니다.
Presentation은 이 immutable screen model만 읽습니다. Loading과 failed 상태에서는 remove/undo를
비활성화하고, ready 상태는 visible row, selection, revision, destructive-action token을 한 snapshot에
묶습니다. Workspace, thread, visible revision이 바뀌면 새 correlated load를 시작하며 redraw와
resize에서는 authority I/O를 수행하지 않습니다.

TUI queue remove와 undo intent는 core command로 진입합니다. Core는 workspace, active thread identity,
base planning revision, 정확한 task status/update token에 단조 증가 generation을 부여하고 mutation
하나만 승인하며 stale, duplicate, ABA completion을 버립니다. Composition은 이 core 소유 intent를
`PlanningQueueUseCases`로 매핑합니다. Application cancellation transaction은 request 제출과 성공/실패
뒤 coherent runtime 및 queue-authority readback을 함께 수행합니다. TUI controller는 remove/undo
presentation만 소유하고 승인된 correlation의 mutation kind와 captured receipt로 settlement를
projection하며, worker를 예약하거나 operation ID를 만들지 않습니다. 같은 workspace/thread context의
정확히 승인된 correlation만 반환된 authority를 projection에 반영할 수 있습니다. Queue overlay를
닫아도 승인된 mutation은 유지되고 Core가 completion을 소비할 때까지 중복 destructive intent를
차단합니다.

`AKRA_HOME`은 신뢰할 수 있는 절대 경로여야 합니다. SQLite DB와 sidecar는 private regular
single-link file이어야 합니다. Repository incarnation marker는 재사용된 checkout 경로가 예전
저장소의 권한을 자동으로 채택하지 못하게 합니다.

## Parallel 경계

```text
TUI intent
  -> application control-plane handle
  -> domain decision
  -> durable store / effect runner
  -> projection
  -> core snapshot / TUI rendering
```

현재 control-plane은 mutex로 직렬화한 synchronous facade입니다. Application writer 하나, effect
accounting, stale completion drop, wake coalescing, durable backpressure, 단일 projection source를
제공합니다. 이 결정을 다시 검토하지 않고 mailbox actor, TUI/core의 raw parallel service owner,
두 번째 dispatch queue를 추가하지 않습니다.

주기적인 pending dispatch poll은 이 facade가 승인하지만 durable authority 읽기는 effect runner에서만
실행합니다. Runtime은 정확한 workspace와 epoch에 묶인 단조 증가 operation을 할당하고 poll 하나만
진행하며 이후 tick을 합칩니다. 정확히 일치하는 completion만 wake 또는 후속 tick을 시작할 수 있고,
disable, workspace 전환, duplicate, ABA completion은 폐기합니다. 읽기 실패는 현재 projection을
보존하고 다음 주기 tick이 재시도가 됩니다.

Post-turn continuation은 승인된 완료 평가와 같은 runtime projection의 queue-head 값을 그대로
사용하며 facade lock 안에서 planning authority를 다시 읽지 않습니다. Durable dispatch enqueue와
cancel도 effect-runner worker에서만 실행합니다. Runtime은 이를 단조 증가
operation/workspace/epoch correlation으로 직렬화하고 supervisor refresh, queued wake, pending poll
순서를 보존합니다. 첫 worker가 실행 중이어도 workspace/epoch/canonical durable trigger가 같은
enqueue 의도는 하나로 합칩니다. 따라서 slot-capacity와 task-intake 요청은 동일한 durable
task-intake mutation을 공유합니다. Cancel은 대기 enqueue보다 먼저 정확히 닫힌 epoch에 대해
끝까지 실행합니다. 현재 workspace의 cancel 실패는 상태에 표시합니다. 모든 cleanup 실패는
원래 operation/workspace/epoch/command identity와 함께 bounded unsettled-cleanup ledger에
남습니다. 대응하는 전역 runtime notice는 conversation의 Loading/Failed 상태와 독립적으로
보존되므로, 나중에 Ready conversation이 생기면 대체 workspace projection을 바꾸지 않고
표시됩니다. 정확한 correlation 재시도는 control-plane effect runner를 통해 원래 cancellation을
다시 실행합니다. Production TUI control-plane pulse는 conversation의 Loading/Failed 상태와
무관하게 기존 bounded 주기마다 가장 오래된 미정산 exact correlation을 재시도합니다. 활성
replacement epoch 작업은 refresh, queued wake, pending poll 우선순위를 유지하고, cleanup은 한 번
bounded defer한 뒤 다음 idle pulse를 받아 어느 lane도 굶지 않습니다. Cleanup cadence는 pending
poll cadence와 분리하며 mutation coalescing이 재시도를 single-flight로 유지합니다. 성공하면
ledger를 정산하고 notice를 지웁니다. 그 밖의 늦거나 ABA인 completion은 진단에만 남습니다.

Pool mutation은 repository-scoped OS lock도 획득합니다. 각 allocation은 추측할 수 없는 정확한
generation을 받고 lease, session, event, delivery, cleanup까지 전달됩니다. 지연 event는 mutation
전에 같은 generation인지 비교합니다. 모든 지원 플랫폼에서 SQLite가 권한을 가집니다.

Post-turn mutation은 continuation permit과 parallel epoch permit을 캡처합니다. 긴 작업은 제한된
commit section 밖에서 실행합니다. 무효화된 permit 결과는 진단에 남을 수 있지만 task authority를
변경하거나 delivery를 enqueue할 수 없습니다.

## 전달과 process 경계

Parallel acquisition은 push remote, credential 없는 canonical GitHub URL, repository/visibility,
integration branch, fetched base, source SHA, ordered reviewed range를 고정합니다. Remote 또는 review
drift는 해당 mutation 전에 차단합니다.

Host 소유 Git 작업은 다음을 지킵니다.

- 신뢰하는 native executable을 고정하고 상속된 Git 실행/route 제어를 제거합니다.
- hook, fsmonitor, signing, replacement object, lazy fetch, prompt, external diff를 끕니다.
- mutation 전에 effective repository/worktree config key 이름을 검사합니다.
- 실행 가능한 filter/driver, unsafe worktree redirect, alternate ref, external tool, 특수 경로,
  과도한 변경, unmerged entry, 숨겨진 dirty submodule state를 거부합니다.
- 격리된 credential-free network config와 별도로 검증한 API identity를 사용합니다.

Subprocess는 `src/subprocess.rs`를 사용합니다. Linux는 process-group과 제한된 `/proc` descendant
containment, Windows는 kill-on-close Job, 그 밖의 Unix는 process-group containment를 보장합니다.
같은 OS 사용자 프로세스의 악의적인 race는 filesystem-only threat boundary 밖입니다.

## TUI 소유권

TUI 변경은 state/reducer, controller/effect, projection/copy, theme/chrome, rendering/layout,
terminal-adapter 책임을 분리합니다. 시각 token은 `AkraTheme` 뒤에 두고 host scrollback과 live
viewport에 나뉘는 append-only row 사이에 panel chrome을 삽입하지 않습니다.

실제 shell overlay identity 변경은 reducer가 `from`, `to`, exit mode를 담은 typed
`ShellOverlayTransition` 하나로 발행합니다. Root TUI coordinator는 reduced state를 먼저 적용한 뒤
그 transition을 유일한 overlay cleanup owner에 전달합니다. Cleanup owner는 wildcard 없이 모든
`ShellOverlay` variant와 `Suspend`/`Exit` mode를 명시합니다. `Approval`로 진입하는 전환은
suspension으로 departed overlay의 local state를 보존합니다. 이 중 `DirectionsMaintenance`만
approval close 뒤 원래 overlay로 복귀하고, 다른 approval interruption은 in-flight local state를
지우지 않은 채 hidden chrome으로 돌아갑니다. 명시적 close는 `OverlayClosed`만 dispatch하고,
exit cleanup은 반환된 transition에서 정확히 한 번 파생합니다.

같은 terminal transaction은 parallel mode, 진행 중 effect, supervisor inspection, withheld
reason, event-stream fact도 각각 한 번만 캡처합니다. Supersession row plan,
host-scrollback/live-tail 분할, prompt lock, animation, draw는 control-plane mutex나 별도 clock을
다시 읽지 않고 이 immutable projection을 사용합니다. 자주 실행되는 prompt, pulse, scheduler
검사는 panel 전용 경량 projection을 공유하며 transcript나 event-stream row를 복제하지 않습니다.

`Terminal::draw` 전에 terminal transaction은 conversation projection과 활성 overlay 하나를
owned `InlineShellFrameModel`로 합칩니다. 그 안의 `InlineInspectionFrameModel` variant가 view,
widget-local state, geometry에 따른 scroll 결정, feedback 비교 기준을 소유합니다. Capture
boundary는 UI-local state만 읽을 수 있고 Core, application service, parallel control plane,
outbound I/O를 다시 조회할 수 없습니다. Production `shell_rendering.rs`와
`shell_rendering/**`는 이 owned frame model만 소비하고 Ratatui `Frame`만 변경한 뒤
`InlineFrameRenderReceipt`를 반환합니다. `NativeTuiApp`, command dispatch, clock, retained
adapter state는 renderer 경계를 넘지 않습니다.

Terminal transaction은 draw와 draw 이후 terminal size 검증이 성공한 뒤에만 receipt를
commit합니다. Exact attempt gate가 failed, resize-raced, stale, duplicate receipt를 버립니다.
Receipt 적용은 capture 당시 baseline과 현재 값을 비교하므로 activity, editor, help, approval,
session list, queue hit area에 관한 오래된 frame feedback이 더 최신 UI edit를 덮을 수 없습니다.

Session frame capture는 presentation 전에 owned `SessionOverlayScreenModel` 하나를 만듭니다. 이
model은 Core가 발행한 catalog projection, workspace, committed/edited query, project filter,
page projection, stable selected thread identity, page-local selected index, rename editor state,
warning, key availability를 합칩니다. Page projection과 selection repair는 한 번만 수행하며
list/detail/warning/key builder와 renderer는 `NativeTuiApp`이나 service를 다시 읽지 않습니다.
Capture가 owned overlay view와 frame-local Ratatui `ListState`를 완성하고, 안정적으로 전달된
receipt만 그 state를 compare-and-apply합니다. 반복 redraw와 resize는 catalog 작업이나 service
호출을 일으키지 않습니다.

Core는 session catalog의 유일한 admission/correlation authority입니다. Adapter는 startup 또는
overlay open에서 ensure-loaded intent, 명시적 reload에서 refresh intent만 보내며 display
`SessionState`를 보고 요청을 억제하거나 `Loading`을 미리 쓰지 않습니다. Core가 settled-state
정책과 동일 workspace/limit in-flight coalescing을 적용하고 generation/workspace/limit 전체
correlation과 함께 accepted `Loading`을 발행합니다. Adapter의 `SessionState`는 이 Core event의
presentation projection일 뿐입니다. Core가 수락한 rename의 catalog/active-stream semantic
projection은 항상 적용되고, exact local pending receipt는 editor draft, feedback, selection,
status settlement만 제어합니다.

Planning worker 진단은 Core가 시작한 post-turn event부터 비동기 completion과 screen model까지
domain `PlanningWorkerPanelState`를 그대로 보관합니다. TUI presentation은 adapter 소유 status
DTO나 왕복 mapper 없이 label과 content visibility만 파생합니다.

상세한 test-guarded 계약은 [TUI 계층 아키텍처](tui-contract.md)를 참고하세요.

## 금지 방향과 gate

- `domain`은 application, core, adapter, runtime, IO framework를 import하지 않습니다.
- `application`은 core, TUI, HTTP, Telegram, concrete outbound adapter를 import하지 않습니다.
- `core`는 inbound UI/transport type이나 concrete outbound adapter를 import하지 않습니다.
- TUI는 planning/parallel mutation의 core/application command gate를 우회하지 않습니다.
- Outbound adapter는 domain policy를 구현하지 않습니다.

```bash
. "$HOME/.cargo/env"
cargo test --test architecture_boundaries
cargo fmt --all -- --check
cargo test --locked
```

범위가 넓은 native/TUI 변경은 `bash scripts/check_native_pr.sh`를 실행합니다.
