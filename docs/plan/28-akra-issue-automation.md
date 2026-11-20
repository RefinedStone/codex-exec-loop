# 28. 이슈 자동화 (issue automation)

상태: **Proposed**. 현재 제품 동작을 설명하지 않는다. 세부 계약은 조각마다 다시 정한다. · 기준일: 2026-11-20 (Asia/Seoul) ·
기준 커밋: `d1ee3ad0` (`origin/prerelease`) ·
관련: [plan 17 Akra Desktop](17-akra-desktop/README.md), [plan 18 kernel](18-akra-kernel-architecture.md),
[plan 19 kernel 성질 목표](19-akra-kernel-quality-goals.md), [plan 20 관찰-제안](20-akra-observe-and-offer.md),
[plan 23 주의 항목](23-akra-attention-items.md), [plan 24 증거로 끝내는 작업](24-akra-evidence-harness.md),
이슈 #2414 (Issue Core epic), 이슈 #2481 (ADE 로드맵 epic)

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
| (없음) | 작업 지시 (work order) | 실행기에 넘기는 값. Akra 에서는 planning task 이다 | `TaskDefinition` |

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
| 이슈와 task 의 연결 | 없다. 넘기기는 입력창만 채운다 |
| 실행 가능 판정 | 없다 |
| 묻기와 답하기 | plan 18 K5 의 남은 일이다. 승인은 턴 안의 app-server 승인만 있다 |
| 댓글, 라벨, 상태 전이 쓰기 | 없다 |
| 완료 기준과 증거 | plan 24 (제안), #2481 의 A01 |
| 주의함 | plan 23 (제안), #2481 의 A02 |

결론: 수집, 판정, 조정, 양방향이 새 일이다. 실행과 전달은 있는 것을 쓴다.

## 4. 원칙

1. **이슈 글은 신뢰하지 않는 입력이다.** 글은 지시문이 아니라 자료이다. 기본 실행 profile 은 승인 없음과
   `danger-full-access` 이다. 그래서 "누가 실행을 허락했는가"를 글의 내용보다 먼저 본다 (§14).
2. **결정적 규칙이 먼저이다.** 모델은 애매할 때만 쓴다. 모델은 판정을 더 보수적으로만 바꾼다 (plan 20 §4).
3. **판정은 한 곳에서 한 번 한다.** Desktop, CLI, 에이전트는 같은 명령과 같은 조회를 쓴다 (plan 18 KP-04).
4. **사람은 언제나 개입한다.** 모든 자동 결정은 기록된다. 사람은 그것을 멈출 수 있다. 사람의 답이 중재자의 답을 이긴다.
5. **자율의 상한은 사용자가 정한다.** 저장소의 파일은 상한을 낮출 수만 있다. 코드 검증기의 승인과 같은 규칙이다.
6. **켜는 순서는 관찰 → 제안 → 반자동 → 자동이다.** 기본은 `observe` 이다 (`akra.prValidationMode` 와 같다).
7. **새 실행 경로를 만들지 않는다.** 실행은 planning task 이다. 전달은 병렬 전달이다. 사람 리뷰 기본은 그대로이다.
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
      ▲ 명령, 조회, 이벤트                        │ create_task, 조회 (planning, parallel)
      │ (kernel spec 의 범위 issue)               ▼
 Desktop · CLI · 에이전트            planning 큐 → 병렬 슬롯 → push → PR 검증 → 통합 (있음)
