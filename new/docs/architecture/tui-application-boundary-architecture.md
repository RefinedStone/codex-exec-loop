# TUI Application Boundary Architecture

> 상태: 배경 기준 문서다. 현재 구현 판정과 다음 작업은
> [repository-wide-rebuild-roadmap.md](../plan/repository-wide-rebuild-roadmap.md)를 따른다.
> 이 문서의 migration slice 문구는 현재 완료 판정이 아니다.

## 목적

이 문서는 `DOC-TUI-00`의 산출물이다. 목표는 TUI를 "화면 중심"으로 유지하되
policy 중심으로 두지 않는 것이다. Shell, conversation, post-turn automation,
planning handoff, parallel handoff가 모두 `NativeTuiApp` 주변에 모이면서 TUI가
application/domain 판단을 흡수하기 쉬워졌다. 이후 `TUI-00`과 `TUI-01`은 이 문서를
기준으로 state inventory와 regression을 먼저 고정한 뒤 구조를 이동한다.

상위 기준은 다음 문서다.

- [repository-wide-rebuild-architecture.md](./repository-wide-rebuild-architecture.md)
- [planning-control-plane-architecture.md](./planning-control-plane-architecture.md)
- [parallel-control-plane-architecture.md](./parallel-control-plane-architecture.md)

## 핵심 원칙

TUI는 inbound adapter다.

```text
keyboard / terminal / shell event
  -> TUI controller or reducer
  -> application command/effect request
  -> application service/runtime
  -> domain decision
  -> repository/port effect
  -> application projection/event
  -> TUI projection cache and rendering
```

TUI가 직접 소유할 수 있는 것은 화면 수명과 입력 수명에 묶인 state다. 재시작 후
복구되어야 하거나 worker/thread 완료 순서에 따라 보상 처리가 필요한 state는
application 또는 repository 경계로 내려가야 한다.

## 역할 분리

| 역할 | 위치 예시 | 소유 책임 | 금지 |
| --- | --- | --- | --- |
| Controller | `shell_chrome.rs`, `planning/controller/*`, `parallel_mode/panel_controller.rs` | key/form/ui event를 UI state 전이와 effect 의도로 낮춘다 | domain policy 재계산, repository write |
| Reducer | `conversation_runtime.rs`, `conversation_lifecycle.rs`, `conversation_input.rs` | 이미 발생한 TUI event를 순수 state transition과 effect list로 바꾼다 | thread spawn, DB/file access, hidden worker retry 판단 |
| Presentation | `shell_presentation/*`, overlay view modules, status panels | projection/cache를 화면 copy와 layout으로 낮춘다 | queue head 산출, validation severity 재판단 |
| Runtime bridge | `app_runtime.rs`, `shell_runtime.rs`, `turn_submission_runtime/*` | reducer effect를 service/port 호출로 실행하고 결과를 background message로 되돌린다 | business rule을 새로 만들거나 durable state를 직접 mutate |
| Application service | `src/application/service/*` | command/event ordering, side effect orchestration, projection rebuild | terminal key/focus/cursor state 보유 |
| Domain | `src/domain/*` | invariant, policy, pure decision | TUI, thread, channel, filesystem/DB 의존 |

## State Ownership

### UI-Only State

아래 state는 TUI가 소유한다. application service로 올리면 surface별 사용성 상태가
business state처럼 보이게 된다.

| State | 현재 위치 예시 | 이유 |
| --- | --- | --- |
| prompt input buffer와 prompt lock 표시 | `conversation_input.rs`, `ConversationViewModel` 표시 field | operator가 지금 입력 가능한지 보여 주는 화면 상태다. 실제 turn 실행 가능성은 application/runtime event 결과로 온다. |
| shell overlay identity와 focus | `ShellOverlay`, `ExitConfirmationState` | terminal focus와 popup stack 수명에 묶인다. |
| selected row/page/scroll/viewport | session overlay, planning overlay, supersession MUD UI | 표시와 탐색 state다. 같은 projection을 다른 surface가 다르게 탐색할 수 있다. |
| editor cursor, dirty flag, close confirmation | planning draft editor controller/UI | draft editing ergonomics다. save/promote는 application command로 나간다. |
| overlay step과 local form buffer | planning init, task intake, directions maintenance | multi-step UI flow의 진행 상태다. 저장 전에는 authority가 아니다. |
| inline history render mode와 terminal layout option | shell rendering/runtime | terminal 환경 적응이다. domain/application 의미가 없다. |

