# Repository-Wide Rebuild Roadmap

## 문서 지위

`new/docs`에서 현재 읽어야 하는 기준 문서는 아래뿐이다.

- [../architecture/parallel-control-plane-architecture.md](../architecture/parallel-control-plane-architecture.md)
- [../architecture/core-runtime-boundary-architecture.md](../architecture/core-runtime-boundary-architecture.md)
- [parallel-control-plane-migration-plan.md](./parallel-control-plane-migration-plan.md)
- 이 문서

기존 repository-wide rebuild roadmap은 완료됐다. 이 문서는 완료된 core-runtime
slice 기록을 반복하지 않고, TUI를 business/orchestration owner에서 thin view adapter로
줄이는 남은 작업만 추적한다. 완료된 inventory, 중복 architecture, 과거 migration memo,
완료 slice 상세는 Git history로 충분하다.

## 고정 원칙

레이어 책임은 아래로 고정한다.

| Layer | 해야 할 일 | 하면 안 되는 일 |
| --- | --- | --- |
| `adapter/inbound/*` | 입력 해석, auth/context mapping, UI-only state, rendering | domain policy, durable mutation, worker launch decision |
| `application/service/*` | command/use case, ordering, transaction, port effect orchestration | surface별 정책 복제, invariant를 큰 `if/else`로 계속 키우기 |
| `domain/*` | invariant, pure decision, state transition rule | I/O, thread/channel, adapter/application 의존 |
| `application/port/outbound/*` | 외부 boundary trait과 request/result | concrete DB/git/filesystem detail |
| `adapter/outbound/*` | DB/git/filesystem/GitHub/app-server mapping | use case policy, domain rule |

state owner는 먼저 분류한 뒤 이동한다. TUI에 남는 상태는 presentation state여야 하고,
app lifecycle, business decision, background orchestration은 core/application/domain에 둔다.

| State | Owner |
| --- | --- |
| overlay, cursor, selection, local editor buffer, transcript render cache | TUI |
| visible projection cache | inbound adapter cache, production authority 아님 |
| in-flight effect id, wake coalescing, poll timer, epoch gate, completion stale gate | core/application runtime |
| task authority, dispatch command, lease, session detail, distributor queue | durable store |
| eligibility, capacity, retry, validation, stale event decision | domain |

## 현재 판정

| 기준 | 판정 |
| --- | --- |
| `new/docs` 기존 rebuild roadmap | 완료 |
| TUI business/orchestration 제거 | 상당히 완료, 약 85-90% |
| TUI thin view adapter 전환 | 아직 진행 중, 약 65-75% |

완료된 기준선은 짧게만 보존한다. startup/session/conversation load와 turn stream의
주요 orchestration은 core runtime 경로로 이동했다. manual prompt intake/bootstrap은
`AppCommand::PrepareManualPrompt` -> `CoreEffect::PrepareManualPrompt` ->
application `ManualPromptPreparationService` 경로를 탄다. parallel/planning 표시 경로는
core `AppSnapshot` projection을 우선 읽고, TUI write-through cache/fallback authority는
제거됐다. Ready conversation의 compatibility cache는
`reducer_event_projection_cache`로 낮췄고, production rendering/post-turn worker
context가 이 cache를 authority로 읽지 못하게 source guard를 둔다. Post-turn
stale/duplicate completion guard는 core turn-stream completion boundary로 이동했고,
TUI는 core가 emit한 accepted completion만 presentation state에 적용한다.

남은 기준은 TUI가 화면 adapter로만 남는지이다.

| 영역 | 남은 문제 |
| --- | --- |
| conversation lifecycle presentation reducer | `ConversationState::Loading/Ready/Failed`가 app lifecycle 의미까지 담는 경로가 남아 있다. core snapshot을 lifecycle authority로 읽고, TUI에는 prompt buffer, cursor, transcript render cache만 남긴다. |
| inline shell/overlay routing | inline command, overlay, auto-follow controls 중 application/domain policy가 섞인 routing을 재분류해야 한다. TUI-only routing은 유지하고 business decision은 application/domain으로 이동한다. |

## 실행 Backlog

| Slice | 상태 | 목표 |
| --- | --- | --- |
| TUI-THIN-CONVERSATION-04 | next | `ConversationState::Loading/Ready/Failed` 중 app lifecycle 의미를 core snapshot 기준으로 읽게 하고, TUI에는 prompt buffer, cursor, transcript render cache만 남긴다. |
| TUI-THIN-ROUTING-05 | todo | inline command, overlay, auto-follow controls 중 application/domain policy가 섞인 routing을 재분류한다. TUI-only routing은 유지하고 business decision은 application/domain으로 이동한다. |

## 문서 운영 규칙

- 새 `new/docs` 문서는 명시적인 architecture decision이나 사용자 요청이 있을 때만 추가한다.
- 새 작업은 이 문서의 `TUI-THIN-*` slice를 갱신한다.
- 완료된 slice 상세와 `done` 행은 이 문서에서 제거한다.
- 완료 기준선은 현재 판단에 필요한 1-2문단 요약으로만 남긴다.
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
