# K1 상세 설계: 결정 규칙과 타이머를 TUI 밖으로

## 문서 상태

- 상태: **Proposed execution plan**. 현재 제품 동작을 설명하지 않는다.
- 상위 계획: [18 Akra 커널 아키텍처](18-akra-kernel-architecture.md) §5.1~§5.3, §6 K1. 결정 KP-01, KP-02 (승인 2026-11-07).
- 기준일: 2026-11-07 (Asia/Seoul)
- 기준 커밋: `6af4a09b` (`origin/prerelease`). K0 결함 수정이 모두 들어간 뒤이다.
- 범위: 같은 프로세스 안에서 두뇌(클라이언트 런타임, 현재 `src/core`)가 TUI 없이 결정하고 움직이게 한다.
  데몬 분리(K3)와 thread 별 상태(K2)는 범위 밖이다.

## 1. 목표

1. "다음에 무엇을 할지"를 정하는 규칙과 타이머의 주인을 core 와 런타임 호스트로 옮긴다.
2. TUI 가 없어도 턴 종료 → 평가 → 자동 후속 턴 루프가 돈다.
3. TUI 는 입력을 명령으로 바꾸고, 투영을 그리기만 한다. 동작은 지금과 같다.

## 2. 현재 상태 (2026-11-07 코드 조사)

평가 정책 값(자동 후속 허용, 정지 키워드, 일시 정지 등)은 이미 core 의 `ConversationRuntimeAuthority` 가
덮어쓴다 (`tui/app/turn_submission_runtime/post_turn_execution.rs:110-131` 주석). 남은 TUI 책임은 아래 다섯
가지이다.

| ID | TUI 가 하는 일 | 위치 | TUI 가 쓰는 화면 상태 |
| --- | --- | --- | --- |
| T1 | 턴 종료를 감지하고 평가 effect 를 만든다 | `tui/app/conversation_runtime.rs` `queue_post_turn_evaluation` | 대화 기록 view model |
| T2 | 평가 입력 중 최근 사용자 메시지와 최근 응답을 화면 기록에서 꺼낸다 | `post_turn_execution.rs:101-131` | `ConversationViewModel` |
| T3 | 경로가 `AutoSubmit` 이면 자동 후속 프롬프트를 TUI 의 `submit_prompt` 로 다시 낸다. 턴 옵션과 슬롯 lease 넘기기도 TUI 상태로 만든다 | `conversation_runtime.rs:707-741`, `turn_submission_runtime.rs:128-233` | `conversation.turn_options`, 마지막 planning handoff |
| T4 | GitHub 리뷰 폴링: 설정 시작과 "시간이 됐는가" 판단. 설정은 첫 화면이 그려진 뒤에만 시작한다 | `tui/app/shell_runtime.rs:168-226`, `github_polling.rs:493-505` | `first_frame_delivered`, `github_review_polling_state` |
| T5 | 병렬 tick 을 TUI 루프에서 보낸다. 화면용 값(`activity_pulse_visible`)을 함께 보낸다 | `shell_runtime.rs:229-237`, `parallel_mode.rs:432-445` | 패널 투영 sample |
| T6 | 시작 순서 `RunStartupChecks` → `LoadSessionCatalog` | `tui/app/app_runtime.rs:2384-2406`, `tui/shell_chrome.rs` | shell chrome 상태 |

구동 루프: TUI 가 `CoreRuntimeDriver` 를 직접 들고, 최대 100ms 마다 `poll_pending_input` 으로 입력함을
조회한다 (`core/runtime/driver.rs:79-82`). 깨우기 API 가 없다.

## 3. 설계

### 3.1 시계: `CoreInput::Tick`

- core 입력에 `Tick { now }` 를 더한다. 호스트가 주기적으로 넣는다.
- tick 에 무엇을 할지는 core 가 정한다.
  - GitHub 폴링: `GithubReviewPollingState`(설정 결과, 다음 폴링 시각)를 core 상태로 옮긴다. due 이면 core 가
    `PollGithubReview` effect 를 직접 낸다 (T4).
  - 병렬 tick: core 가 tick 에서 병렬 제어부 tick 을 낸다. `activity_pulse_visible` 은 화면 값이므로 tick 에서
    빼고, 화면용 pulse 는 TUI 가 투영에서 계산한다 (T5).
- GitHub 폴링 설정 시작 조건을 "첫 화면" 대신 "시작 점검 완료"로 바꾼다.

### 3.2 시작 순서: `CoreInput::Started`

- 호스트가 부팅 때 `Started { workspace_directory }` 를 한 번 넣는다.
- core 가 `RunStartupChecks` 를 실행하고, 완료 이벤트에서 `LoadSessionCatalog` 를 이어서 낸다 (T6).
- TUI 의 shell chrome 은 같은 명령을 내지 않는다. 진행 표시는 core 투영을 읽는다.

### 3.3 평가 입력: core 가 thread 별 최근 두 값을 갖는다

- 턴 스트림 reducer 가 스트림 이벤트에서 thread 별 `latest_user_message`, `latest_agent_reply` 를 유지한다
  (plan 18 §5.2). 전체 기록은 갖지 않는다.
- `PostTurnEvaluationContext` 의 placeholder 필드를 없앤다. core 가 모든 필드를 자기 상태로 채운다 (T2).

### 3.4 턴 종료 → 평가: core 가 직접 effect 를 낸다