```

요청의 "worker 의 집합"은 수집기, 조정기, 실행기이다. 수집기와 조정기가 새 코드이다. 실행기는 기존 병렬 작업자이다.

### 5.2 경계 (떼어낼 수 있는 구조)

모듈은 `domain/issue_automation` 과 `application/service/issue_automation` 이다. 모듈은 아래 port 로만 밖과 만난다.

| port | 방향 | 하는 일 | Akra 의 adapter |
| --- | --- | --- | --- |
| `IssueAutomationPort` | inbound | 명령과 조회 | kernel 프로토콜 (범위 `issue`) |
| `IssueSourcePort` | outbound | 목록, 커서 뒤의 변화, 한 건 읽기, 능력 목록 | GitHub (`gh`). 뒤에 Jira, Linear |
| `IssueFeedbackPort` | outbound | 댓글, 라벨, 담당자, 상태 전이 | provider 마다 하나. 게시 일지의 방식을 다시 쓴다 |
| `IssueWorkExecutionPort` | outbound | 작업 지시의 제출, 취소, 상태 읽기 | planning task 와 병렬 상태 조회 |
| `IssueAutomationStorePort` | outbound | 장부 | SQLite. 저장소 runtime 폴더의 새 파일이다 |
| `IssueAutomationPolicySourcePort` | outbound | 정책 파일 | 파일. `issue_policy` 와 같은 방식이다 |
| `IssueAssessmentModelPort` | outbound | 선택적 모델 판정과 중재자 | 읽기 전용 thread. 처음에는 Fake 이다 |

- 모듈은 planning, parallel, core 의 타입을 import 하지 않는다. architecture test 가 이것을 막는다.
- 떼어낼 때는 모듈과 port 정의를 옮긴다. adapter 는 Akra 에 남는다.
- 다른 실행기(다른 에이전트 런타임, CI)는 `IssueWorkExecutionPort` 의 다른 adapter 이다.
- 지금은 crate 를 나누지 않는다. 이 저장소는 Cargo workspace 가 아니다. 나누는 일은 IA9 이다.
- 장부를 planning DB 에 넣지 않는다. planning DB 의 schema version 을 올리면 이전 binary 가 그 DB 를 열지 못한다.

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
| `done` | 통합했고 결과를 이슈에 돌려줬다 | 끝 |
| `failed` | 시도 한도를 넘었다 | 사람의 결정 |
| `cancelled` | 이슈가 닫혔거나 사람이 취소했다 | 끝 |

상태와 따로 표시 두 개가 있다.

- `held`: 사람이나 규칙이 멈췄다. 보류 중에는 조정기가 새 행동을 내지 않는다.
- `stale`: 판정한 뒤에 이슈의 제목이나 본문이 바뀌었다.

맞추기:

- `next_action(추적 이슈, 정책, 실행 상태, 시각) → 행동` 은 domain 의 순수 함수이다.
- 행동은 닫힌 목록이다: `triage`, `open_decision`, `expire_decision`, `submit_work`, `cancel_work`, `post_feedback`,
  `mark_done`, `none`.
- 입력은 저장된 값이다: 장부, provider 의 최신 스냅숏, task 의 상태, PR 검증 상태. 사건 하나를 놓쳐도 다음 주기에 같은
  답이 나온다. 재시작해도 이어진다 (plan 19 P1).
- 이슈 하나에 진행 중인 행동은 하나이다. 밖으로 쓰는 행동은 먼저 행동 일지에 적는다. 일지의 상태는 게시 일지와 같다
  (`recorded` → `sent` → `confirmed`).

밖의 변화와 대응:

| 밖의 변화 | 대응 |
| --- | --- |
| 판정 뒤에 제목이나 본문이 바뀜 (필드 해시가 다름) | `stale`. 시작 전이면 다시 판정한다. 실행 중이면 결정 요청을 연다 (계속 / 다시 시작 / 멈춤) |
| 이슈가 닫힘 | 시작 전이면 `cancelled`. 실행 중이면 결정 요청을 연다 |
| 다른 사람이 담당자가 됨, 다른 PR 이 연결됨 | `held`, 이유 `already_claimed` |
| 보류 라벨이 붙음 | `held` |
| task 가 `awaiting_user` 가 됨 | 결정 요청으로 올린다 (§10.2) |
| PR 검증이 `Blocked` 나 `Failed` 가 됨 | 시도 수를 올린다. 한도 안이면 다시 하고, 넘으면 `failed` 이다 |

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
trusted_actors = ["maintainer-login"]   # 이슈를 썼거나 라벨을 붙인 사람

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
transition = false
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
- `auto` 의 자동 통합은 지금의 `AKRA_PARALLEL_AUTONOMOUS_DELIVERY=1` 을 그대로 요구한다. 새 길을 열지 않는다.
- 공개 저장소의 기본 상한은 `propose` 이다 (§19 Q-IA3).

### 9.2 관문

| 관문 | 언제 | 통과시키는 쪽 |
| --- | --- | --- |
| `start` | 실행 전 | 사람, 기본값, 중재자, 자동 |
| `spec` | 완료 기준을 확정할 때 | 사람만 |
| `plan` | 구현 계획을 낸 뒤, 파일을 고치기 전 | 사람, 중재자 |
| `delivery` | push 와 PR | 지금의 병렬 전달 규칙 |
| `merge` | 통합 | 사람 리뷰. opt-in 일 때만 자동이다 |
| `close` | 이슈 닫기, 상태 전이 | provider 의 PR 연결 (`Closes #N`) 이 기본이다 |

