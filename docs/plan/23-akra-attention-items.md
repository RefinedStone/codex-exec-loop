# 23. 주의 항목 (attention items)

상태: **Proposed**. 현재 제품 동작을 설명하지 않는다. 세부 계약과 단계는 아직 없다. · 기준일: 2026-11-12 (Asia/Seoul) ·
기준 커밋: `a6cc327e` (`origin/prerelease`) ·
관련: [plan 17 Akra Desktop](17-akra-desktop/README.md), [plan 18 kernel](18-akra-kernel-architecture.md),
[plan 19 kernel 성질 목표](19-akra-kernel-quality-goals.md), [plan 20 관찰-제안](20-akra-observe-and-offer.md),
[Hide 조사](../competitive/hide/README.md)

## 한 줄 요약

kernel 이 "사람이 지금 해야 하는 것"을 **주의 항목**의 목록으로 한 번만 판정한다. Desktop, TUI, Telegram 은 그
목록을 그리기만 한다. 사이드바 묶음, 행의 이유 한 줄, PR 순서, 완료 기준 카드는 모두 이 목록 위에 올라간다.

## 왜 필요한가

- 지금 Desktop 은 스레드의 표시를 renderer 에서 계산한다 (`desktop/src/renderer/sidebar/threadStatus.ts`,
  값 5개: `approval`, `running`, `failed`, `unread`, `idle`). 이것은 "client 는 얇게"(plan 18) 와 어긋난다.
- 사람이 필요한 일의 절반 이상은 스레드가 아니다. task, PR, 저장소 상태이다 (§3). 스레드 목록만 묶으면 놓친다.
- "왜 멈췄나"는 다섯 곳에 흩어져 있다: `ThreadActivity.approval`, `turn_ended` 기록의 `outcome`,
  `auto_follow_pause_reason`, `skipped_tasks[].reason`, `pr_validation.blocker`.
- Hide 는 이 문제를 상태 5축과 core 의 단일 판정으로 풀었다. Akra 는 typed 이벤트가 있으므로 화면 읽기 없이 같은
  결과를 낼 수 있다.

## 1. 원칙

1. **단위는 주의 항목이다. 스레드가 아니다.** 대상은 스레드, task, PR, 저장소 중 하나이다.
2. **요구와 읽음은 다른 축이다.** 항목은 답했을 때 목록에서 나간다. 봤을 때 나가지 않는다. 배지와 알림은 읽지
   않은 요구만 센다.
3. **kernel 이 한 번만 판정한다.** client 는 그룹, 표시, 문구를 다시 정하지 않는다. 읽음도 kernel 의 기록이다.
4. **이유 한 줄은 원인 코드에서 나온다.** 모델을 부르지 않는다 (plan 17 D-13). 문장의 품질은 요약 모델보다 낮지만
   거짓이 없다.
5. **완료 기준은 기계가 기록한 줄만 믿는다.** 모델의 자기 보고는 통과가 아니다.

## 2. 모양 (제안)

```text
attention 조회와 이벤트 (akra/kernel/query, akra/kernel/event)
  items: [{ cause, subject, since, read }]
    cause    닫힌 목록. kernel spec 의 reasons 처럼 summary 와 next 를 가진다
    subject  { kind: thread | task | pull_request | repository, id }
    since    항목이 생긴 시각
    read     사람이 이 항목을 봤는가
```

그룹은 kernel 이 정한다.

| 그룹 | 조건 |
| --- | --- |
| 내 차례 | 답하지 않은 요구가 있다 (읽었어도 남는다) |
| 읽을 것 | 요구는 없고, 끝났고, 읽지 않았다 |
| 작업 중 | 턴, 승인 대기가 아닌 준비, auto-follow, 병렬 작업이 돈다 |
| 나머지 | 그 밖의 모든 것 |

## 3. 원인의 출처 (kernel 이 이미 아는 것)

`schema/akra-kernel-spec.json` 과 `desktop/src/renderer/kernel/protocol.ts` 에서 확인했다.

