# 24. 증거로 끝내는 작업 (evidence harness)

상태: **Proposed**. 현재 제품 동작을 설명하지 않는다. 세부 계약은 조각마다 다시 정한다. · 기준일: 2026-11-13 (Asia/Seoul) ·
기준 커밋: `7a4e851d` (`origin/prerelease`) ·
관련: [plan 17 Akra Desktop](17-akra-desktop/README.md) ([R5 하네스 연동](17-akra-desktop/roadmap/R5-harness-integration.md)),
[plan 18 kernel](18-akra-kernel-architecture.md), [plan 19 kernel 성질 목표](19-akra-kernel-quality-goals.md),
[plan 20 관찰-제안](20-akra-observe-and-offer.md), [plan 23 주의 항목](23-akra-attention-items.md),
[OmO 조사](../competitive/omo-native/README.md)

## 한 줄 요약

task 는 **완료 기준**을 갖는다. 기준은 **증거 파일**이 있어야 통과한다. kernel 이 증거를 기록하고, 통과하지 않은 task 는
전달(push, PR, 통합)로 가지 못한다. Akra Desktop 은 기준과 증거를 보여 준다. 판정은 kernel 이 한다.

## 왜 필요한가

2026-11-12 에 Desktop 작업 PR 7개를 두 방식으로 진행했다 (5개와 2개). 결과가 달랐다.

