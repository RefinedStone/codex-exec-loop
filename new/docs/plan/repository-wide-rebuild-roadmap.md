# Repository-Wide Rebuild Roadmap

## 문서 지위

`new/docs`에서 현재 읽어야 하는 기준 문서는 아래뿐이다.

- [../architecture/parallel-control-plane-architecture.md](../architecture/parallel-control-plane-architecture.md)
- [../architecture/core-runtime-boundary-architecture.md](../architecture/core-runtime-boundary-architecture.md)
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

전체 상태: `complete` (현재 rebuild roadmap 기준)

기존 R-slice 상당수는 완료됐지만, core runtime boundary 기준으로 TUI가 아직
소유한 duplicate projection cache가 rendering/status/post-turn policy 입력의
authority가 되는 경로는 제거했다. Ready conversation compatibility cache는 아직
private reducer/event 호환 범위로 남아 있지만, core planning projection과 동기화되는
보조 사본일 뿐이다.

완료된 이동: manual prompt intake/bootstrap은
`AppCommand::PrepareManualPrompt` -> `CoreEffect::PrepareManualPrompt` ->
application `ManualPromptPreparationService` 경로로 들어간다. TUI turn submission
path는 prompt buffer, stale completion guard, bootstrap review overlay 적용만 맡는다.
parallel-mode 표시 accessor는 core `AppSnapshot.planning_parallel.parallel_mode`를
읽고, 별도 TUI write-through cache/fallback field는 제거했다. planning footer의
loading/failed 경로도 render 중 application service를 다시 호출하지 않고 core
snapshot projection을 읽는다.
Ready conversation의 planning footer, queue overlay, planning status tail, existing
workspace popup도 core planning projection을 읽는다. conversation planning cache는
private reducer/event compatibility cache로 축소했고 resumed-session
status copy와 post-turn evaluation context도 core projection을 읽는다.

| 영역 | 남은 문제 |
| --- | --- |
| planning/parallel projection consumption | 현재 roadmap 기준 잔여 항목 없음. rendering/status/post-turn evaluation context는 core projection을 읽는다. |
| runtime vocabulary | 현재 roadmap 기준 잔여 항목 없음. 새 slice를 추가할 때도 Command/Input/Effect/Completion/Event/Snapshot 의미를 유지한다. |

## 실행 Backlog

| Slice | 상태 | 목표 |
| --- | --- | --- |
| CORE-MANUAL-INTAKE-01 | done | manual prompt intake/bootstrap을 core command/effect로 이동하고 TUI는 prompt buffer와 overlay만 소유한다. |
| CORE-PROJECTION-02 | done | parallel rendering source와 loading/failed planning indicator를 `AppSnapshot` projection 우선 읽기로 전환한다. |
| CORE-READY-PLANNING-03 | done | Ready conversation planning rendering source를 core planning projection으로 전환한다. |
| CORE-PARALLEL-CACHE-04 | done | parallel write-through cache를 줄여 event application은 core projection만 갱신하고 TUI field fallback을 제거한다. |
| CORE-READY-CACHE-05 | done | Ready conversation planning cache를 reducer/event compatibility 범위로 더 좁히고 남은 core projection sync contract를 정리한다. |
| CORE-POST-TURN-PROJECTION-06 | done | post-turn evaluation context의 current runtime projection source를 core projection으로 옮겨 Ready conversation compatibility cache 의존을 줄인다. |

## 문서 운영 규칙

- 새 `new/docs` 문서는 명시적인 architecture decision이나 사용자 요청이 있을 때만 추가한다.
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