UI-only state는 background message가 와도 직접 durable state로 승격하지 않는다. 필요한
경우 application command를 호출하고, 성공 결과 projection을 다시 캐시한다.

### Application Projection Cache

아래 state는 application이 계산한 projection을 TUI가 표시하기 위해 cache한 값이다.
TUI는 이 값을 재계산하지 않고 stale 여부만 표면화한다.

| Cache | 현재 위치 예시 | 원천 |
| --- | --- | --- |
| planning runtime snapshot / queue 표시 | `ConversationViewModel`, planning status/queue presentation | `PlanningServices` application projection |
| planning worker panel status | `PlanningWorkerPanelState` | post-turn execution 결과 |
| parallel readiness/supervisor snapshot | `parallel_mode_*snapshot` fields | `ParallelModeService` control plane |
| session catalog | `SessionState`, session overlay state | `SessionService` |
| startup diagnostics | `StartupState` | `StartupService` |
| GitHub review polling result | `GithubReviewPollingState` | GitHub polling service |

Projection cache는 rendering을 빠르게 하기 위한 local copy다. cache 안에서 queue
ordering, proposal promotion, parallel dispatch 가능 여부를 다시 판단하면 안 된다.

### Runtime Bridge State

일부 state는 현재 TUI runtime bridge에 있지만 최종 소유자는 application runtime인
것들이 있다. `TUI-00`은 이 state를 inventory로 고정하고, `TUI-01` 이후 slice에서
이동 여부를 결정한다.

| State | 현재 위치 예시 | 목표 |
| --- | --- | --- |
| in-flight refresh/wake/tick flags | `ParallelModeControlPlaneRuntime` | TUI는 helper 조회와 effect runner만 담당하고 command correlation은 application runtime store가 소유 |
| active turn execution snapshot capture | `ActiveTurnExecutionSnapshotCapture` | post-turn reconciliation의 input fact로 application event에 싣는다 |
| post-turn repair/queue refresh progress | `PlanningWorkerPanelState` | application event result를 표시하는 projection으로 낮춘다 |
| queued auto prompt metadata | `PostTurnAutomationProvenance`, `QueuedAutoPrompt`, `AutoFollowSubmitContext` | completed turn id와 planning/parallel handoff는 automation provenance로 묶고, prompt submit payload만 TUI effect에 남긴다 |
| conversation aggregate correlation | `ConversationViewModel` active turn, auto-follow, planning handoff, post-turn duplicate guard fields | `conversation_state`는 하위 field inventory를 authority로 삼고, 남은 correlation state는 application runtime store로 이동 검토 |

## Conversation Lifecycle 과 Automation Lifecycle

Conversation lifecycle은 operator 또는 auto-follow가 실제 Codex turn을 시작하고,
stream event를 받아 transcript와 runtime status를 갱신하는 흐름이다.

```text
SubmitPrompt
  -> reducer records submitted transcript state
  -> StartStream effect
  -> app-server stream event
  -> ConversationStream background message
  -> reducer updates conversation projection
```

Automation lifecycle은 turn 종료 뒤 planning/parallel state를 검사하고 다음 action을
결정하는 흐름이다.

```text
Stream finished
  -> EvaluatePostTurnAutomation effect
  -> post-turn executor calls planning/parallel application services
  -> PostTurnEvaluated background message
  -> PostTurnAutomationEvaluated reducer event
  -> reducer records projection and either queues an auto prompt or stops
```

