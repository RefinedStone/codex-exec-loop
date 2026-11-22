# 28. 이슈 자동화 (issue automation)

상태: **Proposed**. 현재 제품 동작을 설명하지 않는다. 세부 계약은 조각마다 다시 정한다. 구현 PR의 조각별 상태는 §17에 기록한다. · 기준일: 2026-11-20 (Asia/Seoul) ·
기준 커밋: `d1ee3ad0` (`origin/prerelease`) ·
관련: [plan 17 Akra Desktop](17-akra-desktop/README.md), [plan 18 kernel](18-akra-kernel-architecture.md),
[plan 19 kernel 성질 목표](19-akra-kernel-quality-goals.md), [plan 20 관찰-제안](20-akra-observe-and-offer.md),
[plan 23 주의 항목](23-akra-attention-items.md), [plan 24 증거로 끝내는 작업](24-akra-evidence-harness.md),
이슈 #2414 (Issue Core epic), 이슈 #2481 (ADE 로드맵 epic)

개정 (2026-11-20): PR #2533 의 사후 리뷰를 반영했다. 실행기와의 계약 (§5.4), 수집의 재동기화 (§13), 끝 원인별 전이와
외부 닫힘 (§12.1, §12.2), 결정 요청의 판 (§10.1), 조각의 선행 조건과 완료 기준 (§17, §18) 을 고쳤다.

## 한 줄 요약

Akra 가 외부 이슈를 수집하고, 정책으로 판정하고, 조건이 맞으면 기존 계획 큐와 병렬 슬롯으로 실행하고, 결과를 이슈에
돌려준다. 사람은 모든 단계에서 묻고, 답하고, 멈출 수 있다. 사람이 마감까지 답하지 않으면, 정책이 허락한 범위에서
중재자가 결정한다.

## 1. 사용자가 원하는 것 (요청 2026-11-20)

- 이슈는 여러 플랫폼(GitHub, Jira 등)에서 온다.
- 이슈마다 "바로 구현해도 되는가, 사람의 리뷰가 필요한가"를 가르는 체계가 있다. 그 체계는 이슈 작성에도 영향을 준다.
- 이슈가 등록되고 조건이 맞으면 자동으로 처리한다. 풀자동과 반자동이 있다. 풀자동에도 사람이 직접 확인하는 예외가 있다.
- 정한 시간 안에 사람이 진행하지 않으면 판정자가 결정하고 진행한다.
- Akra Desktop 에 저장소별 화면이 있다. 이 기능은 나중에 다른 저장소로 떼어낼 수 있다.
- 사람과 AI 의 양방향 상호작용은 반드시 지킨다.

## 2. 용어

요청의 말은 임시 이름이다. 이 문서는 아래 용어를 쓴다.

| 요청의 말 | 이 문서의 용어 | 뜻 | 기존 코드와 계획의 대응 |
| --- | --- | --- | --- |
| issueManager | 이슈 자동화 (issue automation) | 기능 전체의 이름 | 새 모듈 |
| 수집 worker | 수집기 (intake), 이슈 출처 (issue source) | provider 에서 이슈의 목록과 변화를 읽는다 | plan 20 §4.1 의 관찰자 |
| 상태를 관리하는 worker | 조정기 (reconciler) | 추적 이슈의 상태를 가지고 다음 행동을 고른다 | plan 19 §5.2 의 FSM 과 맞추기 |
| 실행 worker | 실행기 (executor) | 작업 지시를 받아 코드를 바꾸고 PR 을 만든다 | planning 큐, 병렬 슬롯, 전달 |
| 이슈 | 추적 이슈 (tracked issue) | Akra 가 장부에 올린 외부 이슈 한 건 | `IssueReference` |
| 템플릿, 체계 | 실행 계약, 자동화 정책, 플레이북 | §7 의 세 층 | `issue_template`, `issue_policy`, skill |
| 바로 구현 / 사람 리뷰 | 판정 (triage), 관문 (gate) | §8, §9 | - |
| 풀자동, 반자동 | 자율 수준 (autonomy level) `auto`, `gated` | §9 | `akra.prValidationMode` 와 같은 방식 |
| 사람이 확인하는 예외 | 사람 필수 규칙 | §9.3 | - |
| 이슈 judge | 중재자 (arbiter) | 마감이 지난 결정 요청을 대신 결정하는 읽기 전용 검토자 | plan 24 §4.5 의 게이트 리뷰어 |
| AI 사람 양방향 | 결정 요청 (decision request), 개입 명령 | §10 | plan 18 §4.3 의 4번과 5번 |
| (없음) | 바인딩 (binding) | 이슈 출처 하나와 저장소 하나의 연결 | - |
| (없음) | 작업 지시 (work order) | 실행기에 넘기는 값. Akra 에서는 최초 task 하나와 그 보완 task 들이다 | `TaskDefinition` |
| (없음) | 허가 (permit) | 작업 지시 하나의 단계 하나를 실행해도 된다는 값 | §5.4 |

## 3. 이미 있는 것과 없는 것

코드에서 확인했다 (2026-11-20, `d1ee3ad0`).

있는 것:

| 부품 | 근거 |
| --- | --- |
| 이슈 참조, 공유본, 필드 해시 | `src/domain/issue_work.rs` (`IssueReference`, `IssueSharedCopy`, `IssueFieldHash`). provider 는 GitHub 과 Jira 이다 |
| 이슈 한 건 읽기 | `issue_snapshot_port` (`resolve`, `read`). GitHub 은 `gh api graphql`, Jira 는 REST v3 이다 |
| 제목과 본문 쓰기, 보드 필드 쓰기 | `issue_update_port`, `issue_board_field_port`, 게시 일지 (`IssueJournaledPublishService`) |
| 규약, 템플릿, 검증, 마스킹 | `issue_policy`, `issue_template`, `validate_issue`, `mask_issue_text`, 코드 검증기 |
| AI 인계용 공유본 | `akra/issue/handoffCopy`, `application/service/issue_handoff` |
| 초안 품질 검토의 모델 계약 | `issue_review_model_port`. 구현은 Fake 하나이다 |
| task 큐 | kernel 명령 16개 (`create_task` 등), task 상태 `awaiting_user` |
| 원자적 task 등록 | `admit_system_task_once` (`application/service/planning/task_mutation.rs`). PR 검증의 보완 task 가 이것을 쓴다 |
| PR 검증의 끝 원인과 복구 행동 | `PrValidationTerminalReason`, `recovery_action` (`domain/parallel_mode/pr_validation.rs`) |
| 격리 실행과 전달 | 병렬 모드: 슬롯 3개, worktree, lease, push → PR 검증 → 통합. 사람 리뷰가 기본이다 |
| 자율 전달의 상한 | `AKRA_PARALLEL_AUTONOMOUS_DELIVERY=1` 은 부모 프로세스에서만 온다. 저장소 설정은 이 권한을 주지 못한다 |
| 사건 기록, 연산 영수증, 멱등 키, 마감 시각 타이머 | plan 19 Q3-1 ~ Q3-7 |
| Desktop 의 이슈 화면 | 업무 패널 (GitHub, Jira, Linear, Projects), 이슈 초안. 읽기와 "스레드로 넘기기"만 한다 |

없는 것:

| 없는 것 | 지금 |
| --- | --- |
| 이슈 목록과 변화 읽기 (Rust) | `issue_snapshot_port` 는 한 건만 읽는다. 목록은 Desktop main 이 읽는다 (`githubService.ts`, `jiraService.ts`, `linearService.ts`) |
| 화면 없이 도는 수집 | Jira 와 Linear 의 토큰은 Electron main 에만 있다 (`jira-token.bin`, `linear-token.bin`). Gateway 는 GitHub 만 `gh` 로 읽는다 |
| 이슈의 수명 상태 | 없다. 이슈는 프롬프트의 재료이다 |
| 이슈의 상태, 라벨, 담당자 읽기 | 없다. snapshot 은 ID, 번호, 제목, 본문, `updatedAt`, 저장소 ID 만 읽는다 |
| 이슈와 task 의 연결 | 없다. 넘기기는 입력창만 채운다 |
| 작업별 실행 허가 | 없다. dispatcher 와 distributor 는 task 의 출처를 모른다. `AKRA_PARALLEL_AUTONOMOUS_DELIVERY=1` 이면 distributor 는 모든 PR 에서 사람의 APPROVED 검사를 건너뛴다 (`pull_request_readiness_error`) |
| 재시작을 넘는 task 1회 생성 | 없다. 멱등 키는 남아 있는 기록 (최근 10,000건) 에서만 기억한다 (`CommandMemory`). 수신 기록과 실제 작업은 따로 쓰인다 |
| 실행 가능 판정 | 없다 |
| 묻기와 답하기 | plan 18 K5 의 남은 일이다. 승인은 턴 안의 app-server 승인만 있다 |
| 댓글, 라벨, 상태 전이 쓰기 | 없다 |
| 완료 기준과 증거 | plan 24 (제안), #2481 의 A01 |
| 주의함 | plan 23 (제안), #2481 의 A02 |

결론: 수집, 판정, 조정, 양방향이 새 일이다. 실행과 전달은 있는 것을 쓴다. 단 실행기를 다시 쓰는 것과, 그 실행기가
이슈별 승인과 보류를 지키는 것은 다른 계약이다. 뒤의 것은 새로 만든다 (§5.4).

## 4. 원칙

1. **이슈 글은 신뢰하지 않는 입력이다.** 글은 지시문이 아니라 자료이다. 기본 실행 profile 은 승인 없음과
   `danger-full-access` 이다. 그래서 "누가 실행을 허락했는가"를 글의 내용보다 먼저 본다 (§14).
2. **결정적 규칙이 먼저이다.** 모델은 애매할 때만 쓴다. 모델은 판정을 더 보수적으로만 바꾼다 (plan 20 §4).
3. **판정은 한 곳에서 한 번 한다.** Desktop, CLI, 에이전트는 같은 명령과 같은 조회를 쓴다 (plan 18 KP-04).
4. **사람은 언제나 개입한다.** 모든 자동 결정은 기록된다. 사람은 그것을 멈출 수 있다. 사람의 답이 중재자의 답을 이긴다.
5. **자율의 상한은 사용자가 정한다.** 저장소의 파일은 상한을 낮출 수만 있다. 코드 검증기의 승인과 같은 규칙이다.
6. **켜는 순서는 관찰 → 제안 → 반자동 → 자동이다.** 기본은 `observe` 이다 (`akra.prValidationMode` 와 같다).
7. **새 실행 경로를 만들지 않는다.** 실행은 planning task 이다. 전달은 병렬 전달이다. 사람 리뷰 기본은 그대로이다.
   기존 실행기는 작업별 허가를 확인하는 자리만 더 가진다 (§5.4).
