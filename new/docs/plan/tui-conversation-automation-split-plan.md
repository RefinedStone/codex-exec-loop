# TUI Conversation And Automation Split Plan

## 목적

이 문서는 `TUI-01A`의 산출물이다. `TUI-00`에서 고정한 state inventory와 regression
anchor를 기준으로 conversation lifecycle과 post-turn automation lifecycle을 분리하는
작업 단위를 정의한다.

상위 기준은 다음 문서다.

- [../architecture/tui-application-boundary-architecture.md](../architecture/tui-application-boundary-architecture.md)
- [tui-shell-state-inventory.md](./tui-shell-state-inventory.md)
- [tui-background-message-inventory.md](./tui-background-message-inventory.md)
- [tui-shell-regression-anchors.md](./tui-shell-regression-anchors.md)

목표는 TUI를 application service로 올리는 것이 아니다. TUI는 inbound adapter로 남고,
그 안에서 controller/reducer/runtime bridge 경계를 더 선명하게 둔다. business policy는
domain/application 결과를 따르고, TUI는 operator input mapping, local focus state,
projection 표시, effect 실행 bridge만 맡는다.

## 현재 Fan-In

`TUI-01`에서 다루는 주요 fan-in은 아래 파일이다.

| 파일 | 현재 역할 | 분리 필요성 |
| --- | --- | --- |
| `src/adapter/inbound/tui/app/conversation_runtime.rs` | prompt submit, stream event, post-turn result, auto prompt queue를 하나의 reducer/effect vocabulary로 처리한다 | conversation event와 automation result가 같은 event enum에 섞여 있어 후속 worker가 policy 위치를 오해하기 쉽다 |
| `src/adapter/inbound/tui/app/app_runtime.rs` | reducer effect 실행, post-turn/parallel continuation, pending task-intake flush를 조립한다 | reducer 후처리와 automation routing이 함께 있어 event ordering 계약을 문서화해야 한다 |
| `src/adapter/inbound/tui/app/shell_runtime.rs` | `BackgroundMessage::PostTurnEvaluated` stale guard, panel projection 갱신, supervisor invalidation, reducer dispatch를 수행한다 | background handler가 durable policy owner처럼 보이지 않게 target boundary를 고정해야 한다 |
| `src/adapter/inbound/tui/app/turn_submission_runtime/post_turn_execution.rs` | post-turn planning/parallel service 호출, repair/refresh 결과, auto-follow action을 만든다 | effect executor와 automation application boundary의 책임을 구분해야 한다 |
| `src/adapter/inbound/tui/app/parallel_mode.rs` | post-turn queue signal을 parallel orchestrator wake로 변환한다 | conversation auto-follow와 parallel handoff가 같은 후처리 경로에서 충돌하지 않게 해야 한다 |

## Lifecycle 정의

### Conversation Lifecycle

conversation lifecycle은 실제 Codex turn의 시작과 provider stream projection을 소유한다.

```text
Prompt intent
  -> ConversationRuntimeEvent::SubmitPrompt
  -> ConversationRuntimeEffect::StartStream
  -> app-server stream
  -> BackgroundMessage::ConversationStream
  -> ConversationRuntimeEvent::StreamUpdated
  -> transcript/status projection
```

소유 책임:

- manual prompt와 auto prompt를 동일한 turn submission protocol로 낮춘다.
- active turn id, transcript insertion, provider stream status, stream failure 표시를 갱신한다.
- stream completion fact를 automation lifecycle로 넘기는 effect를 만든다.

금지:

- queue head 반복 여부, planning repair eligibility, parallel dispatch 가능 여부를 새로 판단하지 않는다.
- planning file/workspace/DB를 직접 읽거나 쓰지 않는다.
- post-turn service 결과 없이 auto-follow prompt를 조립하지 않는다.

### Automation Lifecycle

automation lifecycle은 turn 종료 후 application/domain 결과를 모아 다음 action을 결정한다.

```text
Stream finished
  -> ConversationRuntimeEffect::EvaluatePostTurnAutomation
  -> post-turn executor
  -> planning/parallel application services
  -> BackgroundMessage::PostTurnEvaluated
  -> stale/correlation guard
  -> ConversationRuntimeEvent::PostTurnAutomationEvaluated
  -> QueueAutoPrompt, alert, parallel wake, or stop projection
```

소유 책임:

- turn completion과 workspace snapshot을 input fact로 받아 planning/parallel application service를 호출한다.
- planning repair/queue refresh, official completion refresh, parallel queue signal을 typed result로 만든다.
- `ConversationPostTurnEvaluation`은 accepted authority가 아니라 application/domain 결과 projection과 다음 effect 의도다.

금지:

- `BackgroundMessage::PostTurnEvaluated` handler가 task authority, parallel queue, workspace file을 직접 mutate하지 않는다.
- stale guard를 TUI-local display copy만으로 통과시키지 않는다.
- automation 결과를 conversation stream failure/success와 같은 의미로 합치지 않는다.

## Target Boundary

`TUI-01` 이후 목표 vocabulary는 아래처럼 나눈다.