분리 규칙:

- manual prompt submit은 conversation lifecycle의 시작이다.
- auto-follow, planning repair, queue refresh, official completion refresh는 automation lifecycle이다.
- automation은 TUI background thread에서 durable state를 직접 고치지 않는다. application service를 호출하고 결과 event로 돌아온다.
- reducer는 `PostTurnAutomationEvaluated` 결과를 표시하고 다음 effect를 큐잉할 수 있지만, queue policy를 다시 판단하지 않는다.
- repeated queue head, queue idle, repair eligibility, parallel official completion 같은 판단은 domain/application 결과를 따른다.

## Background Message 규칙

`BackgroundMessage`는 effect가 끝난 뒤 TUI event loop로 되돌아오는 사실이다. 이 enum이
application/domain을 우회하는 write channel이 되면 안 된다.

허용:

- service call 결과를 success/error projection으로 전달
- stream event를 conversation reducer에 전달
- application runtime worker event를 application service로 되돌리는 wake-up 전달
- operator notice나 alert를 표시 state로 전달

금지:

- `BackgroundMessage` handler가 task authority, parallel queue, session catalog durable state를 직접 mutate
- stale completion을 TUI field 비교만으로 accepted result로 처리
- planning/parallel policy를 background handler에서 새로 계산
- background message가 overlay cursor, prompt buffer 같은 UI-only state를 암묵적으로 초기화

모든 background handler는 다음 둘 중 하나여야 한다.

1. TUI reducer event로 들어가 UI-only/projection cache를 갱신한다.
2. application command/event로 들어가고, 그 결과가 다시 projection으로 돌아온다.

## Prompt Lock, Overlay, Selection, Cursor

이 state들은 TUI 전용이다.

- prompt lock은 "현재 입력을 받을 수 있는가"를 화면에 반영한다. 실제 실행 가능성은 conversation runtime/application state가 결정한다.
- overlay는 terminal focus owner다. planning init, task intake, queue, sessions, help는 하나의 visible owner만 가져야 한다.
- selection과 cursor는 projection의 row id를 가리킬 수 있지만 projection ordering을 만들면 안 된다.
- close confirmation은 editor buffer와 validation result의 UI risk다. 저장, promote, reset은 application command다.

## Migration Slices

### TUI-00. Inventory And Regression

`TUI-00`은 구조를 이동하지 않는다. 먼저 아래 산출물을 만든다.

- `NativeTuiApp` field별 state ownership inventory
- background message별 target boundary inventory
- shell input/rendering regression
- prompt lock, overlay focus, selection/cursor가 projection update와 충돌하지 않는 regression

완료 전에는 conversation/automation split을 시작하지 않는다.

### TUI-01. Conversation And Automation Split

`TUI-01`은 `TUI-00` inventory를 기준으로 다음 이동을 작게 나눈다.

- conversation reducer는 manual/auto prompt submission과 stream event projection에 집중한다.
- post-turn automation execution은 application event/effect vocabulary로 낮춘다.
- planning/parallel handoff result는 TUI가 copy/rendering만 맡는 projection으로 들어온다.
- background message는 automation 결과를 직접 mutate하지 않고 reducer 또는 application command로 전달한다.
- `conversation_state`는 opaque `Hybrid`가 아니라 하위 field별 UI-only, Application Projection Cache,
  Runtime Bridge inventory를 따른다.

## Verification 기준

- shell runtime input tests는 key event가 같은 UI intent로 낮아지는지 확인한다.
- shell rendering snapshot tests는 projection cache 표시가 깨지지 않았는지 확인한다.
- conversation runtime tests는 manual turn, stream completion, post-turn event, queued auto prompt를 분리해서 검증한다.
- planning/parallel handoff tests는 TUI가 domain/application policy를 재계산하지 않는지 확인한다.
- 문서 변경은 `git diff --check`로 whitespace regression을 확인한다.