- 턴 종료 이벤트를 처리하는 core reducer 가 평가 effect 를 직접 낸다 (T1).
- TUI 는 같은 이벤트로 화면 기록만 닫는다 (`finish_turn`, settlement 표시).
- 내부 명령 `EvaluatePostTurn`, `ResolvePostTurnContinuation` 은 inbound 어댑터가 만들 수 없게 한다 (§3.7).

### 3.5 자동 후속 턴: core 가 제출한다

- 경로가 `AutoSubmit` 이면 core 가 내부 명령으로 턴을 제출한다. 호출자는 `system:auto-follow` 이다 (T3).
- 턴 옵션: TUI 의 `conversation.turn_options` 는 이미 core 가 복원한 값을 투영한다 (`app_runtime.rs:1420-1444`).
  core 가 thread 의 턴 옵션을 직접 쓴다. TUI 는 설정 변경 명령만 보낸다.
- 슬롯 lease 넘기기: application 정책 객체 `TurnSubmissionPolicy`(이름 제안)가 prompt 출처, 병렬 모드,
  마지막 planning handoff 로 만든다. 지금 TUI 의 `build_parallel_mode_slot_lease_handoff` 규칙을 옮긴다.
- TUI 는 자동 후속 턴을 스트림 이벤트로 받아 화면에 그린다. "auto-follow queued" 상태 문구는 core 이벤트로 만든다.

### 3.6 런타임 호스트

- `composition` 에 `CoreRuntimeHost`(이름 제안)를 둔다. driver, 입력함, 시계, 부팅 입력을 가진다.
- 입력함은 `recv_timeout` 으로 블로킹 수신한다. tick 주기와 다음 deadline 중 이른 쪽까지 기다린다. polling 을 없앤다.
- K1 에서는 TUI 가 같은 스레드에서 호스트를 돌린다 (지금처럼 driver 를 TUI 루프에서 부른다). 호스트만 따로 돌리는
  헤드리스 모드는 시험과 CLI 진입점에서 쓴다. 별도 스레드·프로세스 분리는 K3 이다.

### 3.7 경계 테스트 (`tests/architecture_boundaries.rs`)

- inbound 어댑터는 `AppCommand::EvaluatePostTurn`, `ResolvePostTurnContinuation`, `PollGithubReview` 와
  병렬 tick 입력을 만들 수 없다.
- inbound 어댑터는 `Instant` 비교로 폴링 시각을 판단하지 않는다 (`poll_due` 같은 함수 금지). 화면 애니메이션
  타이머는 예외로 둔다.

## 4. 단계 (PR 단위)

각 단계는 TUI 동작을 유지한다. 단계마다 기존 테스트와 새 테스트가 통과해야 한다.

| 단계 | 내용 | 옮기는 것 | 검증 |
| --- | --- | --- | --- |
| K1-1 (완료 2026-11-07) | `Tick` 입력, GitHub 폴링 일정을 core 로. `AppCommand::PollGithubReview` 제거 | T4 (폴링 시각) | core 단위 테스트: tick 시각별 폴링 effect. TUI 는 tick 만 보냄 |
| K1-2 | 병렬 tick 을 core 의 tick 처리로. pulse 는 화면 계산 | T5 | 기존 `parallel_mode` 테스트, pulse 화면 테스트 |
| K1-3 | `Started` 입력과 시작 순서 | T6 | 부팅 시나리오 테스트. shell chrome 이 시작 명령을 내지 않음 |
| K1-4 | thread 별 최근 사용자 메시지·응답, 턴 종료에서 core 가 평가 effect | T1, T2 | 녹화된 스트림으로 평가 입력 비교 (TUI 경로와 같은 값) |
| K1-5 | `AutoSubmit` 을 core 가 제출. `TurnSubmissionPolicy` | T3 | 자동 후속 루프 2회 시나리오. 턴 옵션·lease 넘기기 비교 |
| K1-6 | `CoreRuntimeHost`, 블로킹 입력함, 헤드리스 시험, 경계 테스트 | 구동 루프 | TUI 없이 가짜 app-server 로 턴 종료 → 평가 → 자동 후속 턴 |

규모: plan 18 의 추정(4~7일, 옮길 로직과 테스트 800~1,500줄)을 유지한다. K1-4, K1-5 가 가장 크다.

## 5. 위험

| 위험 | 대응 |
| --- | --- |
| 자동 후속 루프의 순서가 바뀌어 같은 턴이 두 번 제출되거나 빠진다 | K1-5 전에 현재 루프를 녹화 시나리오로 고정한다. 제출은 core 한 곳에서만 한다 |
| 평가 입력이 화면 기록과 core 값에서 달라진다 (예: steer 로 바뀐 사용자 메시지) | K1-4 에서 두 값을 한동안 함께 계산하고 다르면 로그를 남긴 뒤, 같음을 확인하고 TUI 값을 지운다 |
| 첫 화면 조건을 없애면 GitHub 폴링이 TUI 시작 지연 중에 먼저 돈다 | 시작 점검 완료 뒤로 미룬다. 폴링은 백그라운드 effect 라 화면을 막지 않는다 |
| TUI 파일이 크다 (81,325줄). 옮기는 중 다른 작업과 충돌한다 | 단계마다 작은 PR. TUI 의 해당 코드는 옮긴 PR 에서 바로 지운다 |

## 6. 미결

| ID | 질문 | 영향 |
| --- | --- | --- |
| Q-K1-1 | 헤드리스 시험의 진입점을 CLI 명령(`akra run --headless` 등)으로도 열 것인가, 시험 전용으로만 둘 것인가 | K1-6 범위 |
| Q-K1-2 | tick 주기 (현재 화면 루프 최대 100ms) | 전력 사용, 폴링 지연 |