관문의 처리 방식은 넷이다: `human` (끝까지 기다린다), `timeout_default` (마감 뒤에 권고값), `timeout_arbiter` (마감 뒤에
중재자), `auto`.

### 9.3 사람 필수 규칙 (풀자동의 예외)

아래 중 하나이면 그 관문은 `human` 이다. 마감이 지나도 넘어가지 않는다.

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
            on_timeout: wait | default | arbiter, due_at, asked_by, evidence[],
            state: open | answered | expired | withdrawn,
            answer: { option, note, by, channel, at } }
```

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
| `hold` / `release` | 새 행동을 멈추고 다시 연다. 실행 중인 턴을 끊는 것은 따로 고른다 (`stop_all`) |
| `steer` | 지시를 더한다. 실행 전이면 작업 지시에 붙고, 실행 중이면 그 thread 에 간다 |
| `take_over` | 자동화가 손을 뗀다. 브랜치와 thread 는 남는다. 사람이 그 thread 를 이어 쓴다 |
| `retriage` | 다시 판정한다 |
| `cancel` | 작업을 취소한다. 확인이 필요하다 |

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

1. **작업 지시.** `{ issue, field_hash, title, handoff, acceptance[], scope, branch_hint, closing_reference, profile,
   priority, attempt }`. `handoff` 는 AI 인계 정책을 지난 공유본이다.
2. **task.** Akra 의 adapter 는 `create_task` 를 보낸다. 멱등 키는 이슈, 필드 해시, 시도 번호로 만든다. 같은 지시는 task 를
   두 번 만들지 않는다.
3. **실행.** 병렬 모드가 켜져 있으면 슬롯이 task 를 가져간다. 꺼져 있으면 task 는 큐에 남고 화면이 이유를 보인다. 이슈
   자동화는 병렬 모드를 스스로 켜지 않는다 (§19 Q-IA8).
4. **보면서 하기.** 승인 화면의 다른 선택 "내 대화로 열기"는 지금의 넘기기이다. 그 이슈는 `take_over` 가 된다.
5. **전달.** 지금의 순서 그대로이다. 더하는 것은 PR 의 닫는 참조이다: `Closes owner/name#N`, Jira 키, Linear 의 브랜치 이름.
6. **연결.** 장부가 `이슈 → task → thread, 브랜치, PR` 을 가진다. `TaskDefinition` 은 바꾸지 않는다. 자료 연결은 #2481 의
   A03 이 맡는다.
7. **결과 반환.** 상태 댓글 하나를 만들고 고친다. 댓글을 쌓지 않는다. 숨은 표식으로 자기 댓글을 알아본다. 라벨과 상태
   전이는 정책이 켠 것만 한다.