| 원인 | 대상 | 지금의 출처 |
| --- | --- | --- |
| 승인 대기 | thread | `thread_activity` 의 `approval`, 기록 `approval_requested` / `approval_resolved` |
| 턴 실패, 중단 | thread | 기록 `turn_ended` 의 `outcome` (`completed`, `interrupted`, `failed`, `unknown`, `lost`) |
| 사용자 답 대기 | task | `planning` 의 task 상태 `awaiting_user` |
| 수락 대기 | task | `planning` 의 `proposed_tasks` |
| 건너뛴 task | task | `planning` 의 `skipped_tasks[].reason` |
| auto-follow 멈춤 | repository | `planning` 의 `auto_follow_pause_reason` |
| planning 고장 | repository | `planning` 의 `status: invalid`, `failure_reason` |
| PR 검증 막힘 | pull_request | `parallel` 의 `pr_validation.state`, `blocker` |
| 병렬 슬롯 막힘 | repository | `parallel` 의 pool `blocked`, `missing`, `unavailable` |

주의: spec 의 `reasons` 19개는 명령 거절의 이유이다 (`busy`, `unknown_thread` 등). "왜 멈췄나"의 목록이 아니다.
주의 원인은 새 닫힌 목록이다. 같은 형식(`summary`, `next`)을 쓴다.

## 4. 코드로 확인한 사실 (2026-11-12)

| 질문 | 답 | 근거 |
| --- | --- | --- |
| 실패한 턴을 기록으로 복원할 수 있는가 | 된다. 단 kernel 이 아는 턴만이다 | `turn_ended` 는 `outcome` 을 가진다. 기록은 SQLite `kernel_events` 에 남고 최근 10,000건을 지킨다 (`src/domain/kernel_event.rs` `RETAINED_KERNEL_EVENTS`, `src/adapter/outbound/db/sqlite_planning_authority_adapter/kernel_events.rs`) |
| kernel 이 Desktop 의 사용자 턴을 아는가 | 모른다 | Desktop 대화는 Gateway 를 지나 upstream 으로 간다. kernel 은 자기가 낸 턴만 기록한다 (plan 20 §2) |
| Gateway 에 탭을 넣을 자리가 있는가 | 있다 | `src/adapter/inbound/gateway/session.rs` 는 양방향의 모든 텍스트 메시지를 `record(direction, text)` 로 넘긴다 (녹화용, 네 곳). 탭은 같은 자리에 들어간다 |
| 탭의 비용은 무엇인가 | JSON 해석이 새로 든다 | `router.rs` 는 `akra/` 가 없는 메시지를 해석하지 않고 넘긴다. 탭은 `turn/*` 와 승인 요청을 가려야 하므로 method 를 읽어야 한다 |
| kernel 이 관찰한 턴을 담을 자리가 있는가 | 없다 | `src/core/app/conversation_runtime.rs` 의 `active_turn` 은 하나이다. 관찰한 턴은 이 자리를 쓰면 안 된다 (plan 20 §4). 기록의 자리는 있다: plan 19 §5 가 `<출처>_<일>` 종류를 제안했다 |
| 읽음이 저장되는가 | 안 된다 | `unreadThreadIds` 는 창 하나의 메모리에 있다 (`desktop/src/renderer/session/desktopStore.ts`). 저장도 공유도 없다 |
| task 에 완료 기준이 있는가 | 없다 | `src/domain/planning` 에 기준 필드가 없다 |
| "흘린 것"의 사실이 이미 있는가 | 병렬 슬롯만 있다 | `pool/slot_inspection.rs` 는 `akra-agent/slot-*` 의 git 상태만 읽어 `Idle`, `Blocked`, `AwaitingCleanup` 등으로 가른다. 다른 worktree 의 "커밋 안 한 변경", "푸시 안 한 커밋", "CI 가 실패한 PR" 은 누구도 모으지 않는다 |
| Desktop 이 git 과 PR 을 읽는가 | 읽는다. 요청한 폴더 하나만이다 | `desktop/src/main/gitStatus.ts` (`git status --porcelain`), `desktop/src/main/githubService.ts` (`reviewDecision`, `statusCheckRollup`) |
| 사용 한도를 읽을 수 있는가 | 프로토콜에 있고 Akra 는 쓰지 않는다 | `account/rateLimits/read`, `account/rateLimits/updated`, `RateLimitSnapshot { primary, secondary: { usedPercent, resetsAt } }` 가 생성된 타입에 있다. `src/` 와 `desktop/src/` 는 부르지 않는다 |