| Boundary | 입력 | 출력 | 위치 |
| --- | --- | --- | --- |
| Conversation reducer | prompt submit, stream event, stream execution notice | conversation state, `StartStream`, `EvaluatePostTurnAutomation` effect | `adapter/inbound/tui` |
| Automation router/controller | post-turn evaluated fact, queued task-intake readiness, parallel queue signal | auto prompt submit intent, panel projection update, parallel wake request | `adapter/inbound/tui` |
| Post-turn application bridge | completed turn fact, planning changed files, workspace snapshot | `ConversationPostTurnEvaluation` 또는 후속 typed automation result | `adapter/inbound/tui` runtime bridge calling application services |
| Application services | planning/parallel commands and projections | domain decisions, durable writes, runtime projection | `src/application/service` |
| Domain | queue follow, repair eligibility, task mutation, parallel continuation decision | pure decision/result | `src/domain` |

중요한 점은 automation router/controller도 inbound TUI 경계에 둔다는 것이다. 이것을
application service로 올리면 terminal-specific prompt copy, overlay/panel 표시, auto prompt
transcript marker가 application concern처럼 보인다. application으로 올릴 것은 business
command/event, projection, correlation fact이지 TUI event loop 자체가 아니다.

## State 이동 기준

| State | 현재 분류 | 목표 |
| --- | --- | --- |
| prompt input buffer, overlay focus, cursor/selection | UI-only | TUI controller/presentation에 유지 |
| transcript, stream status, runtime notices | Application Projection Cache plus UI display | conversation reducer가 표시 projection으로만 갱신 |
| `active_turn_execution_snapshot_capture` | Runtime Bridge | post-turn request input fact로 명시하고, 장기적으로 automation correlation store로 이동 검토 |
| `planning_worker_panel_state` | Application Projection Cache | post-turn result projection으로만 갱신, decision 재계산 금지 |
| queued auto prompt metadata | Runtime Bridge | automation result provenance로 분리하고, conversation lifecycle에는 submit intent로만 재진입 |
| parallel post-turn queue signal | Application/domain result | automation router가 parallel application wake request로 변환 |

## Slice 분할

### TUI-01A. Split Plan And Ownership Contract

상태: `done`

산출물:

- 이 문서
- roadmap의 `TUI-01` 작업 단위 분할
- `PLAN-02` 상세 상태 불일치 정리

검증:

- `git diff --check`
- `TUI-01A`, `conversation lifecycle`, `automation lifecycle`, `PostTurnEvaluated`,
  `ConversationRuntimeEvent`, `BackgroundMessage` 용어 검색

### TUI-01B. Post-Turn Automation Router Extract

상태: `done`

목적:

- `shell_runtime.rs`와 `app_runtime.rs`에 흩어진 post-turn automation routing을 TUI-side
  automation router/controller로 모은다.
- behavior를 바꾸지 않고 stale guard, planning worker panel projection assignment,
  parallel supervisor invalidation, reducer dispatch 순서를 명시한다.

소유 범위:

- `src/adapter/inbound/tui/app/shell_runtime.rs`
- `src/adapter/inbound/tui/app/app_runtime.rs`
- 새 파일이 필요하면 `src/adapter/inbound/tui/app/post_turn_automation*.rs`
- `src/adapter/inbound/tui/app/shell_runtime/tests/*`

금지:

- post-turn application service 호출을 새 router로 복사하지 않는다.
- stale guard를 약화하지 않는다.
- TUI router가 planning/parallel domain policy를 새로 계산하지 않는다.

검증:

- `cargo test stale_post_turn_evaluation_background_message_is_ignored`
- `cargo test duplicate_post_turn_evaluation_for_same_turn_is_ignored`
- `cargo test conversation_stream_background_message_is_routed_through_runtime_reducer`
- 영향 범위가 넓으면 `cargo test shell_runtime`

완료 근거:

- `post_turn_automation.rs`가 `BackgroundMessage::PostTurnEvaluated`의 stale guard, applied
  turn 기록, planning worker panel projection 갱신, supervisor invalidation, reducer
  dispatch를 소유한다.
- `ConversationRuntimeEvent` reducer 이후 pending task-intake flush와 parallel post-turn
  continuation routing도 같은 TUI-side router에 모았다.
- post-turn application service 호출과 domain/application policy 판단은 기존 위치에 남겼다.

### TUI-01C. Conversation Reducer Vocabulary Split

상태: `done`

선행:

- `TUI-01B`

목적:

- conversation reducer의 event/effect 이름에서 stream lifecycle과 automation result를 구분한다.
- `EvaluatePostTurnAutomation` effect와 `PostTurnAutomationEvaluated` event를 사용해
  auto-follow가 post-turn automation의 일부 action임을 드러낸다.
- `PostTurnAutomationEvaluated` 적용은 conversation projection update와 automation action
  dispatch가 명확히 나뉘도록 테스트를 보강한다.

소유 범위:

- `src/adapter/inbound/tui/app/conversation_runtime.rs`
- `src/adapter/inbound/tui/app/conversation_model.rs`
- `src/adapter/inbound/tui/app/app_runtime.rs`
- 관련 conversation runtime tests

금지:

