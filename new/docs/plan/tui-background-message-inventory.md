# TUI Background Message Inventory

## 목적

이 문서는 `TUI-00B`의 산출물이다. 기준 architecture는
[../architecture/tui-application-boundary-architecture.md](../architecture/tui-application-boundary-architecture.md)
이며, `BackgroundMessage`가 durable state를 직접 바꾸는 우회 channel이 되지 않도록
각 variant의 source와 target boundary를 고정한다.

## Target Boundary 규칙

`BackgroundMessage` handler는 아래 둘 중 하나만 허용한다.

1. TUI reducer/controller event로 전달해 UI-only state 또는 projection cache를 갱신한다.
2. application service/runtime event로 되돌려 domain/application decision을 다시 통과시킨다.

handler가 DB, filesystem planning authority, parallel dispatch queue, task authority를
직접 변경하면 안 된다.

## Variant Inventory

| BackgroundMessage | Source | Target boundary | 허용 mutation |
| --- | --- | --- | --- |
| `StartupLoaded` | startup service effect | `ShellChromeEvent::StartupLoaded` reducer | startup projection cache, draft workspace sync, startup submit queue resolution |
| `SessionsLoaded` | session service effect | `ShellChromeEvent::SessionsLoaded` reducer | session projection cache, session overlay UI reset |
| `ConversationLoaded` | conversation service effect | `ConversationLifecycleEvent::ConversationLoaded` reducer | active conversation projection cache, planning runtime snapshot refresh, overlay content reset |
| `ConversationStream` | app-server stream effect | `ConversationRuntimeEvent::StreamUpdated` reducer | conversation transcript/status projection only |
| `ConversationRuntimeNotice` | stream/post-turn effect runner | `ConversationRuntimeEvent::StreamExecutionObserved` reducer | runtime notice copy only |
| `OperatorAlert` | application effect result | TUI alert rendering helper | operator-visible alert copy only |
| `InvalidateParallelModeSupervisorSnapshot` | stream/parallel effect | parallel panel/application refresh request | supervisor projection invalidation, no durable queue write |
| `WakeParallelModeOrchestrator` | post-turn/parallel scheduling effect | parallel application wake request | application control-plane wake, TUI epoch/correlation update only |
| `ParallelModeEnterProgress` | parallel enable effect | parallel panel projection cache | readiness/supervisor projection cache and status copy |
| `ParallelModeEntered` | parallel enable effect | parallel panel projection cache | enabled flag, readiness/supervisor projection cache and status copy |
| `ParallelModeSupervisorSnapshotRefreshed` | parallel refresh effect | parallel panel projection cache | supervisor projection cache only |
| `ParallelModeOrchestratorWakeCompleted` | parallel control-plane effect | parallel application result mapping | readiness/supervisor projection cache, dispatch outcome notice |
| `ParallelModeWorkerEvent` | worker completion channel | parallel application control-plane worker event | application service handles worker completion; TUI only routes |
| `ParallelModeOrchestratorTickCompleted` | parallel tick effect | parallel panel projection cache/application wake state | blocked status and notices; no durable queue mutation |
| `PostTurnEvaluated` | post-turn execution effect | stale guard, then `ConversationRuntimeEvent::PostTurnAutomationEvaluated` reducer | planning worker panel projection, runtime snapshot projection, automation provenance, queued auto prompt effect |
| `GithubReviewPollLoaded` | GitHub polling effect | GitHub polling controller/projection cache | polling projection cache only |

## Regression Anchors

Existing and new tests that protect this boundary:

| Contract | Test |
| --- | --- |
| startup background result enters shell chrome reducer | `startup_background_message_updates_app_state` |
| resumed conversation reloads planning projection through lifecycle path | `resumed_session_status_surfaces_planning_and_queue_context` |
| conversation stream message enters conversation runtime reducer | `conversation_stream_background_message_is_routed_through_runtime_reducer` |
| stale post-turn result cannot overwrite current conversation/projection | `stale_post_turn_evaluation_background_message_is_ignored` |
| duplicate post-turn result for the same turn is ignored | `duplicate_post_turn_evaluation_for_same_turn_is_ignored` |
| background drain is bounded and schedules another poll | `background_drain_budget_yields_to_terminal_events` |

## 다음 Slice

`TUI-00C`는 prompt lock, overlay focus, selection/cursor와 projection update 충돌을 막는
input/rendering regression을 추가했다. 완료 문서는
[tui-shell-regression-anchors.md](./tui-shell-regression-anchors.md)이다.