8. **밖으로 쓰는 글은 공유본이다.** 댓글과 라벨도 승인, 마스킹, 일지를 지난다 (#2414 의 규칙).
9. **원문을 저장하지 않는다.** 장부에는 참조, 해시, 공유본만 둔다.
10. **떼어낼 수 있게 만든다.** 모듈은 port 로만 밖과 만난다 (§5.2).

## 5. 구조

### 5.1 구성 요소

```text
 provider (GitHub, Jira, Linear)
    │ 목록, 변화                              ▲ 댓글, 라벨, 상태 (공유본, 일지)
    ▼                                          │
┌──────────────────── 이슈 자동화 모듈 (저장소당 하나) ─────────────────────┐
│ 수집기 ──▶ 장부 ◀──▶ 조정기 ─┬─ 판정 (규칙 먼저, 모델은 선택)              │
│ (출처 port) (추적 이슈,       ├─ 결정 요청 (질문, 선택지, 마감)             │
│             결정 요청,        ├─ 중재자 (마감 뒤, 정책이 허락할 때)         │
│             행동 일지)        └─ 작업 지시 ──▶ 실행 port                    │
└───────────────────────────────────────────────┬──────────────────────────┘
      ▲ 명령, 조회, 이벤트                        │ 작업 지시 등록, 단계별 허가, 조회 (§5.4)
      │ (kernel spec 의 범위 issue)               ▼
 Desktop · CLI · 에이전트            planning 큐 → 병렬 슬롯 → push → PR 검증 → 통합 (있음)
```

요청의 "worker 의 집합"은 수집기, 조정기, 실행기이다. 수집기와 조정기가 새 코드이다. 실행기는 기존 병렬 작업자이다.

### 5.2 경계 (떼어낼 수 있는 구조)

모듈은 `domain/issue_automation` 과 `application/service/issue_automation` 이다. 모듈은 아래 port 로만 밖과 만난다.

| port | 방향 | 하는 일 | Akra 의 adapter |
| --- | --- | --- | --- |
| `IssueAutomationPort` | inbound | 명령과 조회 | kernel 프로토콜 (범위 `issue`) |
| `IssueSourcePort` | outbound | 발견 (선택 조건, 커서), 재동기화 (안정 ID 로 한 건), 능력 목록 (§13) | GitHub (`gh`). 뒤에 Jira, Linear |
| `IssueFeedbackPort` | outbound | 댓글, 라벨, 담당자, 상태 전이 | provider 마다 하나. 게시 일지의 방식을 다시 쓴다 |
| `IssueWorkExecutionPort` | outbound | 작업 지시의 1회 등록, 단계별 허가의 부여와 철회, 취소, 상태와 끝 원인 읽기 (§5.4, §12.1) | planning authority 의 작업 지시 기록, dispatcher 와 distributor 의 확인 |
| `IssueAutomationStorePort` | outbound | 장부 | SQLite. 저장소 runtime 폴더의 새 파일이다 |
| `IssueAutomationPolicySourcePort` | outbound | 정책 파일 | 파일. `issue_policy` 와 같은 방식이다 |
| `IssueAssessmentModelPort` | outbound | 선택적 모델 판정과 중재자 | 읽기 전용 thread. 처음에는 Fake 이다 |

- 모듈은 planning, parallel, core 의 타입을 import 하지 않는다. architecture test 가 이것을 막는다.
- 떼어낼 때는 모듈과 port 정의를 옮긴다. adapter 는 Akra 에 남는다.
- 다른 실행기(다른 에이전트 런타임, CI)는 `IssueWorkExecutionPort` 의 다른 adapter 이다.
- 지금은 crate 를 나누지 않는다. 이 저장소는 Cargo workspace 가 아니다. 나누는 일은 IA9 이다.
- 이슈의 장부를 planning DB 에 넣지 않는다. planning DB 의 schema version 을 올리면 이전 binary 가 그 DB 를 열지 못한다.
  예외는 §5.4 의 작업 지시 기록이다. 그 기록은 task 와 같은 트랜잭션에 있어야 한다. 표는 schema version 을 올리지 않고
  더한다 (`kernel_events` 표의 방식).

### 5.3 실행 자리

| 질문 | 답 (제안) | 이유 |
| --- | --- | --- |
| 명령은 어디로 들어오는가 | kernel spec 의 새 범위 `issue` | CLI `akra kernel issue … --json`, 생성 skill, Desktop 타입, 멱등 키, 영수증, `caller` 기록을 그대로 얻는다 |
| kernel 이 없을 때의 명령 | 저장만 바꾸는 명령은 `state: store` 이다 | task 명령과 같이 저장소 잠금 아래에서 실행된다 |
| 타이머는 누가 돌리는가 | 런타임 호스트 | KP-02. 수집 주기, 결정 요청의 마감, 재시도는 모두 마감 시각이다 (`next_deadline`) |
| kernel 은 언제 떠 있는가 | `observe`, `propose`: client 가 붙어 있는 동안. `gated`, `auto`: "남은 일"로 센다 | 수동 수준은 저장소 잠금을 계속 쥐지 않는다. 자동 수준은 `has_remaining_work` 에 조건 하나를 더한다 |
| Gateway 를 다시 켠 뒤 | Gateway 가 `gated`, `auto` 인 저장소의 kernel 을 다시 올린다 | 이 장치는 지금 없다 (§21). IA6 에서 한다 |
| 기록은 어디에 쓰는가 | `kernel_events` 에 `issue_*` 종류로 쓴다 | plan 20 §4.1 "사건은 한 곳" |
| 상태의 원본은 무엇인가 | 장부 | 기록은 최근 10,000건만 남는다. 상태를 기록에서 되살리지 않는다 |

- 제약: `gated`, `auto` 인 저장소는 Gateway kernel 이 저장소 잠금을 쥔다. 그 저장소의 TUI 는 시작하지 못한다 (K6 까지).
- 조정기는 planning 과 parallel 의 메모리 상태를 공유하지 않는다. 저장된 값을 읽고 명령만 보낸다. 그래서 plan 19 §3.2 의
  "상태 기계가 둘이다"의 비용(메시지로 맞추기)을 늘리지 않는다.

### 5.4 작업 지시와 실행 허가 (실행기와의 계약)

planning 큐와 병렬 전달은 조정기와 따로 돈다. 그래서 이슈 장부의 값만으로는 실행기를 멈추지 못한다.

- 슬롯을 기다리는 task 의 이슈를 보류해도, dispatcher 는 나중에 그 task 를 가져간다.
- 프로세스에 `AKRA_PARALLEL_AUTONOMOUS_DELIVERY=1` 이 있으면, distributor 는 그 이슈의 관문을 모르고 사람의 APPROVED
  검사를 건너뛴다. 이 값은 다른 작업을 위해 켜져 있을 수 있다.

이 절은 그 간격을 메우는 계약이다. 새 실행기를 만들지 않는다. 기존 dispatcher 와 distributor 가 확인하는 자리를 더한다.

**작업 지시 기록.** planning authority 에 둔다. task 를 만드는 것과 같은 트랜잭션에서 쓴다.

```text
work_order: { work_order_id, origin, task_ids[], permits[] }
  work_order_id  바뀌지 않는다. 이슈, 필드 해시, 시도 번호에서 만든다
  origin         이 지시를 낸 모듈 (예: issue_automation)
  task_ids       최초 task 하나와, PR 검증이 만든 보완 task 들
  permits        단계별 허가
```

- **1회 등록.** adapter 는 `work_order_id` 로 조회하고, 없을 때만 만든다. 조회와 생성은 한 트랜잭션이다.
  `admit_system_task_once` 의 방식을 쓴다. 그 함수를 그대로 부르지 않는다. 그 함수는 task 의 provenance 로 중복을 가리고,
  provenance 는 task 가 고쳐질 때 바뀐다.
- **명령의 멱등 키와 다르다.** kernel 명령의 멱등 키는 영수증의 중복을 막는다. 기록이 정리되면 키를 잊는다. 수신 기록을
  쓴 뒤 task 를 만들기 전에 죽으면, 재전송은 영수증만 다시 받는다. "task 하나"는 작업 지시 기록이 보장한다.
- **계보.** 최초 task 와 보완 task 는 한 작업 지시에 속한다. 보류, 취소, 예산은 계보 전체에 든다.

**허가.** 단계는 `dispatch`, `push`, `pull_request`, `integrate` 이다.

```text
permit: { work_order_id, stage, policy_version, field_hash, gate_revision, source_sha }
  gate_revision  그 관문의 판 번호 (§9.2)
  source_sha     push, pull_request, integrate 에만 있다. 고정한 소스 커밋이다
```

- **유효 권한 = 프로세스 상한 ∩ 바인딩과 이슈의 정책 ∩ 현재 관문의 승인.** 프로세스의 opt-in 은 상한이다. 그것만으로는
  이슈 작업을 통합하지 못한다.
- **확인하는 자리.** 슬롯 배분 (claim 과 같은 트랜잭션), 소스 push 직전, PR 작성 직전, 통합 직전. 허가가 없거나 묶인 값이
  지금 값과 다르면 그 단계를 하지 않고 이유를 남긴다. 다른 작업은 계속한다.
- **기존 흐름.** 작업 지시 기록이 없는 task (사람이 만든 task) 는 지금과 똑같이 돈다.
- **경계.** dispatcher 와 distributor 는 planning authority 의 이 기록만 읽는다. 이슈 자동화 모듈을 import 하지 않는다.

**철회와 인계.**

| 명령이나 변화 | 허가에 주는 영향 |
| --- | --- |
| `hold`, `stale`, `cancel` | 그 작업 지시의 아직 쓰지 않은 허가를 모두 철회한다. 실행 중인 턴을 끊는 것은 따로 고른다 |
| `release` | 허가를 되살리지 않는다. 관문을 다시 지나야 새 허가가 나온다 |
| `take_over` | 자동 후속 실행, 전달, 정리의 허가를 모두 철회한다. thread 는 사람이 이어 쓴다. 호스트는 브랜치와 worktree 를 지우지 않는다. 슬롯의 lease 는 사람이 끝낼 때까지 남는다 (§19 Q-IA14) |

이 계약은 IA3 의 범위이다. `propose` 에서도 통합의 허가는 사람의 승인에서만 나온다.

## 6. 추적 이슈의 상태와 맞추기

상태는 닫힌 목록이다.

| 상태 | 뜻 | 나가는 길 |
| --- | --- | --- |
| `discovered` | 수집기가 봤다. 아직 판정하지 않았다 | 판정 |
| `ineligible` | 선택 조건이나 신뢰 조건에 맞지 않는다 | 이슈가 바뀌면 다시 판정한다 |
| `needs_info` | 실행 계약이 부족하다 (예: 완료 기준이 없다) | 이슈가 바뀌면 다시 판정한다 |
| `awaiting_decision` | 열린 결정 요청이 있다 | 답, 마감 |
| `ready` | 시작해도 된다. 예산과 자리를 기다린다 | `queued` |
| `queued` | 작업 지시를 냈다. task 가 큐에 있다 | `running`, `cancelled` |
| `running` | 실행기가 일한다 | `in_review`, `awaiting_decision`, `failed` |
| `in_review` | PR 이 열려 있다. CI, 리뷰, 통합을 기다린다 | `done`, `running` (수정), `failed` |
| `done` | 신뢰된 통합 증거가 확인됐다. 통합 후 검증·복구와 외부 이슈의 닫힘은 따로 기록한다 (§12.1, §12.2) | 끝 |
| `failed` | 구현 시도 한도를 소진했다 | 사람의 결정. 뒤늦게 신뢰된 통합을 확인하면 `done` |
| `cancelled` | 이슈가 닫혔거나 사람이 취소했다 | 끝 |

상태와 따로 표시 두 개가 있다.

- `held`: 사람이나 규칙이 멈췄다. 보류 중에는 조정기가 새 행동을 내지 않는다. 실행기도 멈춘다: 허가를 철회한다 (§5.4).
- `stale`: 판정한 뒤에 이슈의 제목이나 본문이 바뀌었다.
- `unknown`: 이슈를 다시 읽지 못했다 (403, 404, 조회 실패). 닫힘으로도, 변화 없음으로도 보지 않는다.

맞추기:

- `next_action(추적 이슈, 정책, 실행 상태, 시각) → 행동` 은 domain 의 순수 함수이다.
- 행동은 닫힌 목록이다: `triage`, `open_decision`, `expire_decision`, `submit_work`, `cancel_work`, `post_feedback`,
  `mark_done`, `none`.
- 입력은 저장된 값이다: 장부, provider 의 최신 스냅숏, task 의 상태, PR 검증 상태. 사건 하나를 놓쳐도 다음 주기에 같은
  답이 나온다. 재시작해도 이어진다 (plan 19 P1).
- 이슈 하나에 진행 중인 행동은 하나이다. 밖으로 쓰는 행동은 먼저 행동 일지에 적는다. 일지의 상태는 게시 일지와 같다:
  `recorded` → `sent` → `confirmed` 또는 `needs_confirmation`.
- 결과를 모르는 행동 (`needs_confirmation`) 은 다시 보내지 않는다. 조정기는 읽기만 해서 확인한다. 확인할 때까지 그
  이슈의 새 쓰기를 막는다 (§12).
- 제출과 시작의 직전에 이슈를 다시 읽고 자격을 다시 본다 (§13).

밖의 변화와 대응:

| 밖의 변화 | 대응 |
| --- | --- |
| 판정 뒤에 제목이나 본문이 바뀜 (필드 해시가 다름) | `stale`. 시작 전이면 다시 판정한다. 실행 중이면 결정 요청을 연다 (계속 / 다시 시작 / 멈춤) |
| 이슈가 닫힘 | 시작 전이면 `cancelled`. 실행 중이면 결정 요청을 연다 |
| 선택 라벨이 없어짐 | 다시 판정한다. 시작 전이면 `ineligible` 이고 허가를 철회한다. 실행 중이면 결정 요청을 연다 |
| 이슈를 읽지 못함 (403, 404, 조회 실패) | `unknown`. 새로 시작하지 않고 전달하지 않는다. 다음 주기에 다시 읽는다 |
| 다른 사람이 담당자가 됨, 다른 PR 이 연결됨 | `held`, 이유 `already_claimed` |
| 보류 라벨이 붙음 | `held` |
| task 가 `awaiting_user` 가 됨 | 결정 요청으로 올린다 (§10.2) |
| PR 검증이 `Blocked` 나 `Failed` 가 됨 | 끝의 원인에 따라 다르다. 모두 다시 구현하는 것이 아니다 (§12.1) |

닫힌 이슈와 선택 라벨을 잃은 이슈는 발견의 결과에서 사라진다. 그래서 위의 변화는 재동기화가 읽는다 (§13).

## 7. 체계: 실행 계약, 정책, 플레이북

### 7.1 세 층

요청의 질문은 "skill 인가, plugin 인가, 하드코딩인가"이다. 답은 층마다 다르다.

| 층 | 형태 | 정하는 쪽 | 호스트가 강제하는가 |
| --- | --- | --- | --- |
| 상태, 관문, 행동, 이유 코드 | 코드의 닫힌 목록 | Akra | 예 |
| 자동화 정책 | 설정 파일 | 사용자, 저장소 | 예 |
| 실행 계약 | 이슈의 라벨과 본문 섹션 | 이슈를 쓰는 사람 | 예. 판정의 입력이다 |
| 플레이북 | skill 문서 | Akra 기본, 저장소 | 아니요. 모델에게 주는 안내이다 |
| provider 차이 | port 의 adapter | Akra | - |

- 권한은 정책에서만 온다. skill 과 이슈 글은 권한을 주지 못한다.
- 동적 plugin 은 만들지 않는다. 확장하는 자리는 port 이다.

### 7.2 실행 계약 (이슈 작성에 주는 영향)

- **라벨.** 모든 provider 에서 통한다. 기본 이름은 `akra:auto`, `akra:gated`, `akra:manual`, `akra:hold` 이다. 라벨은 그
  이슈의 자율 수준을 요청하거나 낮춘다. 정책의 상한을 넘지 못한다. 이름은 정책에서 바꾼다.
- **본문 섹션.** 기계가 읽는다. `완료 기준` 은 필수이고 plan 24 의 `acceptance` 로 바뀐다. `범위` 는 경로나 구성
  요소이다. `하지 않는 것` 은 선택이다. 섹션 검사는 기존 `validate_issue` 와 `required_sections` 를 쓴다.
- **이슈 양식.** Akra 가 양식을 준다: GitHub issue form, Jira 와 Linear 의 설명 템플릿. 설치는 사용자가 시작하는
  명령이고 PR 로 들어간다.
- **준비 검사.** 이슈 초안 화면 (#2426) 이 부족한 섹션, 예상 위험 등급, 예상 관문을 보인다. 판정과 같은 함수를 쓴다.
  사용자는 이슈를 올리기 전에 결과를 안다.
- **에이전트가 만드는 이슈.** 후속 작업과 큰 이슈의 분해는 새 이슈 게시 경로 (#2436) 와 사람의 승인을 지난다. 그 이슈는
  사람이 라벨을 붙이기 전에는 `ineligible` 이다. 이것이 자기 증식을 막는다.

### 7.3 자동화 정책 파일

- 자리: 사용자 파일 `$AKRA_HOME/issue-automation.toml`, 저장소 파일 `.akra/issue-automation.toml`.
- `issue-policy.toml` 과 따로 둔다. 한 파일이 깨졌을 때 다른 기능이 막히지 않는다.
- 값마다 저장소 > 사용자 > 내장 순서로 정한다. 예외가 있다: 자율 상한, 이슈에 쓰는 허락, 예산의 최대값은 사용자
  파일에서만 온다. 저장소 파일은 낮출 수만 있다.
- 저장소 파일은 신뢰와 보호도 풀지 못한다. 신뢰하는 행위자는 사용자 파일에서만 온다. 저장소 파일은 그 목록을 줄일
  수만 있다. 보호 경로와 사람 필수 규칙은 내장 값과 사용자 파일이 정한다. 저장소 파일은 더할 수만 있다.
- 깨진 파일은 `policy_blocked` 이다. 내장 값으로 대신하지 않는다 (`IssuePolicyBlocked` 와 같다).
- 판정과 결정 요청에 정책 판(내용의 SHA-256)을 적는다. 정책이 바뀌면 열린 판정은 낡은 것이다.

아래는 모양의 예이다. 키 이름은 IA2 에서 정한다.

```toml
schema_version = 1

[binding.github]
repository = "owner/name"          # 없으면 origin
mode = "observe"                   # off | observe | propose | gated | auto

[select]                           # 추적할 이슈
labels_any = ["akra:auto", "akra:gated", "akra:manual"]
trusted_actors = ["maintainer-login"]   # 이슈를 썼거나 라벨을 붙인 사람. 사용자 파일에서만 온다

[[classify.rule]]                  # 위험 등급. 위에서부터 본다
class = "high"
labels_any = ["security", "migration"]
paths_any = [".github/**", "scripts/release/**"]

[[classify.rule]]
class = "low"
labels_any = ["docs", "chore"]

[gates.gated]                      # human | timeout_default | timeout_arbiter | auto
low    = { start = "auto",  delivery = "human", merge = "human" }
normal = { start = "timeout_arbiter", delivery = "human", merge = "human" }
high   = { start = "human", plan = "human", delivery = "human", merge = "human" }

[timeouts]
start = "24h"
remind_before = "2h"

[budget]
max_running = 2
max_started_per_day = 5
max_attempts = 3

[feedback]                         # 이슈에 쓰는 것. 기본은 모두 꺼짐
comment = false
labels = false
transition = false                 # false 이면 Akra 는 이슈를 닫지도 전이하지도 않는다

[close]                            # 통합 뒤의 외부 이슈 (§12.2)
when = "never"                     # never | integration | provider_link
```

### 7.4 플레이북 (skill)

- Akra 가 기본 skill 을 준다: 판정 기준, 이슈 종류별 구현 절차, 중재자 기준. `akra-kernel` skill 과 같은 길
  (`skills/extraRoots/set`)로 등록한다.
- 저장소는 같은 이름의 skill 로 바꾼다. Desktop 의 스킬 패널이 출처를 관리한다 (#2383).
- 실행 thread 의 첫 지시는 호스트가 만든다: 공유본, 완료 기준, 범위, 브랜치 규칙. skill 은 "어떻게"만 말한다.

## 8. 판정 (triage)

| 단계 | 보는 것 | 결과 | 모델 |
| --- | --- | --- | --- |
| 1. 자격 | 선택 조건, 열림, 신뢰하는 행위자, 담당자, 이미 연결된 PR | `ineligible` 과 이유 | 없음 |
| 2. 준비 | 필수 섹션, 완료 기준이 목록으로 읽히는가, 크기 한도, 지원하지 않는 내용, 마스킹 결과 | `needs_info` 와 부족한 것 | 없음 |
| 3. 위험 | 정책의 등급 규칙 (라벨, 언급한 경로, 이슈 종류) | `low`, `normal`, `high`. 맞는 규칙이 없으면 `normal`, 읽지 못하면 `high` | 없음 |
| 4. 모델 판정 (선택, 기본 꺼짐) | 명확성, 범위의 크기, 빠진 기준의 초안 | 등급을 올리거나 `needs_info` 로 내린다. 등급을 낮추지 못한다 | 읽기 전용 thread |
| 5. 실행 뒤 | 실제 diff: 바뀐 경로, 줄 수, 보호 경로 | 관문을 `human` 으로 올린다 | 없음 |

- 판정 결과는 `{ eligibility, readiness, risk_class, gates[], reasons[], policy_version, field_hash }` 이다.
- 이유는 닫힌 목록이다. 이유마다 `summary` 와 `next` 가 있다 (kernel spec 의 `reasons` 와 같은 형식).
- 판정은 필드 해시에 묶인다. 사람이 승인한 글과 에이전트가 받는 글은 같은 해시이다.
- 모델이 쓴 완료 기준은 제안이다. 사람이 받아들여야 기준이 된다 (plan 24 §5).
- 5단계가 필요한 이유: 이슈의 글은 범위를 작게 말할 수 있다. 호스트는 이미 바뀐 파일 범위를 확인한다 (병렬 계약).
  그 자리에서 등급을 다시 본다.

## 9. 자율 수준과 관문

### 9.1 자율 수준

바인딩마다 값이 하나 있다. 이슈의 라벨은 그 이슈의 값을 낮춘다.

| 수준 | 수집과 판정 | task | 시작 | 전달과 통합 | 이슈에 쓰기 |
| --- | --- | --- | --- | --- | --- |
| `off` | 안 한다 | - | - | - | - |
| `observe` | 한다 | 안 만든다. "이렇게 했을 것이다"만 보인다 | - | - | 안 한다 |
| `propose` | 한다 | 사람이 시작을 승인한 뒤에 만든다 | 사람. 마감이 없다 | 사람 리뷰 | 정책이 켠 것만 |
| `gated` (반자동) | 한다 | 관문을 지나면 만든다 | 등급별 관문 | 사람 리뷰 | 정책이 켠 것만 |
| `auto` (풀자동) | 한다 | 자동 | 자동. 사람 필수 규칙은 예외이다 | 정책과 프로세스 opt-in | 정책이 켠 것만 |

- 수준을 올리는 것은 사람의 명령이다. Desktop main 이 확인 창을 띄운다.
- `auto` 의 자동 통합은 둘을 모두 요구한다: 지금의 `AKRA_PARALLEL_AUTONOMOUS_DELIVERY=1` 과 그 작업의 `integrate` 허가.
  프로세스의 opt-in 만으로는 통합하지 않는다 (§5.4). 새 길을 열지 않는다.
- `propose` 와 `gated` 의 "사람 리뷰"는 표시가 아니다. 실행기가 허가로 강제한다 (§5.4).
- 공개 저장소의 기본 상한은 `propose` 이다 (§19 Q-IA3).

### 9.2 관문

| 관문 | 언제 | 통과시키는 쪽 |
| --- | --- | --- |
| `start` | 실행 전 | 사람, 기본값, 중재자, 자동 |
| `spec` | 완료 기준을 확정할 때 | 사람만 |
| `plan` | 구현 계획을 낸 뒤, 파일을 고치기 전 | 사람, 중재자 |
| `delivery` | push 와 PR | 지금의 병렬 전달 규칙 |
| `merge` | 통합 | 사람 리뷰. opt-in 일 때만 자동이다 |
| `close` | 이슈 닫기, 상태 전이 | 정책 `close.when`. 기본은 닫지 않는다 (§12.2) |

관문의 처리 방식은 넷이다: `human` (끝까지 기다린다), `timeout_default` (마감 뒤에 권고값), `timeout_arbiter` (마감 뒤에
중재자), `auto`.

관문마다 판 번호 (`gate_revision`) 가 있다. 정책 판, 이슈의 필드 해시, 위험 등급, 고정한 소스 커밋 중 하나가 바뀌면 판이
오른다. 이전 판의 승인과 허가는 쓰지 못한다.

### 9.3 사람 필수 규칙 (풀자동의 예외)

아래 중 하나이면 그 관문은 `human` 이다. 마감이 지나도 넘어가지 않는다. 저장소 파일은 이 목록을 줄이지 못한다 (§7.3).

- 위험 등급이 `high` 이다. 또는 보류 라벨이나 수동 라벨이 있다.
- 작성자를 신뢰하지 않고, 신뢰하는 행위자의 라벨도 없다.
- diff 가 보호 경로를 건드렸다: CI workflow, 릴리스 스크립트, 자격 증명, 정책 파일, `AGENTS.md`.
- diff 가 크기 한도를 넘었다.
- 완료 기준이 없다. 또는 기계가 확인하지 못하는 기준(`screen`)만 있다.
- 같은 이슈가 시도 한도까지 실패했다.
- 마스킹이 `user_confirm` 구간을 만났다.
- 예산을 다 썼다.
- 중재자가 확신하지 못했다.

## 10. 사람과 AI 의 양방향

### 10.1 결정 요청

```text
decision: { id, issue, gate, kind, question, options[{ id, label, effect }], recommended,
            basis: { policy_version, field_hash, work_order_id, gate_revision },
            on_timeout: wait | default | arbiter, due_at, asked_by, evidence[],
            state: open | answered | expired | withdrawn | superseded,
            answer: { option, note, by, confirmation, caller, channel, at } }
```

- `basis` 는 이 질문이 본 판이다. 답을 저장할 때 지금의 판과 비교한다. 다르면 답을 적용하지 않고 요청을 `superseded` 로
  닫는다. 필요하면 새 요청을 연다. 이 검사가 오래 열려 있던 Desktop 카드의 답, 늦게 끝난 중재자의 답, 바뀐 이슈에 대한
  답을 거른다.
- `confirmation` 은 사람 확인의 증거이다 (§14). `caller` 는 표시용 이름이고 증거가 아니다. 둘을 섞지 않는다.

- `kind` 는 닫힌 목록이다: 시작 승인, 기준 확정, 계획 승인, 위험 예외, 이슈가 바뀜, 실패 뒤 처리, 에이전트의 질문.
- 이 객체는 plan 18 §4.3 의 "묻기·답하기"와 "승인을 독립 객체로"의 첫 구현이다. 대상(subject)을 가진 일반 객체로
  만든다. 첫 대상이 이슈이다.
- 열린 결정 요청은 plan 23 의 "내 차례"이다. 주의 항목의 대상 종류에 `issue` 를 더한다.
- 결정 요청 하나는 답 하나만 받는다. 먼저 기록된 답이 이긴다.

### 10.2 방향과 수단

| 방향 | 예 | 수단 |
| --- | --- | --- |
| Akra → 사람 | "이슈 #12 를 시작할까요? 내일 09:00 에 중재자가 결정합니다" | Desktop 의 이슈 탭과 주의함, OS 알림. 정책이 켜면 이슈 댓글과 Telegram |
| 사람 → Akra | 승인, 거절, 보류, 풀기, 다시 판정, 지시 추가, 내가 맡기 | Desktop. 나중에 신뢰하는 사람의 이슈 댓글 명령 |
| 실행 중인 에이전트 → 사람 | "A 와 B 중 무엇입니까?" | task 의 `awaiting_user`, 또는 에이전트가 `akra kernel issue ask_issue_decision … --json` 을 실행한다 |
| 사람 → 실행 중인 에이전트 | 답, 방향 수정 | 조정기가 답을 그 thread 에 `submit_prompt` 나 `steer_turn` 으로 전달한다 |
| 에이전트 → Akra | 이슈 상태 조회, 후속 이슈 제안 | CLI `--json` 과 skill (KP-05) |

개입 명령:

| 명령 | 하는 일 |
| --- | --- |
| `hold` / `release` | 새 행동을 멈추고 다시 연다. `hold` 는 허가를 철회한다. `release` 뒤에는 관문을 다시 지난다. 실행 중인 턴을 끊는 것은 따로 고른다 (`stop_all`) |
| `steer` | 지시를 더한다. 실행 전이면 작업 지시에 붙고, 실행 중이면 그 thread 에 간다 |
| `take_over` | 자동화가 손을 뗀다. 자동 후속 실행, 전달, 정리의 허가를 철회한다. 브랜치, worktree, thread 는 남는다. 사람이 그 thread 를 이어 쓴다 |
| `retriage` | 다시 판정한다 |
| `cancel` | 작업 지시의 계보 전체를 취소하고 허가를 철회한다. 확인이 필요하다 |

허가의 철회와 인계의 계약은 §5.4 에 있다.

### 10.3 마감과 중재자

- 열린 결정 요청은 `due_at` 을 가진다. 마감 전에 한 번 다시 알린다.
- 마감이 지나면 `on_timeout` 을 따른다.
  - `wait`: 계속 기다린다. 사람 필수 규칙의 관문은 항상 이것이다.
  - `default`: 권고값을 고른다. 모델을 부르지 않는다.
  - `arbiter`: 중재자가 결정한다.
- 중재자는 읽기 전용 검토자이다. 입력은 공유본, 판정, 정책, 저장소(읽기)이다. 출력은 구조화된 값이다:
  `{ decision, confidence, reasons[], blockers[] }`.
- 중재자의 한계는 코드가 강제한다.
  - 사람 필수 규칙을 넘지 못한다.
  - `spec` 과 `merge` 관문을 통과시키지 못한다.
  - 자율 수준을 올리지 못한다.
  - 확신이 기준보다 낮거나 blocker 가 있으면 요청을 `wait` 로 남기고 다시 알린다.
- 중재자의 결정은 `by: arbiter` 로 기록되고 바로 알린다. 사람은 뒤에 보류나 취소로 뒤집는다.
- 중재자가 시작시킨 일도 전달과 통합의 관문은 그대로 지난다.

## 11. 사용 장면

1. **반자동, 사람이 답한다.** 이슈가 등록된다 → 수집기가 5분 안에 읽는다 → 판정: 준비됨, 등급 `normal` → 결정 요청
   "시작할까요?" → 사람이 [시작] → task → 슬롯 → PR → 사람 리뷰 → 통합 → 상태 댓글 → `done`.
2. **반자동, 사람이 답하지 않는다.** 같은 흐름에서 24시간이 지난다 → 정책이 `timeout_arbiter` 이다 → 중재자: 시작, 확신
   높음 → `by: arbiter` 기록과 알림 → 실행 → PR 은 사람 리뷰를 기다린다.
3. **풀자동의 예외.** 수준 `auto`, 등급 `low` → 자동 시작 → diff 가 `.github/workflows/` 를 건드린다 → `delivery` 관문이
   `human` 이 된다 → "내 차례"에 이유 `protected_path` 와 함께 나온다.
4. **실행 중의 질문.** 에이전트가 묻는다 → task 가 `awaiting_user` → 결정 요청 → 사람이 답한다 → 답이 그 thread 에
   간다 → 계속한다.
5. **이슈가 바뀐다.** 실행 중에 본문이 바뀐다 → `stale` → 결정 요청 (계속 / 다시 시작 / 멈춤).
6. **사람이 맡는다.** 사람이 [내가 맡기] → 자동화가 손을 뗀다 → 사람이 같은 thread 와 브랜치로 계속한다.

## 12. 실행과 결과 반환

1. **작업 지시.** `{ work_order_id, issue, field_hash, title, handoff, acceptance[], scope, branch_hint,
   link_reference, profile, priority, attempt }`. `handoff` 는 AI 인계 정책을 지난 공유본이다.
2. **task.** Akra 의 adapter 는 작업 지시를 한 번 등록한다 (§5.4). 등록은 최초 task 를 만들고 작업 지시 기록을 같은
   트랜잭션에 쓴다. `create_task` 의 멱등 키에 기대지 않는다. 재시작, 응답 유실, 기록의 정리 뒤에도 같은
   `work_order_id` 는 같은 task 를 돌려준다.
3. **실행.** 병렬 모드가 켜져 있고 `dispatch` 허가가 있으면 슬롯이 task 를 가져간다. 병렬 모드가 꺼져 있으면 task 는 큐에
   남고 화면이 이유를 보인다. 이슈 자동화는 병렬 모드를 스스로 켜지 않는다 (§19 Q-IA8).
4. **보면서 하기.** "내 대화로 열기"는 확인된 현재 thread로 이동하거나 공유본 초안을 연다. `take_over`와 턴 전송은 별도 행동이며, 대화를 여는 것만으로 실행하지 않는다.
5. **전달.** 지금의 순서 그대로이다. push, PR 작성, 통합은 각각 그 단계의 허가를 확인한다 (§5.4). 더하는 것은 PR 의
   참조이다: 이슈 주소, Jira 키, Linear 의 브랜치 이름. 지금의 PR 본문은 고정 양식이고 참조가 없다
   (`build_distributor_pull_request_body`, `src/application/service/parallel_mode/distributor/delivery/github.rs`).
   닫는 키워드 (`Closes #N`) 는 이슈를 닫는 수단으로 믿지 않는다 (§12.2).
6. **연결.** 장부가 `이슈 → 작업 지시` 를 가진다. 작업 지시 기록이 `task 계보` 를 가진다. 장부는 thread, 브랜치, PR 도
   적는다. `TaskDefinition` 은 바꾸지 않는다. 자료 연결은 #2481 의 A03 이 맡는다.
7. **결과 반환.** 상태 댓글 하나를 만들고 고친다. 댓글을 쌓지 않는다. 라벨과 상태 전이는 정책이 켠 것만 한다.
   - 쓰기는 행동 일지를 지난다. 일지는 의도 ID, 만든 댓글의 ID, 쓴 계정을 적는다.
   - 숨은 표식만으로는 중복을 막지 못한다. 응답을 잃은 쓰기는 `needs_confirmation` 이다. 조정기는 다시 읽어서 댓글 ID 와
     쓴 계정과 의도 ID 를 확인한다. 확인하지 못하면 자동으로 다시 쓰지 않는다. 사람에게 알린다.
8. **완료 기준과 증거.** plan 24 가 맡는다. 증거 게이트 (E3) 가 없는 동안 전달과 통합은 모든 위험 등급에서 사람
   리뷰이다. 자동 전달 (`auto`, IA6) 은 E3 의 enforce 를 요구한다.
9. **예산.** 동시에 도는 이슈 수, 하루에 시작하는 수, 이슈당 실행 횟수 (§12.1). 사용 한도 (`account/rateLimits/*`) 가
   기준을 넘으면 새로 시작하지 않는다 (#2481 의 R01).

### 12.1 실행의 끝과 다시 하기

IA4-1의 실행 추적 계약은 [현재 제품 reference](../reference/current-product.md#issue-automation-execution)
([한국어](../ko/reference/current-product.md#이슈-자동화-실행))에 둔다. 구현 PR #2638의 병합 상태와
IA4의 남은 조각은 §17에 기록한다.

실행 port의 중립 종료 분류는 `rework_needed`, `observe_again`, `contract_recovery`,
`identity_conflict`, `pull_request_closed`, `queue_recovery`이다. 원인별 대응, 실행·검증 횟수,
같은 task의 질문 답변 재개, 통합 후 `done` 보존은 reference의 계약을 따른다. 기존 PR 검증 FSM과
원본 종료 기록은 유지하며, 보완 task 생성과 작업 지시 계보 등록은 같은 admission 트랜잭션에서 처리한다.

IA4-2 (#2552)는 전송권을 얻은 피드백 의도마다 게시 횟수를 한 번 센다. 미확정 의도는
`feedback_unconfirmed`로 표시하고 읽기로만 확인한다. 이 횟수는 실행 시도 한도와 독립적이다.
댓글이나 라벨 쓰기의 결과를 다시 읽어 확인하고, 결과를 확정하기 전에는 자동으로 다시 쓰지 않는 계약을
해당 조각에서 구현·검증한다.

### 12.2 통합과 외부 이슈의 닫힘

통합의 완료와 provider 이슈의 닫힘은 다른 사실이다. 장부는 둘을 따로 적는다.

- GitHub 은 PR 의 base 가 저장소의 기본 브랜치일 때만 닫는 키워드를 따른다
  ([GitHub 문서](https://docs.github.com/en/issues/tracking-your-work-with-issues/using-issues/linking-a-pull-request-to-an-issue)).
  다른 브랜치로 가는 PR 에서는 키워드를 무시하고, 연결도 만들지 않고, 병합해도 이슈를 닫지 않는다.
- 이 저장소의 기본 브랜치는 `main` 이다. 병렬 통합 브랜치의 기본값은 `prerelease` 이다. distributor 는 PR 의 base 가 통합
  브랜치인지 확인한다. 그래서 `Closes owner/name#N` 은 이 전달 경로에서 이슈를 닫지 않는다.
- `done` 은 외부 닫힘을 요구하지 않는다. 화면은 "통합함"과 "외부 이슈가 열려 있음"을 따로 보인다.
- 닫기 정책의 실행은 IA4-3 (#2553)의 범위이다. IA4-1은 통합 사실을 기록하며 외부 이슈를 닫지 않는다.
- 아래 `close.when` 동작은 IA4-3 구현 계약이다. [현재 동작](../reference/current-product.md#issue-integration-and-external-closure)을 따른다.

| `close.when` | 뜻 | 조건 |
| --- | --- | --- |
| `never` (기본) | Akra 는 이슈를 닫지 않는다. 상태 댓글이 켜져 있으면 통합을 알린다. 닫기는 사람이 한다 | 없음 |
| `integration` | 통합과 검증이 끝나면 Akra 가 이슈를 닫거나 전이한다 | `feedback.transition = true` 와 `close` 관문의 승인 |
| `provider_link` | provider 의 PR 연결에 맡긴다 | PR 의 base 가 기본 브랜치이다 |

- `feedback.transition = false` 이면 Akra 는 이슈를 닫지도 전이하지도 않는다. 이때 `close.when = integration` 은 정책
  오류이다 (`policy_blocked`).
- 통합 브랜치가 기본 브랜치로 올라가는 것 (`prerelease` → `main`) 을 Akra 는 관찰하지 않는다. 그때 닫으려면 `never` 로
  두고 사람이 닫는다 (§19 Q-IA13).

## 13. 수집 (provider 와 자격 증명)

| provider | 읽기 | 쓰기 | 자격 증명 | 순서 |
| --- | --- | --- | --- | --- |
| GitHub Issues | `gh api graphql`. 발견: 선택 조건의 열린 이슈, `updatedAt` 커서. 재동기화: node ID 로 한 건 | 댓글, 라벨, 담당자. 닫기는 §12.2 | `gh` 로그인. Gateway 에서 된다 | 첫 번째 |
| GitHub Projects | 보드의 열과 필드 | `issue_board_field_port` (있음) | `read:project` | 뒤 |
| Jira Cloud | JQL, `updated` 커서 | 댓글, 전이 | Electron main 에만 있다. 결정이 필요하다 | 뒤 |
| Linear | GraphQL, `updatedAt` | 댓글, 상태 | Electron main 에만 있다. 결정이 필요하다 | 뒤 |

- 수집은 폴링이다. 기본 주기는 5분이다. "지금 동기화" 명령이 있다. 실패하면 간격을 늘린다.
- webhook 은 받지 않는다. 로컬 데몬은 공개 주소가 없다.
- 읽기는 둘이다.
  - **발견.** 선택 조건에 맞는 열린 이슈를 `updatedAt` 커서로 읽는다. 새 이슈를 찾는 일만 한다.
  - **재동기화.** 끝나지 않은 추적 이슈를 안정 ID 로 다시 읽는다. 선택 조건과 무관하다. 읽는 값: 열림과 닫힘, 라벨,
    담당자, 신뢰의 근거 (누가 썼는가, 누가 라벨을 붙였는가), 필드 해시.
- 둘을 나누는 이유: 닫힌 이슈와 선택 라벨을 잃은 이슈는 발견의 결과에서 사라진다. 사라진 것은 변화로 보이지 않는다.
  발견만 하면 장부에 예전의 `ready` 나 `queued` 가 남는다.
- 지금의 `issue_snapshot_port` 는 상태, 라벨, 담당자, 신뢰의 근거를 읽지 않는다. 재동기화의 계약은 새로 만든다 (IA1).
- 403, 404, 조회 실패는 `unknown` 이다. 닫힘으로도, 변화 없음으로도 보지 않는다. `unknown` 인 이슈는 시작하지 않고
  전달하지 않는다.
- 제출과 시작의 직전에 재동기화를 한 번 더 한다. 낡은 자격으로 실행하지 않는다.
- 커서는 그 쪽 (page) 의 처리와 장부 저장이 끝난 뒤에 나아간다. 중간에 죽으면 같은 쪽을 다시 읽는다. 장부 쓰기는
  멱등이다.
- 출처는 능력 목록을 낸다: `{ incremental, comments, labels, assign, transition, link_pull_request, board_fields }`.
  조정기는 없는 능력의 행동을 고르지 않는다.
- 바인딩: GitHub 은 저장소의 `origin` 이 기본이다. Jira 프로젝트와 Linear 팀은 저장소가 없다. 사용자가 그것을 저장소에
  연결한다 (plan 17 R12 의 "팀과 로컬 폴더"). 이슈 하나는 바인딩 하나에만 속한다.
- 댓글은 기본으로 읽지 않는다. 읽을 때는 신뢰하는 작성자의 댓글만 읽는다.
- Jira 와 Linear 의 자격 증명은 세 가지 선택이 있다.

| 선택 | 내용 | 한계 |
| --- | --- | --- |
| (a) Gateway 의 OS 키 저장소 | `keyring` crate 가 이미 있다 (Vault 키). Desktop 은 Gateway 를 지나 토큰을 저장한다 | 토큰의 주인이 Desktop 에서 Gateway 로 바뀐다 |
| (b) 연결된 Desktop 에 되묻기 | `akra/resourceVersion/current` 의 방식 | 앱이 꺼지면 수집이 멈춘다 |
| (c) MCP hub 의 로그인 | Notion 버전 확인의 방식 | 서버마다 도구 모양이 다르다. 쓰기의 일지를 직접 다루기 어렵다 |

제안은 (a) 이다 (§19 Q-IA2).

## 14. 안전

| 위협 | 대응 |
| --- | --- |
| 이슈 글 안의 지시문. 공개 저장소에는 누구나 이슈를 쓴다 | 자격 단계가 글보다 먼저이다. 신뢰하는 행위자가 쓴 이슈이거나, 그 사람이 붙인 라벨이 있어야 한다. 글은 자료 구역으로 감싸서 넘긴다. 댓글은 기본으로 넣지 않는다 |
| 승인한 글과 실행한 글이 다르다 | 필드 해시로 고정한다. 바뀌면 `stale` 이다 |
| 저장소의 파일이 권한을 올린다 | 자율 상한, 쓰기 허락, 예산 최대, 신뢰하는 행위자는 사용자 파일과 프로세스 환경에서만 온다. 저장소 파일은 보호 경로와 사람 필수 규칙을 줄이지 못한다 (§7.3) |
| 프로세스의 자율 전달 opt-in 이 이슈의 관문을 건너뛴다 | 유효 권한은 프로세스 상한 ∩ 정책 ∩ 관문의 승인이다. distributor 는 통합 직전에 그 작업의 허가를 확인한다 (§5.4) |
| 보류하거나 낡은 이슈의 task 를 실행기가 가져간다 | 허가의 철회. dispatcher 는 슬롯 배분의 트랜잭션에서 허가를 확인한다 (§5.4) |
| 닫혔거나 라벨을 잃은 이슈를 낡은 자격으로 실행한다 | 재동기화. 조회 실패는 `unknown` 이다. 제출과 시작의 직전에 다시 읽는다 (§13) |
| 오래된 답이 새 상황에 적용된다 | 결정 요청의 `basis`. 판이 다르면 `superseded` 이다 (§10.1) |
| 에이전트가 자기 관문을 스스로 통과시킨다 | kernel 의 `caller` 는 인증이 아니다. 그래서 `human` 관문의 답은 사람 확인 경로에서만 받는다: Desktop main 의 확인 창이 만든 값 (`confirmation`). CLI 로 온 답은 `by: agent` 로 기록하고 `human` 관문을 통과시키지 못한다 (§19 Q-IA11) |
| 비밀 값이 이슈나 댓글로 나간다 | 밖으로 쓰는 글은 `mask_issue_text` 를 지난 공유본뿐이다. 에이전트의 글을 그대로 올리지 않는다 |
| 중복 실행 | 작업 지시의 1회 등록 (§5.4). 명령의 멱등 키에 기대지 않는다 |
| 중복 댓글 | 행동 일지, 상태 댓글 하나, 댓글 ID 와 쓴 계정의 확인. 결과를 모르면 다시 쓰지 않는다 (§12) |
| 자기가 만든 변화에 다시 반응한다 | 자기 댓글과 자기 라벨은 변화로 세지 않는다. 에이전트가 만든 이슈는 사람의 라벨 전에는 `ineligible` 이다 |
| 폭주 | 예산, 시도 한도, 사용 한도 확인, 바인딩 일시 정지, 기존 `stop_all` |
| 모델 사용량 | 두 규칙을 나눈다. plan 17 D-13 은 개발과 검증의 규칙이다: 시험은 가짜 provider, 가짜 작업자, 가짜 시계를 쓰고 모델을 부르지 않는다. 제품의 자동 실행은 다른 계약이다: `observe` 와 `propose` 는 사람이 승인하기 전에 턴을 내지 않는다. `gated` 와 `auto` 는 사용자가 수준을 켠 것이 opt-in 이고 예산이 한도이다 (§19 Q-IA1) |
| 두 기기나 두 사람이 같은 이슈를 집는다 | 쓰기가 켜져 있으면 시작할 때 담당자나 라벨로 표시한다. 기기는 지금 한 대이다 (plan 18 Q-K6) |

## 15. kernel 계약 (제안)

이름은 제안이다. 조각마다 다시 정한다. `schema/akra-kernel-spec.json` 과 생성 skill 이 함께 바뀐다.

| 종류 | 이름 | 내용 |
| --- | --- | --- |
| 명령 | `set_issue_automation_mode` | 바인딩의 자율 수준. 올릴 때 확인이 필요하다 |
| 명령 | `sync_issues` | 지금 수집한다 |
| 명령 | `track_issue`, `untrack_issue` | 한 건을 손으로 올리거나 내린다 |
| 명령 | `retriage_issue` | 다시 판정한다 |
| 명령 | `answer_issue_decision` | `{ decision_id, option, note }` |
| 명령 | `ask_issue_decision` | 에이전트가 질문을 연다 |
| 명령 | `hold_issue`, `release_issue`, `steer_issue`, `take_over_issue`, `cancel_issue_work` | 개입 (§10.2) |
| 조회 | `issue_automation` | 바인딩, 수준과 상한, 예산, 수집 상태 (마지막 성공, 오류) |
| 조회 | `issues` | 추적 이슈의 쪽 목록: 상태, 묶음, 판정, 이유, 연결 |
| 조회 | `issue` | 한 건: 공유본, 판정, 관문, 결정 요청, 시간순 기록 |
| 이벤트 | `issues`, `issue_decision` | 바뀜 |
| 기록 | `issue_tracked`, `issue_triaged`, `issue_decision_requested`, `issue_decision_resolved`, `issue_work_submitted`, `issue_work_permit_granted`, `issue_work_permit_revoked`, `issue_integrated`, `issue_feedback_posted`, `issue_held`, `issue_released`, `issue_closed`, `issue_automation_mode_changed` | `by` 를 가진다: 사람, 에이전트, 기본값, 중재자, 규칙. `issue_integrated` 와 `issue_closed` 는 다른 사실이다 (§12.2) |
| 이유 | `source_unreachable`, `issue_unreadable`, `credentials_missing`, `policy_blocked`, `not_selected`, `untrusted_actor`, `criteria_missing`, `needs_clarification`, `risk_requires_human`, `protected_path`, `diff_too_large`, `snapshot_stale`, `already_claimed`, `attempts_exhausted`, `budget_exhausted`, `mode_ceiling`, `executor_unavailable`, `permit_missing`, `permit_stale`, `decision_superseded`, `feedback_unconfirmed` | `summary` 와 `next` 를 가진다 |

- 화면의 묶음은 조회가 정한다 (plan 23 원칙 3): 내 차례, 곧 시작, 실행 중, 리뷰 중, 끝남, 제외.
- 원문은 명령, 조회, 기록 어디에도 없다. 공유본과 해시만 있다.

## 16. Akra Desktop

- **자리.** 오른쪽 패널의 새 탭 "이슈"이다. 최대화할 수 있다 (plan 27 G-5 와 같다). 업무 패널의 GitHub, Jira, Linear
  보기는 "내 일을 읽는 화면"으로 남는다. 이슈 탭은 "이 저장소의 자동화를 다루는 화면"이다.
- **머리.** 바인딩, 자율 수준 선택, 일시 정지, 수집 상태, 예산 (예: 실행 2/2, 오늘 3/5). 상한을 넘는 수준은 고를 수 없다.
- **목록.** 조회가 준 묶음 그대로이다. 행은 번호, 제목, 상태, 이유 한 줄, 마감까지 남은 시간이다.
- **상세.** 공유본, 판정과 이유, 관문 목록, 완료 기준과 증거 (plan 24 E4 의 카드), 시간순 기록, 연결 (thread, PR, 이슈).
- **결정 카드.** 질문, 선택지 버튼, 권고, 마감, "마감 뒤에는 중재자가 결정한다"는 줄. 답은 명령 하나이다.
- **개입.** 보류, 풀기, 지시 추가, 내가 맡기, 다시 판정.
- **관찰 보고.** "최근 30일의 이슈에 이 정책을 쓰면: 자동 시작 12, 사람 승인 5, 제외 30". 사용자는 수준을 올리기 전에
  이것을 본다.
- **정책.** 유효한 값과 그 값의 출처를 읽기 전용으로 보인다 (`akra config` 와 같다). 고칠 때는 파일을 연다.
- **계산하지 않는다.** 판정, 묶음, 마감은 조회의 값이다. renderer 에 판정 코드가 없다.
- **떼어내기.** 화면은 스펙의 조회와 명령만 쓴다. 다른 client (별도 앱, 웹) 가 같은 계약으로 붙는다.
- **시험.** 가짜 Gateway, 숨긴 창, 가짜 provider 서버를 쓴다.

## 17. 순서 (제안)

| 조각 | 내용 | 동작 변화 | 선행 |
| --- | --- | --- | --- |
| IA1 | 수집 (GitHub, 읽기만): `IssueSourcePort` 의 발견과 재동기화, 조회 실패의 `unknown`, 커서가 나아가는 기준, 장부, 바인딩, `observe`, 조회 `issue_automation` 과 `issues`, Desktop 목록 | 켠 저장소에서 목록이 보인다. 턴도 쓰기도 없다 | - |
| IA2 | 정책 파일, 결정적 판정, 이유 코드, 관찰 보고, 초안 화면의 준비 검사, 이슈 양식 | 판정이 보인다 | IA1 |
| IA3 | 결정 요청 (판 포함), `propose`, 작업 지시의 1회 등록, 단계별 허가와 dispatcher·distributor 의 확인, 개입 명령의 철회와 인계 (§5.4) | 첫 끝-끝 흐름: 이슈 → 승인 → task → 기존 실행 → PR. 이슈 작업의 통합은 사람의 승인을 요구한다 | IA2 |
| IA4 | 실행 추적과 결과 반환: 끝 원인별 전이와 횟수 (§12.1), 통합과 닫힘의 분리 (§12.2), `IssueFeedbackPort` (댓글, 라벨) 와 일지, 결과 모름의 처리 | 이슈에 상태가 돌아간다 | IA3 |
| IA5 | `gated`: 자동 시작, 예산, 시도 한도, 실행 뒤 diff 판정, 마감과 `default`. 전달과 통합은 모든 등급에서 사람 리뷰이다 | 반자동 | IA4, Q-IA1 |
| IA6 | 중재자, `auto`, 자동 전달, Gateway 를 사람 없이 다시 켠 뒤 이어 하기, 일시 정지 | 풀자동 | IA5, plan 24 E3 (자동 전달의 조건) |
| IA7 | Jira, Linear, Projects 보드: 자격 증명, 바인딩 화면, 전이 | 출처 확장 | IA4, Q-IA2 |
| IA8 | 이슈 댓글 명령, Telegram 결정, 에이전트의 `ask_issue_decision` | 수단 확장 | IA3 |
| IA9 | 떼어내기 준비: crate 경계, 두 번째 실행기 adapter | 구조 | 필요할 때 |

- 조각 하나가 PR 하나이다. 큰 조각은 #2481 의 하위 이슈 양식으로 다시 나눈다.
- 모든 조각은 kernel 시뮬레이션 시험 (`src/composition/kernel_host/simulation.rs`) 을 통과한다. 시험은 가짜 출처, 가짜
  실행기, 가짜 시계를 쓴다. 모델 턴이 없다.
- 죽였다 살리는 시험의 범위는 조각마다 다르다. IA3 ~ IA5 는 client 가 다시 붙어 kernel 이 다시 올라온 뒤의 복구까지
  본다. 사람 없이 Gateway 가 kernel 을 다시 올리는 복구는 IA6 이 본다.
- 계약의 순서: IA1 의 수집 계약 → IA3 의 영속 등록, 작업별 허가, 사람 개입 → IA4 의 끝과 피드백의 복구 → IA5, IA6 의
  자동화 확대. IA2 의 판정은 IA3 의 입력이므로 그 사이에 둔다.
- #2481 과의 관계:
  - A01 (완료 기준, 증거) 은 IA6 의 자동 전달 조건이다. IA5 까지는 사람 리뷰가 전달 조건이다. 그래서 IA1 ~ IA5 는
    A01 을 기다리지 않는다.
  - A02 (주의함) 는 결정 요청을 보이는 자리이다. IA3 은 주의함이 없어도 이슈 탭으로 시작한다.
  - A03 (인계 manifest) 은 작업 지시의 자료 연결을 맡는다.
  - 이 계획은 #2481 의 새 작업군 후보이다 (§19 Q-IA9).

### IA4 조각 상태

| 조각 | 상태 | 계약과 남은 범위 |
| --- | --- | --- |
| IA4-1 · #2551 | PR [#2638](https://github.com/RefinedStone/codex-exec-loop/pull/2638) · prerelease 반영 | 실행 추적·종료 원인별 복구·횟수 분리·질문 답변. [reference](../reference/current-product.md#issue-automation-execution) |
| IA4-2 · #2552 | PR [#2640](https://github.com/RefinedStone/codex-exec-loop/pull/2640) · prerelease 반영 | 상태 댓글·진행 라벨·전송 일지·읽기 재확인·게시 횟수. [reference](../reference/current-product.md#issue-automation-feedback) |
| IA4-3 · #2553 | PR [#2641](https://github.com/RefinedStone/codex-exec-loop/pull/2641) · 리뷰 대기 | 통합·외부 상태 분리, 사람 닫기 승인, 단일 전송 일지, 작업 지시의 PR 참조 |
| IA4-4 · #2554 | PR [#2642](https://github.com/RefinedStone/codex-exec-loop/pull/2642) · #2641 연계 · 리뷰 대기 | 실행·검증·게시 횟수, 서버 복구 사유, 정확한 대화·PR 연결, 읽기 전용 재확인, 최근 20개 기록 |
| IA4-5 · #2555 | 자동 회귀 통과 · 실제 모델 확인 허가 확보·준비 중 | 실제 SQLite/Git·서비스 재조립을 사용하는 결합 회귀. [검증 범위·증거](../validation/issue-automation-propose-flow.md) |

IA4-5의 회귀 harness는 실제 application service·SQLite·로컬 Git을 사용하고 source/provider 응답,
worker 완료와 사람 확인 capability를 fixture로 제공한다. 등록·댓글 ACK 유실 뒤 같은 저장 경로의
서비스 재조립, 이벤트 10,001개 추가 후 production retention과 durable receipt, 작업 신원·댓글의
중복 방지를 검사한다. 결합 시험 7개와 관련 SQLite·수명 회귀가 통과했으며 실행 SHA와 결과는
연결한 검증 문서에 기록한다. 등록 ACK 유실 뒤 실행 관찰이 누락하던 Submitted fact도 같은
transaction에서 복구하도록 수정했다.
실제 private publication lease가 Unix 전용이므로 Windows native의 실제 publication 경로는
미지원이며 결합 suite는 `#[cfg(unix)]`로 제한한다. WSL Ubuntu의 Linux 내부 독립 clone에서
자동 회귀를 통과했다.
OS process kill과 실제 모델·GitHub 확인은 이 가짜 흐름의 증거에 포함하지 않는다.
live는 Linux의 동일 사용자·`AKRA_HOME` 아래 Desktop·Gateway·helper와 실제 native 확인 GUI를
사용하는 구성이 필요하다. WSLg GUI와 GitHub·모델 인증은 아직 확인하지 않았다.
실제 모델 경로에는 구현 worker 뒤 별도 planning worker가 필요하므로 총 1회 상한을 약속하지 않으며,
별도 사용자 허가와 정식 native 시작 승인을 받은 계획으로만 진행한다.

PR #2638은 선행 [PR #2637](https://github.com/RefinedStone/codex-exec-loop/pull/2637)의
`codex/issue-2550-desktop-interventions` 브랜치를 base로 한 연계 PR이었다. 부모 체인은 2026-11-22
`prerelease`에 반영됐다. 검증 결과는 해당 PR의 실행 SHA별 증거에 기록한다.
IA4 전체의 완료 기준은 후속 조각까지 포함한다.

## 18. 조각의 완료 기준

plan 24 §7 의 방식을 쓴다. 조각마다 아래를 PR 에 붙인다.

| 조각 | 기준 | 확인하는 곳 |
| --- | --- | --- |
| IA1 | 가짜 출처의 이슈 3건이 장부에 들어간다. 다시 수집해도 늘지 않는다. 원문이 장부와 기록에 없다. 추적한 이슈가 닫히거나 선택 라벨을 잃으면 장부가 그것을 안다. 조회 실패는 `unknown` 이다. 쪽 처리 중에 죽어도 이슈를 빠뜨리지 않는다 | `command`: 단위 시험. `data`: 장부의 줄 |
| IA2 | 같은 이슈와 같은 정책은 같은 판정이다. 깨진 정책 파일은 `policy_blocked` 이고 내장 값으로 돌지 않는다. 저장소 파일이 상한을 올리지 못한다 | `command`: 표 기반 단위 시험 |
| IA3 | 승인은 task 를 하나만 만든다 (재전송, 재시작, 기록의 정리 포함). 거절과 보류는 task 를 만들지 않는다. 이슈가 바뀌면 `stale` 이다. 허가가 없는 단계는 실행되지 않는다. 판이 다른 답은 적용되지 않는다 | `command`: 시뮬레이션 시험 |
| IA4 | 응답을 잃어도 댓글이 하나이다. 댓글의 글은 공유본이다. 자기 댓글은 다시 판정을 일으키지 않는다. 같은 끝 기록을 두 번 읽어도 횟수가 한 번만 오른다 | `command`: 가짜 provider 시험 |
| IA4-1 · #2551 | 같은 실행·끝 기록은 한 번만 센다. 한도까지 실행해도 보완이 필요하면 `failed`와 사람 결정으로 이어진다. 취소는 보완 계보에도 적용된다. 현재 질문의 답은 같은 thread로 한 번 시도하며, 결과 모름은 자동 재전송하지 않는다. 통합 후 복구는 새 구현과 실행 횟수 증가 없이 통합 사실을 보존한다 | `command`: domain·SQLite·가짜 실행기·kernel 시뮬레이션 시험. 결과는 구현 PR의 검증 증거 |
| IA5 | 예산을 넘겨 시작하지 않는다. 보호 경로를 건드린 diff 는 `human` 관문으로 간다. 3번 실패하면 `failed` 이다. 임의 시점에 죽였다 살려도 결과가 같다 | `command`: 시뮬레이션 시험 (seed) |
| IA6 | 중재자가 사람 필수 관문을 넘지 못한다. 사람의 답이 먼저이면 중재자의 답을 버린다. 판이 바뀐 뒤에 온 중재자의 답은 `superseded` 이다. 일시 정지 뒤에 새 행동이 없다 | `command`: 시뮬레이션 시험 |

아래 시나리오는 PR #2533 의 리뷰가 더했다. 해당 조각의 완료 기준에 넣는다.

| 상황 | 확인할 결과 | 조각 |
| --- | --- | --- |
| 프로세스의 자율 전달 opt-in 이 켜져 있다. 이슈 작업의 통합을 사람이 승인하지 않았다 | 통합 0회 | IA3 (`propose`), IA5 (`gated`) |
| `auto` 의 결과가 보호 경로를 바꿨다. `delivery` 를 승인하지 않았다 | 소스 push 0회, PR 작성 0회 | IA5 (diff 판정), IA6 |
| 슬롯을 기다리는 중에 `hold` 나 `stale`. 또는 push 직전에 `take_over` | 그 작업의 다음 실행과 전달이 없다. 다른 작업은 계속한다 | IA3 |
| task 생성 전후의 종료, 응답 유실, 기록 10,000건 초과 | task 가 없는 채로 남지 않는다. task 가 둘이 되지 않는다. 같은 `work_order_id` 로 복구한다 | IA3 |
| 추적한 뒤에 이슈가 닫힘, 선택 라벨이 없어짐, 조회 불가 | 낡은 자격으로 실행하지 않는다 | IA1, IA3 |
| 통합 뒤에 검증 계약이 없다 (`PostMergeContractBlocked`) | 구현을 새로 실행하지 않는다. 계약 복구의 길로 간다. 실행 횟수가 오르지 않는다 | IA4 |
| 기본 브랜치는 `main`, 통합 브랜치는 `prerelease` | 통합 상태와 외부 닫힘이 따로 보인다 | IA4 |
| 댓글 생성의 응답 유실과 재조회 실패 | 결과 모름으로 남는다. 댓글이 둘이 되지 않는다 | IA4 |

## 19. 결정 (제안)과 미결정

결정은 모두 제안이다. 사용자가 답하면 상태와 날짜를 고친다.

| ID | 결정 | 상태 |
| --- | --- | --- |
| IA-01 | 이름은 "이슈 자동화"이다. 용어는 §2 를 쓴다 | 제안 |
| IA-02 | 모듈은 port 로만 밖과 만나는 application 모듈이다. 명령은 kernel spec 의 범위 `issue` 이다. 타이머는 런타임 호스트가 돌린다 | 제안 |
| IA-03 | 실행은 planning task 와 병렬 전달을 쓴다. 새 실행 경로를 만들지 않는다. 기존 실행기는 작업별 허가를 확인한다 | 제안 |
| IA-04 | 체계는 세 층이다: 코드의 닫힌 목록, 정책 파일, 플레이북 skill. 권한은 정책에서만 온다 | 제안 |
| IA-05 | 자율 수준은 `off`, `observe`, `propose`, `gated`, `auto` 이다. 기본은 `observe` 이다 | 제안 |
| IA-06 | 자율 상한, 쓰기 허락, 예산 최대는 사용자 파일에서만 온다 | 제안 |
| IA-07 | 결정 요청은 대상을 가진 일반 객체이다 (plan 18 K5 의 묻기·답하기) | 제안 |
| IA-08 | 중재자는 사람 필수 규칙, `spec`, `merge` 를 넘지 못한다 | 제안 |
| IA-09 | 첫 provider 는 GitHub Issues 이다. 수집은 폴링이다 | 제안 |
| IA-10 | 정책 파일은 `issue-policy.toml` 과 따로 둔다 | 제안 |
| IA-11 | 유효 권한은 프로세스 상한 ∩ 바인딩과 이슈의 정책 ∩ 현재 관문의 승인이다. 슬롯 배분, push, PR 작성, 통합의 직전에 확인한다 (§5.4) | 제안 (리뷰 반영) |
| IA-12 | "task 하나"는 planning authority 의 작업 지시 기록이 보장한다. 명령의 멱등 키는 영수증의 중복만 막는다 (§5.4) | 제안 (리뷰 반영) |
| IA-13 | 수집은 발견과 재동기화로 나눈다. 조회 실패는 `unknown` 이다 (§13) | 제안 (리뷰 반영) |
| IA-14 | 실행의 끝은 원인별로 다르게 대응한다. 실행, 검증, 게시의 횟수를 따로 센다 (§12.1) | 제안 (리뷰 반영) |
| IA-15 | 통합과 외부 이슈의 닫힘은 다른 사실이다. `done` 은 외부 닫힘을 요구하지 않는다 (§12.2) | 제안 (리뷰 반영) |
| IA-16 | 결정 요청과 허가는 판 (정책 판, 필드 해시, 작업 지시, 관문 판) 에 묶인다. 판이 다른 답은 적용하지 않는다 (§10.1) | 제안 (리뷰 반영) |
| IA-17 | 저장소 파일은 신뢰하는 행위자, 보호 경로, 사람 필수 규칙을 풀지 못한다 (§7.3) | 제안 (리뷰 반영) |

| ID | 질문 | 기본값 (답이 없을 때) |
| --- | --- | --- |
| Q-IA1 | `gated` 와 `auto` 는 사람의 건별 행동 없이 턴을 시작한다. 제품의 계약을 "자율 수준을 켠 것이 opt-in 이고 예산이 한도이다"로 정하는가. D-13 은 개발과 검증의 규칙이므로 바꾸지 않는다 | IA5 전에 정한다. 그 전에는 `propose` 까지만 만든다 |
| Q-IA2 | Jira 와 Linear 의 자격 증명을 어디에 두는가 | Gateway 의 OS 키 저장소 (§13 의 a) |
| Q-IA3 | 공개 저장소에서 어디까지 허락하는가 | 상한은 `propose` 이다. 신뢰하는 행위자의 라벨이 필수이다 |
| Q-IA4 | 중재자는 어떤 모델과 프로필인가. 확신의 기준은 무엇인가 | 병렬 에이전트 프로필 하나를 읽기 전용으로 쓴다. 기준은 IA6 에서 측정으로 정한다 |
| Q-IA5 | 이슈 하나가 task 여럿이 되는가 | 이슈 하나는 작업 지시 하나이다. 작업 지시는 최초 task 하나와 PR 검증의 보완 task 들을 가진다. 범위의 분해는 하위 이슈를 제안하는 것으로 한다 |
| Q-IA6 | 이슈에 쓰는 것의 기본은 무엇인가 | 모두 꺼짐. IA4 는 댓글과 라벨만 만든다 |
| Q-IA7 | `gated`, `auto` 인 저장소는 kernel 이 계속 떠 있고 TUI 를 막는다. 받아들이는가 | 예 (K6 까지) |
| Q-IA8 | 이슈 자동화가 병렬 모드를 스스로 켜는가 | 아니요. 화면이 이유를 보인다 |
| Q-IA9 | 이 계획을 #2481 의 작업군으로 올리는가 | 예. 이 문서를 근거로 댓글에서 제안한다 |
| Q-IA10 | 화면의 자리는 오른쪽 패널의 탭인가, 주 화면인가 | 오른쪽 패널의 탭이고 최대화할 수 있다 |
| Q-IA11 | 사람의 답을 무엇으로 증명하는가. 터미널에서 사람이 답하는 길을 여는가 | Desktop main 의 확인 창을 지난 답만 `by: human` 이다. 터미널의 길은 열지 않는다 |
| Q-IA12 | 수집 주기와 요청 한도 | 5분. 실패하면 간격을 늘린다 |
| Q-IA13 | 통합 브랜치가 기본 브랜치와 다를 때 외부 이슈를 언제 닫는가 | `close.when = never`. 이슈를 열어 두고 사람이 닫는다 (§12.2) |
| Q-IA14 | `take_over` 한 작업의 슬롯 lease 를 어떻게 하는가 | 사람이 끝낼 때까지 그 슬롯을 쥔다. 호스트는 정리하지 않는다. 세부는 IA3 에서 정한다 |
| Q-IA15 | 작업 지시 기록을 planning authority 의 어디에 두는가 (새 표인가, 기존 문서의 필드인가) | 새 표. schema version 을 올리지 않는다 |

## 20. 하지 않는 것

- webhook 서버.
- 여러 사용자와 권한 (RBAC).
- 동적 plugin 로딩.
- 이슈 사이의 의존 그래프(DAG) 실행.
- 사람 리뷰를 중재자로 바꾸는 것.
- 보호 브랜치로 가는 자동 통합의 새 길.
- 이슈 원문과 댓글 원문의 저장.
- 모델이 쓴 요약을 통과로 세는 것 (plan 24).
- 이슈 한 건으로 여러 저장소를 고치는 것.
- 병렬 슬롯 수의 변경.

## 21. 확인하지 않은 것

- Gateway 를 다시 켠 뒤, 병렬 모드가 켜져 있던 저장소의 kernel 을 누가 다시 올리는가. 문서는 "첫 attach 가 kernel 을
  만든다"만 말한다. 코드를 읽지 않았다.
- 작업 지시의 이슈 참조와 브랜치 이름을 병렬 전달까지 나르는 길. PR 본문을 만드는 함수만 읽었다. queue record 와 브랜치
  이름 규칙 (`branch_names.rs`) 은 읽지 않았다.
- dispatcher 가 슬롯을 배분하는 트랜잭션과 distributor 의 push, PR 작성, 통합의 직전에 허가 확인을 넣는 크기. 리뷰가
  가리킨 자리 (`orchestrator_loop.rs`, `distributor/delivery/github.rs` 의 `pull_request_readiness_error`) 만 읽었다.
- 닫는 키워드의 동작. GitHub 문서의 내용이다. 이 저장소에서 따로 시험하지 않았다.
- GitHub GraphQL 로 "누가 라벨을 붙였는가"를 읽는 비용 (timeline 조회)과 요청 한도.
- Jira 의 전이 API 와 Linear 의 상태 쓰기 API. 기억 기준이다. 구현 전에 공식 문서를 본다.
- 런타임 호스트에 마감 출처 하나를 더하는 크기.
- `issue_*` 기록이 kernel 기록의 보존 한도 (최근 10,000건) 에 주는 영향.
- `account/rateLimits/*`. Akra 는 아직 이것을 부르지 않는다 (plan 23 §4).
- 같은 기능을 가진 다른 제품. 조사하지 않았다.
- 구현 크기. 조각의 닿는 곳만 정했다.