이 확인으로 바뀐 판단 두 개:

- "흘린 것"은 "데이터는 있고 화면만 없다"가 아니다. 슬롯 밖의 worktree 를 읽는 수집기가 새로 필요하다. 그 자리는
  plan 20 §4.1 의 git 관찰자이다.
- 스레드의 판정을 kernel 로 옮기려면 관찰자 모드가 먼저이다. 그 전에 옮기면 판정이 두 곳에 생긴다: kernel 이 낸
  턴은 kernel 이, 사용자 턴은 renderer 가 한다.

## 5. 올라가는 기능

| 기능 | 필요한 것 | 비고 |
| --- | --- | --- |
| 사이드바 묶음 (내 차례, 읽을 것, 작업 중, 나머지) | §2 | 스레드가 아닌 항목도 "내 차례"에 행으로 나온다 |
| 행의 이유 한 줄 | 원인의 `summary` | 모델 호출 없음 |
| PR 을 다음 행동자 순으로 | 대상이 `pull_request` 인 항목 | 순서: 병합 가능 → 내 리뷰 필요 → 에이전트가 고치는 중 |
| 완료 기준 카드 | 병렬 전달이 이미 검증하는 사실 (check, CI, 리뷰, 병합) | task 에 묶인 스레드에서만 그린다. 그 밖에는 그리지 않는다 |
| "흘린 것" | git 관찰자 (plan 20 §4.1) | 행동은 "마무리 맡기기", "놔두기" 두 개 |
| 사용 한도 | `account/rateLimits/*` | 이 계획과 독립이다. 작다 |

## 6. 순서 (제안)

1. `attention` 조회와 이벤트. §3 의 출처만 쓴다. task, PR, 저장소 항목과 kernel 이 낸 턴의 스레드 항목이 나온다.
2. 사용 한도 표시. 1 과 독립이다.
3. Gateway 탭과 관찰자 모드 (plan 20 ①). 그때 스레드의 판정을 `threadStatus.ts` 에서 kernel 로 옮긴다.
4. 읽음을 kernel 기록으로 옮긴다.
5. 완료 기준 카드. 기계가 기록한 줄만 보인다.
6. git 관찰자와 "흘린 것".
7. 나중: 사람이 쓰는 기준, human gate, 프로젝트 가드레일, 통합 이동(`Ctrl+K`).

## 7. 하지 않는 것

- 미션 지도(DAG). 병렬 슬롯이 3개인 동안은 지금의 lane 보드로 충분하다.
- 모델이 쓰는 요약 줄. 원인 코드의 문장으로 시작한다.
- 화면을 읽어서 상태를 추측하는 것.
- Hide 코드의 복사. 라이선스(FSL-1.1-ALv2)가 경쟁 제품에 쓰는 것을 금지한다.

## 8. 미결정

- **여러 저장소의 항목을 누가 모으는가.** 사이드바는 여러 저장소를 보여 준다. kernel 은 저장소당 하나이고 10분 뒤에
  내려간다. 제안: Gateway 가 모은다. kernel 은 저장소 밖을 모른다 (plan 18).
- **내려간 kernel 의 항목.** kernel 이 내려가면 그 저장소의 항목은 마지막 값으로 남는가, 사라지는가.
- **읽음의 범위.** 읽음은 사용자의 것이다. 저장소 kernel 의 기록에 두면 여러 사용자를 가를 수 없다. 지금은 사용자가
  한 명이므로 kernel 기록으로 시작한다.
- **가드레일의 방식.** Hide 는 에이전트 hook 으로 막는다. Akra 는 hook 이 없다. app-server 승인 정책으로 따로
  설계한다.
- **부모-자식 감시와 Project Memory.** 가치는 있다. plan 20 의 세부 설계에서 정한다.

## 9. 확인하지 않은 것

- 관찰자 모드의 구현 크기. 닿는 곳(Gateway 세션, kernel 입력, 기록 종류)만 확인했다.
- Hide 의 실행 동작. 소스와 공개 문서만 읽었다. 미션과 task 식별의 핵심 설계 문서는 비공개 저장소에 있다.
- 탭의 JSON 해석이 일반 대화의 지연에 주는 영향. 측정하지 않았다.