| | 기준과 증거 없이 진행 (#2244~#2253) | OmO 의 `ulw-loop` 로 진행 (#2254, #2255) |
| --- | --- | --- |
| 화면 확인 | PR 4개에 "화면 모양은 확인하지 않았다" 라고 적고 머지했다 | 기능마다 Electron 화면 시험을 새로 만들고 스크린샷을 확인했다 |
| 리뷰 | 코드를 읽는 리뷰. 머지한 뒤에 한 것도 있다 | 리뷰어가 명령을 실행해서 재현했다 |
| 머지 전에 잡은 결함 | #2249 는 머지 뒤 리뷰에서 데이터 손실 4건이 나왔다 (#2250 에서 고침) | `push.followTags` 로 태그가 올라가는 것, hunk 버리기가 줄 끝을 바꾸는 것을 머지 전에 잡았다 |
| 놓친 것 | Codex 리뷰가 사용 한도로 돌지 않았다 | Codex 리뷰 댓글 2건을 기다리지 않고 머지했다 (#2256 에서 고침) |

차이를 만든 것은 모델이 아니다. 구조이다 (OmO 설치본의 `skills/ulw-loop` 를 읽었다).

1. 성공 기준을 실행 전에 정한다. 기준마다 시나리오, 확인하는 곳, 증거 파일 경로가 있다. 모호한 기준은 실행 전에 반려한다.
2. 상태는 한 곳(SDK)만 쓴다. 증거 파일이 없으면 통과 기록을 거절한다.
3. 증거는 코드의 tree 해시에 묶인다. 코드가 바뀌면 그 증거는 무효이다.
4. 만든 쪽과 확인하는 쪽이 다르다. 작업자의 "끝났다"는 주장으로 본다.
5. 멈추는 조건이 있다. 같은 기준이 3번 실패하면 사람에게 넘긴다.

이 계획은 이 구조를 Akra 의 kernel 과 planning 에 넣는다. OmO 의 코드와 문구는 복사하지 않는다 (§8).

## 1. Akra 에 이미 있는 것

코드에서 확인했다 (2026-11-13).

| OmO 의 장치 | Akra | 근거 |
| --- | --- | --- |
| 상태를 한 곳만 쓴다 | 저장소당 kernel 하나, 기록은 SQLite `kernel_events` | plan 23 §4, `schema/akra-kernel-spec.json` (명령 10, 조회 7, 기록 12) |
| 목표와 단계 | planning 의 direction 과 task | `src/domain/planning/mod.rs` `TaskDefinition` |
| 작업자와 worktree 분리 | 병렬 모드: 슬롯 3개, 슬롯별 worktree, lease | `docs/reference/current-product.md` "Parallel Contract" |
| 작업자 보고를 믿지 않는다 | 호스트가 lease, worktree, branch, 고정한 base, 바뀐 파일 범위, 깨끗한 상태를 확인한 뒤 커밋한다 | 같은 문서 |
| 머지 게이트 | 전달은 직렬이다: push → PR 검증 → 검토한 범위만 통합 → 원격 확인 → 정리 | 같은 문서, `src/domain/parallel_mode/pr_validation.rs` `PrValidationPhase` |
| 막힌 이유 | `auto_follow_pause_reason`, `skipped_tasks[].reason`, `pr_validation.blocker` | plan 23 §3 |
| 사람 리뷰 | 기본으로 필요하다 | `current-product.md` |

Akra 는 "누가 하고 어떻게 머지하는가"가 이미 엄격하다.

## 2. 없는 것

| 없는 것 | 지금 | 근거 |
| --- | --- | --- |
| task 의 완료 기준 | 필드가 없다. `description` 한 덩어리만 있다 | `TaskDefinition` 의 필드 목록 |
| 증거 | 작업자가 쓴 글 한 줄이 전부이다 | `PlanningOfficialCompletionRefreshPayload.validation_summary` ("worker가 실행한 validation/test summary") |
| 증거와 코드 상태의 연결 | base OID 와 커밋 SHA 는 고정한다. 증거 개념이 없다 | `current-product.md` |
| 기준을 통과해야 전달한다 | 전달 조건에 기준이 없다 | 위 전달 순서 |
| 기계 리뷰어 | 없다 | - |
| 화면에서 하는 확인 | 에이전트가 Desktop 브라우저를 조작하지 못한다 | plan 17 R9 (계획) |
| Desktop 에서 보기 | Akra 패널은 planning 과 병렬 상태를 읽기만 한다 | `desktop/src/renderer/kernel/` |

`validation_summary` 는 모델의 자기 보고이다. plan 23 의 원칙 5 ("완료 기준은 기계가 기록한 줄만 믿는다")와 어긋난다.

## 3. 원칙

1. **단위는 task 이다.** 일반 대화 스레드는 나중이다 (§6 E7). kernel 은 지금 Desktop 의 사용자 턴을 모른다 (plan 23 §4).
2. **기준은 실행 전에 있다.** 기준이 없는 task 는 병렬 슬롯에 배정하지 않는다. 사람이 "기준 없이 진행"을 고를 수 있다.
   그 선택은 기록에 남는다.
3. **통과는 증거 파일이 정한다.** 모델의 글은 통과가 아니다. 호스트가 파일이 있는지, 비어 있지 않은지, 어느 tree 에서
   만들었는지 확인한다.
4. **증거는 tree 에 묶인다.** 전달하는 커밋의 tree 와 다른 tree 에서 만든 증거는 낡은 것이다. 통과로 세지 않는다.
5. **kernel 이 한 번만 판정한다.** Desktop, TUI, Admin 은 그리기만 한다 (plan 18).
6. **사람 리뷰는 그대로 둔다.** 기계 리뷰어는 더하는 것이다. "사람 리뷰가 기본으로 필요하다"를 바꾸지 않는다.
7. **모델을 부르는 일은 사용자가 시작한 작업 안에서만 한다** (plan 17 D-13). 시험은 가짜 Gateway 와 가짜 작업자를 쓴다.

## 4. 모양 (제안)

### 4.1 완료 기준

`TaskDefinition` 에 목록 하나를 더한다.

```text
acceptance: [{ id, scenario, channel, essential }]
  id         task 안에서 유일하다 (예: C1)
  scenario   무엇을 실행하고 무엇을 보면 통과인가. 한 문장이고 통과와 실패가 갈린다
  channel    닫힌 목록: command | data | http | screen
  essential  false 이면 통과하지 않아도 전달을 막지 않는다
```

- `command`: 명령의 출력과 종료 코드. `data`: 파일이나 DB 의 전후 차이. `http`: 응답. `screen`: 화면 캡처.
- `screen` 은 에이전트가 화면을 조작할 수 있어야 한다. plan 17 R9 가 선행이다. 그 전에는 화면 시험(e2e)의 기록을
  `command` 로 받는다.
- 증거 파일의 경로는 기준에 적지 않는다. kernel 이 정한다 (§4.2). 작업자가 경로를 고르면 작업 트리 밖을 쓸 수 있다.
- `TaskAuthorityDocument` 는 `deny_unknown_fields` 이고 `version` 을 가진다. 필드를 더하면 버전을 올리고, 이전 문서는
  빈 목록으로 읽는다.
- 검증 (`src/domain/planning/validation.rs`): id 중복, 빈 시나리오, 모르는 channel 을 거절한다. "잘 동작하는지 확인" 같은
  문장을 기계가 가려낼 수는 없다. 그것은 리뷰(§4.5)와 사람의 몫이다.

### 4.2 증거

- 자리: `$AKRA_HOME/projects/<project>/runtime/evidence/<task>/<criterion>/<attempt>/`. 작업 트리 밖이다.
  이유: 호스트는 작업자의 바뀐 파일 범위를 확인하고 커밋한다. 증거가 작업 트리에 있으면 커밋에 섞인다. 증거에 비밀 값이
  들어갈 수도 있다. 저장소, 녹화, 로그에 넣지 않는다.
- 작업자는 kernel 이 준 경로에 파일을 쓴다. 그다음 구조화된 명령으로 결과를 알린다. planning 작업자의 명령을 호스트가
  검증하고 적용하는 지금의 방식과 같다.
- 호스트가 확인하는 것: 파일이 그 경로 안에 있다, 비어 있지 않다, 크기 한도 안이다. 호스트가 직접 계산하는 것: 파일의
  SHA-256, 그 worktree 의 `HEAD^{tree}` 와 작업 트리가 깨끗한지.

### 4.3 kernel 계약

`schema/akra-kernel-spec.json` 에 더한다 (plan 19 §5.4: 스펙이 kernel 의 얼굴이다).

| 종류 | 이름 (제안) | 내용 |
| --- | --- | --- |
| 명령 | `record_evidence` | `{ task_id, criterion_id, status: pass \| fail \| blocked, note }`. 증거 파일이 없으면 거절한다 |
| 명령 | `waive_criterion` | 사람만 한다. 기준 하나를 "확인하지 않고 진행"으로 둔다. 이유가 필요하다 |
| 기록 | `criterion_evidence_recorded` | `{ task_id, criterion_id, status, artifact_sha256, tree, attempt, by }` |
| 기록 | `criterion_waived` | `{ task_id, criterion_id, reason, by }` |
| 조회 | `planning` 을 넓힌다 | task 마다 기준 목록과 기준별 상태: `pending`, `pass`, `fail`, `blocked`, `stale`, `waived` |
| 이유 | `criteria_missing`, `criteria_not_met`, `evidence_missing`, `evidence_stale` | 명령 거절과 전달 막힘의 이유. `summary` 와 `next` 를 가진다 |

- `by` 는 명령을 보낸 쪽이다. K4 가 이미 명령의 호출자를 기록한다 (`af44991c`).
- `stale` 은 kernel 이 계산한다: 기록의 `tree` 가 지금 전달하려는 커밋의 tree 와 다르다.
- 기록 이름은 plan 19 §5 의 `<출처>_<일>` 모양을 따른다.

### 4.4 전달 게이트

지금의 순서 앞에 조건 하나를 둔다.

```text
작업자 완료 보고 → 호스트 확인과 커밋 → [필수 기준이 모두 pass 또는 waived 이고 stale 이 아니다] → push → PR 검증 → 통합
```

- 조건을 통과하지 못하면 전달하지 않는다. 이유는 `criteria_not_met` 또는 `evidence_stale` 이다. plan 23 의 주의 항목으로
  나온다 (대상은 task).
- 호스트가 커밋을 만든 뒤에 tree 가 정해진다. 그래서 순서는 "커밋 → 그 tree 에서 증거 확인"이다. 작업자가 증거를 만든
  tree 와 커밋의 tree 가 같아야 한다. 작업자는 변경을 커밋하지 않고 두므로, 호스트는 증거를 기록할 때 작업 트리의 내용으로
  tree 를 계산한다 (`git write-tree` 를 임시 index 로). 구현 방법은 E2 에서 정한다.
- 멈추는 조건: 같은 기준이 3번 `fail` 이면 task 를 `awaiting_user` 로 둔다. auto-follow 의 진행 없는 반복 중단
  (OmO 조사의 "Adopt" 항목)과 같은 규칙을 쓴다.
- PR 의 외부 리뷰(Codex 등)를 기다리는 것은 PR 검증의 `PreMergeObservation` 이 맡는다. 이 계획은 그 단계를 바꾸지 않는다.
  E3 에서 "리뷰 봇의 미해결 댓글이 있으면 통합하지 않는다"가 지금 동작인지 확인한다.

### 4.5 게이트 리뷰어

- 구현한 슬롯과 다른 슬롯이 읽기 전용으로 검토한다. 입력: task, 기준, diff, 증거 파일의 경로.
- 리뷰어는 재현한다. 결과는 구조화된 값이다: `recommendation: approve | reject`, `blockers[]`, 실행한 명령과 출력의 경로.
- 기록: `gate_review_recorded { task_id, tree, recommendation, blockers, by }`.
- `reject` 이면 task 를 다시 연다. 수정 뒤에는 바뀐 부분만 다시 본다. 다시 보는 것은 2번까지이다. 그 뒤에는
  `awaiting_user` 이다.
- 한계를 적는다: 바뀐 부분만 다시 보면 수정이 만든 다른 회귀를 놓칠 수 있다. 그래서 수정 뒤에는 낡은 증거를 모두 다시
  만든다 (§3 원칙 4 가 이것을 강제한다).
- 슬롯은 3개이다. 리뷰어가 하나를 쓰면 구현은 2개가 된다. E5 에서 정한다.

### 4.6 Akra Desktop

Desktop 은 그리기만 한다. 계산하지 않는다.

- Akra 패널의 task 카드: 기준 목록, 기준별 상태, 증거 열기(읽기 전용), 낡은 증거 표시.
- "기준 없이 진행"과 "이 기준은 확인하지 않고 진행"은 사람의 명령이다. main 이 확인 창을 띄운다.
- 대화에서 task 만들기 (R5 범위 3): 스레드의 결론을 task 로 큐에 넣는다. 이때 기준을 함께 쓴다.
- 전달이 막힌 task 는 plan 23 의 "내 차례"에 이유 한 줄과 함께 나온다.
- 여러 저장소의 값을 누가 모으는가는 plan 23 §8 의 미결정과 같다. 여기서 따로 정하지 않는다.

## 5. 기준은 누가 쓰는가

| 쓰는 쪽 | 장점 | 위험 |
| --- | --- | --- |
| 사람 | 원하는 결과와 맞다 | 번거롭다. 쓰지 않게 된다 |
| planning 작업자 (모델) | 항상 있다 | 쉬운 기준을 쓴다. 자기가 만들 것에 맞춘 기준을 쓴다 |
| 둘 다 (제안) | 모델이 초안을 쓰고 사람이 task 를 수락할 때 본다 | 사람이 읽지 않고 수락한다 |

제안: 모델이 쓴 task 는 지금도 `Proposed` 상태로 들어오고 사람이 수락한다. 기준도 그때 함께 보인다. 사람이 고칠 수 있다.
구현한 작업자는 자기 task 의 기준을 바꾸지 못한다. 바꾸려면 새 제안을 내야 한다.

## 6. 순서 (제안)

조각 하나가 PR 하나이다. 각 조각은 kernel 시뮬레이션 시험 (`src/composition/kernel_host/simulation.rs`)을 통과한다.

| 조각 | 내용 | 동작 변화 |
| --- | --- | --- |
| E1 | `TaskDefinition.acceptance` 와 검증. Admin 과 TUI 는 읽기만 한다 | 없음. 기준을 적을 수 있게만 된다 |
| E2 | 증거 자리, `record_evidence`, 기록 2종, `planning` 조회에 기준 상태 | 없음. 증거를 기록할 수 있게만 된다 |
| E3 | 전달 게이트와 이유 코드. 기본은 `observe` (막지 않고 기록만 한다) | 설정을 켠 저장소만 막는다 |
| E4 | Desktop: task 카드, 증거 열기, 사람의 두 명령 | 화면 |
| E5 | 게이트 리뷰어 | 슬롯 하나를 쓴다 |
| E6 | `screen` channel | plan 17 R9 뒤 |
| E7 | 일반 대화 스레드 | plan 20 의 관찰자 모드 뒤 |

- E3 는 PR 검증의 `akra.prValidationMode` 와 같은 방식으로 시작한다: `observe` 가 기본이고, 켜야 막는다.
  기준이 없는 기존 task 가 갑자기 막히지 않게 한다.
- E1 과 E2 는 서로 독립이 아니다. E2 는 E1 뒤이다. E4 는 E2 뒤이면 시작할 수 있다.

## 7. 이 계획 자체의 완료 기준

이 계획도 같은 규칙을 쓴다. 조각마다 아래를 PR 에 붙인다.

| 조각 | 기준 | 확인하는 곳 |
| --- | --- | --- |
| E1 | 기준이 있는 task 문서와 없는 이전 문서를 모두 읽는다. 중복 id 와 빈 시나리오를 거절한다 | `command`: 단위 시험의 출력 |
| E2 | 파일 없는 `record_evidence` 를 거절한다. 기록에 호스트가 계산한 SHA-256 과 tree 가 있다. 코드를 바꾸면 상태가 `stale` 이 된다 | `command`: 시뮬레이션 시험, `data`: `kernel_events` 의 줄 |
| E3 | `observe` 에서는 전달하고 기록만 남는다. 켜면 필수 기준이 없는 task 의 push 가 일어나지 않는다 | `data`: 로컬 bare 원격의 ref 목록 |
| E4 | 카드가 kernel 의 상태를 그대로 보여 준다. renderer 에 판정 코드가 없다 | `command`: Electron 화면 시험과 스크린샷 |
| E5 | `reject` 가 task 를 다시 연다. 3번째에는 `awaiting_user` 이다 | `command`: 시뮬레이션 시험 |

## 8. 하지 않는 것

- OmO 의 코드와 스킬 문구의 복사. 구조만 참고한다. OmO 의 라이선스는 이 계획에서 확인하지 않았다.
- 모델이 쓴 요약을 통과로 세는 것.
- 작업 지도(DAG). plan 23 §7 과 같다.
- 사람 리뷰를 기계 리뷰로 바꾸는 것.
- 증거 파일을 저장소에 커밋하는 것.
- OmO 식의 "통과하면 바로 끝낸다". Akra 는 PR 검증 단계를 지난다.

## 9. 미결정

- **증거의 보존.** 크기 한도와 지우는 때. task 가 `Done` 이 된 뒤 며칠 두는가.
- **증거를 만든 tree 의 계산.** 작업자는 커밋하지 않는다. 호스트가 작업 트리로 tree 를 계산하는 방법 (§4.4).
- **병렬 모드 밖의 task.** 주 대화의 auto-follow 로 하는 task 에도 게이트를 두는가. 거기에는 전달 단계가 없다.
- **기준을 통과시키는 쪽.** 구현한 작업자가 자기 증거를 기록해도 되는가, 다른 슬롯만 되는가. OmO 는 지휘자가 직접 한다.
- **plan 23 과의 경계.** plan 23 §5 의 "완료 기준 카드"와 §6 의 7번 "사람이 쓰는 기준"은 이 계획이 맡는다. plan 23 에는
  주의 항목의 원인(`criteria_not_met` 등)만 남긴다. plan 23 을 고칠 때 맞춘다.
- **R5 와의 경계.** R5 의 범위 2~4 (계획 보기, 대화를 작업으로, 실행과 결과)는 E4 와 겹친다. R5 를 이 계획의 E4 로
  대신할지 정한다.

## 10. 확인하지 않은 것

- OmO 의 GitHub 저장소. 이 PC 의 설치본(`…/omo-runtime/win32-x64/plugin/skills/ulw-loop`)과 `.omo/evidence/` 의 기록만 읽었다.
- OmO 의 이번 실행은 SDK 의 목표 등록 없이 돌았다 (`ULW_LOOP_PLAN_MISSING` 기록). 즉 §"왜 필요한가"의 장치가 그 실행에서
  전부 작동한 것은 아니다. 좋은 결과가 장치에서 왔는지, 스킬 문서의 지시에서 왔는지는 가르지 못했다.
- 병렬 전달 코드의 세부 (`src/application/service/parallel_mode/`). 문서(`current-product.md`)와 타입 이름만 확인했다.
  게이트를 넣을 정확한 자리는 E3 에서 정한다.
- PR 검증이 리뷰 봇의 미해결 댓글을 막는지.
- 증거 기록이 kernel 기록의 보존 한도(최근 10,000건, plan 23 §4)에 주는 영향.
- 2026-11-12 의 비교는 PR 7개이고 작업의 종류가 다르다. 통제한 실험이 아니다.