8. **완료 기준과 증거.** plan 24 가 맡는다. E3 게이트가 생기면 `gated` 와 `auto` 의 전달은 필수 기준의 통과를 요구한다.
9. **예산.** 동시에 도는 이슈 수, 하루에 시작하는 수, 이슈당 시도 수. 사용 한도 (`account/rateLimits/*`) 가 기준을 넘으면
   새로 시작하지 않는다 (#2481 의 R01).

## 13. 수집 (provider 와 자격 증명)

| provider | 읽기 | 쓰기 | 자격 증명 | 순서 |
| --- | --- | --- | --- | --- |
| GitHub Issues | `gh api graphql`: 선택 조건의 열린 이슈, `updatedAt` 커서 | 댓글, 라벨, 담당자. 닫기는 PR 연결 | `gh` 로그인. Gateway 에서 된다 | 첫 번째 |
| GitHub Projects | 보드의 열과 필드 | `issue_board_field_port` (있음) | `read:project` | 뒤 |
| Jira Cloud | JQL, `updated` 커서 | 댓글, 전이 | Electron main 에만 있다. 결정이 필요하다 | 뒤 |
| Linear | GraphQL, `updatedAt` | 댓글, 상태 | Electron main 에만 있다. 결정이 필요하다 | 뒤 |

- 수집은 폴링이다. 기본 주기는 5분이다. "지금 동기화" 명령이 있다. 실패하면 간격을 늘린다.
- webhook 은 받지 않는다. 로컬 데몬은 공개 주소가 없다.
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
| 저장소의 파일이 권한을 올린다 | 자율 상한, 쓰기 허락, 예산 최대는 사용자 파일과 프로세스 환경에서만 온다 |
| 에이전트가 자기 관문을 스스로 통과시킨다 | kernel 의 `caller` 는 인증이 아니다. 그래서 `human` 관문의 답은 사람 확인 경로에서만 받는다: Desktop main 의 확인 창이 만든 값. CLI 로 온 답은 `by: agent` 로 기록하고 `human` 관문을 통과시키지 못한다 (§19 Q-IA11) |
| 비밀 값이 이슈나 댓글로 나간다 | 밖으로 쓰는 글은 `mask_issue_text` 를 지난 공유본뿐이다. 에이전트의 글을 그대로 올리지 않는다 |
| 중복 실행, 중복 댓글 | 멱등 키, 행동 일지, 상태 댓글 하나, 숨은 표식 |
| 자기가 만든 변화에 다시 반응한다 | 자기 댓글과 자기 라벨은 변화로 세지 않는다. 에이전트가 만든 이슈는 사람의 라벨 전에는 `ineligible` 이다 |
| 폭주 | 예산, 시도 한도, 사용 한도 확인, 바인딩 일시 정지, 기존 `stop_all` |
| 모델 사용량 (plan 17 D-13) | `observe` 와 `propose` 는 사람이 승인하기 전에 턴을 내지 않는다. `gated` 와 `auto` 는 D-13 의 해석을 넓힌다. 결정이 필요하다 (§19 Q-IA1) |
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
| 기록 | `issue_tracked`, `issue_triaged`, `issue_decision_requested`, `issue_decision_resolved`, `issue_work_submitted`, `issue_feedback_posted`, `issue_held`, `issue_released`, `issue_closed`, `issue_automation_mode_changed` | `by` 를 가진다: 사람, 에이전트, 기본값, 중재자, 규칙 |
| 이유 | `source_unreachable`, `credentials_missing`, `policy_blocked`, `not_selected`, `untrusted_actor`, `criteria_missing`, `needs_clarification`, `risk_requires_human`, `protected_path`, `diff_too_large`, `snapshot_stale`, `already_claimed`, `attempts_exhausted`, `budget_exhausted`, `mode_ceiling`, `executor_unavailable` | `summary` 와 `next` 를 가진다 |

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
| IA1 | 수집 (GitHub, 읽기만): `IssueSourcePort`, 장부, 바인딩, `observe`, 조회 `issue_automation` 과 `issues`, Desktop 목록 | 켠 저장소에서 목록이 보인다. 턴도 쓰기도 없다 | - |
| IA2 | 정책 파일, 결정적 판정, 이유 코드, 관찰 보고, 초안 화면의 준비 검사, 이슈 양식 | 판정이 보인다 | IA1 |
| IA3 | 결정 요청, `propose`, 개입 명령, 이슈와 task 의 연결 | 첫 끝-끝 흐름: 이슈 → 승인 → task → 기존 실행 → PR | IA2 |
| IA4 | 실행 추적과 결과 반환: 상태 맞추기, 닫는 참조, `IssueFeedbackPort` (댓글, 라벨) 와 일지 | 이슈에 상태가 돌아간다 | IA3 |
| IA5 | `gated`: 자동 시작, 예산, 시도 한도, 실행 뒤 diff 판정, 마감과 `default` | 반자동 | IA4, Q-IA1, plan 24 E3 (권장) |
| IA6 | 중재자, `auto`, Gateway 를 다시 켠 뒤 이어 하기, 일시 정지 | 풀자동 | IA5 |
| IA7 | Jira, Linear, Projects 보드: 자격 증명, 바인딩 화면, 전이 | 출처 확장 | IA4, Q-IA2 |
| IA8 | 이슈 댓글 명령, Telegram 결정, 에이전트의 `ask_issue_decision` | 수단 확장 | IA3 |
| IA9 | 떼어내기 준비: crate 경계, 두 번째 실행기 adapter | 구조 | 필요할 때 |

- 조각 하나가 PR 하나이다. 큰 조각은 #2481 의 하위 이슈 양식으로 다시 나눈다.
- 모든 조각은 kernel 시뮬레이션 시험 (`src/composition/kernel_host/simulation.rs`) 을 통과한다. 시험은 가짜 출처, 가짜
  실행기, 가짜 시계를 쓴다. 모델 턴이 없다.
- #2481 과의 관계:
  - A01 (완료 기준, 증거) 은 IA5 의 전달 조건이다. IA1 ~ IA4 는 A01 을 기다리지 않는다.
  - A02 (주의함) 는 결정 요청을 보이는 자리이다. IA3 은 주의함이 없어도 이슈 탭으로 시작한다.
  - A03 (인계 manifest) 은 작업 지시의 자료 연결을 맡는다.
  - 이 계획은 #2481 의 새 작업군 후보이다 (§19 Q-IA9).

## 18. 조각의 완료 기준

plan 24 §7 의 방식을 쓴다. 조각마다 아래를 PR 에 붙인다.

| 조각 | 기준 | 확인하는 곳 |
| --- | --- | --- |
| IA1 | 가짜 출처의 이슈 3건이 장부에 들어간다. 다시 수집해도 늘지 않는다. 원문이 장부와 기록에 없다 | `command`: 단위 시험. `data`: 장부의 줄 |
| IA2 | 같은 이슈와 같은 정책은 같은 판정이다. 깨진 정책 파일은 `policy_blocked` 이고 내장 값으로 돌지 않는다. 저장소 파일이 상한을 올리지 못한다 | `command`: 표 기반 단위 시험 |
| IA3 | 승인은 task 를 하나만 만든다 (재전송과 재시작 포함). 거절과 보류는 task 를 만들지 않는다. 이슈가 바뀌면 `stale` 이다 | `command`: 시뮬레이션 시험 |
| IA4 | 응답을 잃어도 댓글이 하나이다. 댓글의 글은 공유본이다. 자기 댓글은 다시 판정을 일으키지 않는다 | `command`: 가짜 provider 시험 |
| IA5 | 예산을 넘겨 시작하지 않는다. 보호 경로를 건드린 diff 는 `human` 관문으로 간다. 3번 실패하면 `failed` 이다. 임의 시점에 죽였다 살려도 결과가 같다 | `command`: 시뮬레이션 시험 (seed) |
| IA6 | 중재자가 사람 필수 관문을 넘지 못한다. 사람의 답이 먼저이면 중재자의 답을 버린다. 일시 정지 뒤에 새 행동이 없다 | `command`: 시뮬레이션 시험 |

## 19. 결정 (제안)과 미결정

결정은 모두 제안이다. 사용자가 답하면 상태와 날짜를 고친다.

| ID | 결정 | 상태 |
| --- | --- | --- |
| IA-01 | 이름은 "이슈 자동화"이다. 용어는 §2 를 쓴다 | 제안 |
| IA-02 | 모듈은 port 로만 밖과 만나는 application 모듈이다. 명령은 kernel spec 의 범위 `issue` 이다. 타이머는 런타임 호스트가 돌린다 | 제안 |
| IA-03 | 실행은 planning task 와 병렬 전달을 쓴다. 새 실행 경로를 만들지 않는다 | 제안 |
| IA-04 | 체계는 세 층이다: 코드의 닫힌 목록, 정책 파일, 플레이북 skill. 권한은 정책에서만 온다 | 제안 |
| IA-05 | 자율 수준은 `off`, `observe`, `propose`, `gated`, `auto` 이다. 기본은 `observe` 이다 | 제안 |
| IA-06 | 자율 상한, 쓰기 허락, 예산 최대는 사용자 파일에서만 온다 | 제안 |
| IA-07 | 결정 요청은 대상을 가진 일반 객체이다 (plan 18 K5 의 묻기·답하기) | 제안 |
| IA-08 | 중재자는 사람 필수 규칙, `spec`, `merge` 를 넘지 못한다 | 제안 |
| IA-09 | 첫 provider 는 GitHub Issues 이다. 수집은 폴링이다 | 제안 |
| IA-10 | 정책 파일은 `issue-policy.toml` 과 따로 둔다 | 제안 |

| ID | 질문 | 기본값 (답이 없을 때) |
| --- | --- | --- |
| Q-IA1 | `gated` 와 `auto` 는 사람의 건별 행동 없이 턴을 시작한다. D-13 을 "자율 수준을 켠 것이 시작이고 예산이 한도이다"로 넓히는가 | IA5 전에 정한다. 그 전에는 `propose` 까지만 만든다 |
| Q-IA2 | Jira 와 Linear 의 자격 증명을 어디에 두는가 | Gateway 의 OS 키 저장소 (§13 의 a) |
| Q-IA3 | 공개 저장소에서 어디까지 허락하는가 | 상한은 `propose` 이다. 신뢰하는 행위자의 라벨이 필수이다 |
| Q-IA4 | 중재자는 어떤 모델과 프로필인가. 확신의 기준은 무엇인가 | 병렬 에이전트 프로필 하나를 읽기 전용으로 쓴다. 기준은 IA6 에서 측정으로 정한다 |
| Q-IA5 | 이슈 하나가 task 여럿이 되는가 | 아니요. 분해는 하위 이슈를 제안하는 것으로 한다 |
| Q-IA6 | 이슈에 쓰는 것의 기본은 무엇인가 | 모두 꺼짐. IA4 는 댓글과 라벨만 만든다 |
| Q-IA7 | `gated`, `auto` 인 저장소는 kernel 이 계속 떠 있고 TUI 를 막는다. 받아들이는가 | 예 (K6 까지) |
| Q-IA8 | 이슈 자동화가 병렬 모드를 스스로 켜는가 | 아니요. 화면이 이유를 보인다 |
| Q-IA9 | 이 계획을 #2481 의 작업군으로 올리는가 | 예. 이 문서를 근거로 댓글에서 제안한다 |
| Q-IA10 | 화면의 자리는 오른쪽 패널의 탭인가, 주 화면인가 | 오른쪽 패널의 탭이고 최대화할 수 있다 |
| Q-IA11 | 사람의 답을 무엇으로 증명하는가. 터미널에서 사람이 답하는 길을 여는가 | Desktop main 의 확인 창을 지난 답만 `by: human` 이다. 터미널의 길은 열지 않는다 |
| Q-IA12 | 수집 주기와 요청 한도 | 5분. 실패하면 간격을 늘린다 |

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
- `create_task` 로 만든 task 를 병렬 전달이 어떤 PR 본문으로 올리는가. `distributor/delivery/github.rs` 에 닫는 참조와
  관련된 문자열이 있다. 읽지 않았다.
- GitHub GraphQL 로 "누가 라벨을 붙였는가"를 읽는 비용 (timeline 조회)과 요청 한도.
- Jira 의 전이 API 와 Linear 의 상태 쓰기 API. 기억 기준이다. 구현 전에 공식 문서를 본다.
- 런타임 호스트에 마감 출처 하나를 더하는 크기.
- `issue_*` 기록이 kernel 기록의 보존 한도 (최근 10,000건) 에 주는 영향.
- `account/rateLimits/*`. Akra 는 아직 이것을 부르지 않는다 (plan 23 §4).
- 같은 기능을 가진 다른 제품. 조사하지 않았다.
- 구현 크기. 조각의 닿는 곳만 정했다.