- user-visible transcript/status copy를 의미 없이 변경하지 않는다.
- auto prompt submit 경로를 manual prompt submit 경로와 분리 구현하지 않는다.

검증:

- `cargo test conversation_runtime`
- `cargo test shell_runtime`

완료 근거:

- `ConversationRuntimeEffect::EvaluateAutoFollow`를
  `ConversationRuntimeEffect::EvaluatePostTurnAutomation`으로 바꿨다.
- `ConversationRuntimeEvent::PostTurnEvaluated`를
  `ConversationRuntimeEvent::PostTurnAutomationEvaluated`로 바꿨다.
- `BackgroundMessage::PostTurnEvaluated`는 background worker 완료 fact 이름으로 남기고,
  TUI-side automation router가 reducer vocabulary로 변환한다.

### TUI-01D. Automation Provenance And Handoff Contract

상태: `done`

선행:

- `TUI-01C`

목적:

- queued auto prompt metadata, planning handoff, parallel handoff signal을 automation result
  provenance로 묶는다.
- pending task-intake flush와 parallel post-turn continuation이 같은 completed turn fact를
  소비한다는 ordering contract를 테스트로 고정한다.

소유 범위:

- `src/adapter/inbound/tui/app/app_runtime.rs`
- `src/adapter/inbound/tui/app/parallel_mode.rs`
- `src/adapter/inbound/tui/app/turn_submission_runtime/post_turn_execution.rs`
- 관련 shell runtime/conversation tests

금지:

- parallel wake 가능 여부를 TUI에서 새로 판단하지 않는다.
- task-intake와 auto-follow 중복 submit guard를 제거하지 않는다.

검증:

- pending task-intake flush regression
- parallel post-turn queue continuation regression
- `cargo test shell_runtime`
- `cargo test parallel_mode`

완료 근거:

- `ConversationPostTurnEvaluation`에 `PostTurnAutomationProvenance`를 추가해 completed
  turn id, planning handoff task, parallel queue signal을 하나의 automation result provenance로
  묶었다.
- `QueuedAutoPrompt`는 prompt/mode/transcript submit payload만 보유하고, completed turn id와
  planning handoff는 provenance에서 `QueueAutoPrompt` effect로 전달한다.
- parallel post-turn continuation은 `evaluation.provenance.parallel_queue_signal`을 읽는다.
- `queued_auto_prompt_uses_post_turn_provenance_for_handoff` regression으로 queued auto prompt가
  action payload가 아닌 provenance의 completed turn/handoff를 사용하는 계약을 고정했다.

### TUI-01E. Conversation State Hybrid Retirement Audit

상태: `ready`

선행:

- `TUI-01D`

목적:

- [tui-shell-state-inventory.md](./tui-shell-state-inventory.md)의 `conversation_state`
  `Hybrid` 항목을 하위 field 기준으로 재분류한다.
- Runtime Bridge로 남은 field는 후속 application runtime store slice 또는 TUI-only 유지 사유를
  명확히 남긴다.

소유 범위:

- `new/docs/plan/tui-shell-state-inventory.md`
- `new/docs/plan/tui-background-message-inventory.md`
- 필요한 경우 `new/docs/architecture/tui-application-boundary-architecture.md`

금지:

- 실제 코드 이동 없이 inventory만 `done`처럼 보이게 하지 않는다.
- UI-only state를 application/domain 소유로 승격하지 않는다.

검증:

- `git diff --check`
- inventory와 실제 field 이름 일치 검색

## Regression Matrix

`TUI-01`의 코드 slice는 최소한 아래 contract를 보존해야 한다.

| Contract | Anchor |
| --- | --- |
| delayed post-turn result는 현재 thread/turn projection을 덮지 않는다 | `stale_post_turn_evaluation_background_message_is_ignored` |
| 같은 turn의 post-turn result는 중복 적용되지 않는다 | `duplicate_post_turn_evaluation_for_same_turn_is_ignored` |
| stream background message는 conversation reducer로만 들어간다 | `conversation_stream_background_message_is_routed_through_runtime_reducer` |
| supervisor projection refresh는 overlay focus와 selection을 보존한다 | `parallel_projection_refresh_preserves_supersession_overlay_focus_and_selection` |
| queued auto prompt는 manual prompt와 같은 submit protocol로 stream을 시작한다 | conversation runtime queue/submit tests |
| parallel post-turn continuation은 domain/application decision signal을 따른다 | parallel mode tests |

## 완료 기준

`TUI-01` 전체 완료는 아래 조건을 모두 만족해야 한다.

- conversation lifecycle이 prompt submit과 stream projection 중심으로 읽힌다.
- automation lifecycle이 post-turn result, auto prompt provenance, planning/parallel handoff를
  별도 route로 읽힌다.
- `BackgroundMessage::PostTurnEvaluated`는 stale/correlation guard 이후 reducer 또는
  application event로만 이동한다.
- TUI에 남은 controller/router는 inbound-specific mapping만 소유하며 business policy를 만들지 않는다.
- `conversation_state`의 `Hybrid` 분류가 제거되거나 남은 항목별 유지 사유가 문서화된다.
