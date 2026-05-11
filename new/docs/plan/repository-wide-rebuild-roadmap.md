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

전체 상태: `partial`

기존 R-slice 상당수는 완료됐지만, core runtime boundary 기준으로 TUI가 아직
소유한 orchestration이 남아 있다. 완료 판정은 아래 항목이 사라진 뒤에만 가능하다.

| 영역 | 남은 문제 |
| --- | --- |
| post-turn continuation | completion payload 용어와 stale/duplicate guard는 정리됐지만, evaluator spawn/timeout owner가 아직 TUI path에 있다. |
| manual prompt intake/bootstrap | planning intake와 first-use planning scaffold 실행을 TUI turn submission path가 직접 호출한다. |
| planning/parallel projection consumption | core `AppSnapshot`에 projection은 있지만 TUI가 여전히 별도 cache를 primary source처럼 갱신한다. |
| runtime vocabulary | post-turn legacy 용어는 제거했다. 다음 slice도 Command/Input/Effect/Completion/Event/Snapshot 의미를 기준 문서대로 제한해야 한다. |

## 실행 Backlog

| Slice | 목표 |
| --- | --- |
| CORE-POSTTURN-03 | post-turn evaluator 실행과 timeout completion을 core/application effect completion으로 옮긴다. |
| CORE-MANUAL-INTAKE-01 | manual prompt intake/bootstrap을 core command/effect로 이동하고 TUI는 prompt buffer와 overlay만 소유한다. |
| CORE-PROJECTION-02 | planning/parallel rendering source를 `AppSnapshot` projection으로 수렴하고 TUI duplicate cache를 줄인다. |

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
