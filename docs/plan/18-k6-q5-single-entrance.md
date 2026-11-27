# 18-K6·Q5. 입구 하나와 동시성 정책

## 문서 상태

- 상태: **구현 중**. Q5-1, Q5-2a (2026-11-17), Q5-2b (2026-11-18), Q5-2c (2026-11-26), Q5-2d (2026-11-27, #2796), Q5-3 (2026-11-27, #2797), K6-1 (2026-11-27, #2798), K6-2a (2026-11-27, #2799 의 첫 조각)을 구현했다 (§7). 나머지 조각은 제안이다. 현재 제품 동작의 기준은
  `docs/reference/` 이다.
- 상위 문서: [18 Akra 커널 아키텍처](18-akra-kernel-architecture.md) §5.4, §5.8, §5.9, §6 의 K6. 결정 KP-03, KP-04,
  KP-06. [19 Akra 커널 성질 목표](19-akra-kernel-quality-goals.md) §3.2, §5.3, §6 의 Q5, 성질 P4.
- 이 문서를 따로 둔 이유: plan 18 은 K6 을 한 줄로, plan 19 는 Q5 를 한 줄로 적었다. 두 줄은 서로 묶여 있다. TUI 가
  Gateway 의 커널에 붙으려면 커널이 thread 를 클라이언트와 무관하게 다뤄야 하고(Q5), 병렬 제어의 전환
  ([18-K4-9b](18-k4-9b-parallel-control.md))은 K6 을 기다린다.
- 기준일: 2026-11-18 (Asia/Seoul)
- 기준 커밋: `2459191d` (`origin/prerelease`).
- 범위: 동시 턴 수의 정책, thread 별 상태 기계, TUI 가 Gateway 의 커널에 붙는 길, 직접 경로의 삭제, 이름 정리.
  관찰자와 기능 카탈로그(plan 20), 엔진 adapter(Q6), 병렬 제어의 전환(K4-9b)은 범위 밖이다.

## 1. 목표

| 번호 | 목표 | 확인하는 방법 |
| --- | --- | --- |
| S1 | 동시 턴 수는 정책 값이다 (P4). 기본값은 1 이다 | 정책 값만 2 로 바꾼 시험이 thread 두 개의 턴을 함께 돌린다. 코드의 다른 곳은 바꾸지 않는다 |
| S2 | thread 마다 상태 기계가 있다. 보는 자리와 바꿔 넣지 않는다 | `conversation_turn_reducer.rs` 에 `mem::swap` 이 없다 |
| S3 | 저장소의 모든 클라이언트가 커널 하나에 붙는다 (KP-04) | Gateway 의 커널이 도는 저장소에서 TUI 가 시작하고, 같은 thread 요약과 자동 후속 값을 본다 |
| S4 | 입구의 직접 경로가 없다 | K4 가 "TUI 가 커널을 쥐고 있으면 직접 실행"으로 남긴 갈래(K4 §3.7, K4-9d §3.1)가 코드에 없다 |
| S5 | 이름이 구조와 같다 | `NativeClientRuntime` 이 `Kernel` 이다 (plan 18 §5.9) |

## 2. 현재 상태 (2026-11-17 코드 조사)

### 2.1 "대화 턴은 하나"가 있는 자리

| 자리 | 규칙 | 근거 |
| --- | --- | --- |
| 턴을 받을 때 | 어느 thread 든 턴이 돌고 있으면 `RejectedActive` 로 거절한다 | `src/core/app/conversation_turn_reducer.rs` `admit_turn_submission_in_session` |
| 큐에 남긴 thread 프롬프트 | 모든 session 이 쉴 때만 다시 낸다 | `src/core/app/controller.rs` `queued_thread_prompt_is_due` |
| 턴 뒤 경로 | 경로를 기다리는 평가가 둘 이상이면 모두 이어 가지 않고 끝낸다 | `conversation_turn_reducer.rs` `next_post_turn_host_step`, `PostTurnHostStep::SettleWithoutContinuation` |
| 자동 후속 설정 | 최대 턴 수와 멈춤이 커널에 하나이다 | 프로토콜의 `auto_follow` 조회, `auto_follow_configured` 기록 |
| 정지 | `stop_all` 은 app-server 의 모든 턴에 닿는다. 끝나지 않은 정지가 있으면 새 턴을 받지 않는다. 정지 요청은 보는 session 의 턴만 적는다 | `admit_turn_submission_in_session` 의 `RejectedStopPending`, `admit_stop_request` |
| 승인 검토 저장 | 모든 thread 가 "지금 턴" 자리 하나를 함께 쓴다. 새 턴이 그 자리를 바꾼다 | `ApprovalReviewPersistenceCoordinator::begin_conversation_turn` (2026-11-17 리뷰). Q5-2d 에서 session 마다 두고 이 함수를 지웠다 |

값이 2 이상이면 이 자리들에서 동작이 틀린다. 예: 둘째 턴이 승인 검토 저장의 자리를 바꾸면, 첫 턴의 저장 실패를
보는 대화에 알리지 않는다. 그래서 Q5-3 전에는 값을 2 이상으로 열지 않는다.

K2 §3.3 은 이유를 "턴 스트림 worker 가 턴 하나를 기준으로 짜여 있다"로 적었다. 지금은 다르다.
`spawn_turn_submission_worker` (`src/composition/core_turn_submission.rs`)는 턴마다 worker 를 하나 만들고, worker 가
보내는 입력은 모두 `TurnSubmissionCorrelation` 을 가진다. core 는 그 값으로 주인 session 을 찾는다. 남은 가정은 위
표의 다섯 자리이다.

확인하지 않은 것: app-server 연결 하나가 턴 스트림 둘을 함께 받는가. 병렬 slot 은 worktree 마다 다른 thread 로
이미 함께 돈다. main session 의 턴 둘은 실제 app-server 로 돌려 보지 않았다 (모델 턴이 필요하다).

### 2.2 session 을 바꿔 넣는다

`ConversationTurnFeatureReducer` 는 보는 session 하나(`session`)와 뒤로 보낸 session 의 목록
(`background_sessions`)을 가진다. 뒤의 session 의 입력은 `with_owning_session` 이 그 session 을 보는 자리와
`mem::swap` 으로 바꾼 뒤에 같은 규칙으로 처리하고 되돌린다. 부르는 자리는 17곳이다. 그동안 `serving_background` 가
화면용 투영을 막는다.

K2 §3.2 의 설계는 맵(`by_thread`)과 초안(`draft`)과 보는 thread(`viewed`)였다. 구현은 "보는 자리 + 목록"으로
했다. 규칙이 `self.session` 을 직접 읽기 때문이다.

### 2.3 TUI 는 커널을 프로세스 안에 쥔다

- TUI 는 80,752 줄이다 (`src/adapter/inbound/tui`). 시작할 때 저장소 잠금을 잡고
  (`acquire_native_tui_kernel_lock`) 자기 `NativeClientRuntime` 을 만든다. Gateway 의 커널이 그 저장소를 쥐고 있으면
  화면을 열기 전에 끝난다 (K3 §3.5).
- TUI 는 core 의 입력을 직접 보낸다. 자리는 10개 파일의 42곳이다. `AppCommand` 는 39개, `AppEvent` 는 51개이다.
- 커널 프로토콜은 명령 13개, 조회 7개, 이벤트 8개이다 (`schema/akra-kernel-spec.json`). 대화 스트림(글 조각, 도구
  실행, 항목)은 프로토콜에 없다. Desktop 은 대화를 Gateway 의 app-server 중계로 직접 읽는다.
- 프로토콜에 없는 TUI 의 기능: 대화 열기와 스트림, 수동 프롬프트 준비, 계획 작성(초안, 편집기, 승격), 큐 변경,
  directions 정리, 리뷰 센터, 병렬 보드와 병렬 peek, 대화 설정 저장, GitHub 리뷰 폴링 설정, 시작 점검.

### 2.4 그 밖의 흔적 (plan 19 §3.2 에서 남은 것)

| 흔적 | 지금 |
| --- | --- |
| 루프가 세 겹이다 | `KernelHost` → `run_host_cycle` → `CoreRuntime` 그대로이다 |
| 상태 기계가 둘이다 | core 와 병렬 제어부가 따로이다. `report_parallel_mode_to_core` 로 맞춘다 |
| 준비 → 제출 규칙이 둘이다 | core 의 `submit_prepared_thread_prompt` 와 TUI 의 `apply_manual_prompt_intake_outcome` |

## 3. 설계

### 3.1 순서: Q5 를 먼저, K6 을 뒤에

K6 은 프로토콜을 크게 넓힌다 (§2.3). 그 전에 커널이 thread 를 "보는 자리" 없이 다루게 한다. 그러면 TUI 가 붙을 때
"클라이언트별 보기"(K2-5)는 "그 클라이언트가 구독한 thread"가 된다. 커널에는 보는 자리가 남지 않는다.

```text
Q5-1 정책 값 ─→ Q5-2 thread 별 상태 ─→ Q5-3 차선 ─→ K6-1 대화 길 ─→ K6-2 TUI 가 붙는다 ─→ K6-3 직접 경로 삭제
                                                                                      └→ K4-9b (병렬 제어)
Q5-4 병렬 제어부, Q5-5 루프, Q5-6 이름: K6-3 뒤
```

### 3.2 동시 턴 수의 정책 (Q5-1)

core 에 정책 값을 둔다.

```rust
pub struct TurnConcurrencyPolicy {
    /// 모든 thread 를 통틀어 함께 도는 대화 턴의 최대 수이다. 1 이상이다.
    max_concurrent_turns: usize,
}
```

- 기본값은 1 이다 (plan 19 Q-Q3). 이 조각은 사용자 설정을 더하지 않는다. 값은 커널을 조립할 때 정한다.
- thread 하나의 턴은 지금처럼 하나이다. 그 thread 에 턴이 있으면 `RejectedActive` 이다.
- 다른 thread 의 턴은 센다. 도는 턴의 수가 `max_concurrent_turns` 이상이면 `RejectedActive` 이다. 1 이면 지금과
  같다.
- 거절의 이유 코드(`busy`)와 thread 프롬프트의 큐 남기기는 바꾸지 않는다.

값이 2 이상일 때의 규칙은 이 조각에서 안전한 쪽으로 둔다. Q5-3 에서 다시 정한다.

| 자리 (§2.1) | Q5-1 | Q5-3 |
| --- | --- | --- |
| 큐에 남긴 thread 프롬프트 | 그대로: 모든 session 이 쉴 때만 낸다 | 자리가 나면 낸다 |
| 턴 뒤 경로가 둘 이상 | 그대로: 모두 이어 가지 않고 끝낸다 | 자동 후속 차선을 가진 thread 만 이어 간다 |
| 자동 후속 설정 | 그대로: 커널에 하나 | 차선에 하나 |
| 정지 | 그대로: 모든 턴을 멈춘다 | 뒤의 session 의 턴도 정지 요청에 적는다 |
| 승인 검토 저장 | 그대로: 자리 하나 | thread 마다 자리를 둔다 |

### 3.3 thread 별 상태 (Q5-2)

`ConversationTurnFeatureReducer` 의 session 을 맵 하나로 둔다. 키는 커널이 주는 session 번호이다 (Q5-2c, 2026-11-26).
처음 제안은 thread id 를 키로 하는 맵(`by_thread`)과 초안 하나(`draft`)였다 (K2 §3.2 의 모양). 코드와 맞지 않아서 바꿨다
(§7 의 Q5-2c).

```rust
struct ThreadSessions {
    by_key: BTreeMap<SessionKey, ThreadSession>, // 보는 session 도 맵에 있다
    viewed: SessionKey,                          // 보는 자리는 맵의 키이다. K6-2 에서 클라이언트로 옮긴다
    next_key: u64,
}
```

- thread id 는 session 의 값이다 (`TurnStreamState`). thread id 로 찾을 때는 맵을 차례로 본다.
- session 을 뒤로 보낼 때 새 번호를 준다. 그래서 번호의 차례가 뒤로 보낸 차례이다 (이전 목록의 차례와 같다).

- 규칙은 `&mut ThreadSession` 과 공유 값(세대 번호, 기록, 승인 검토 줄)을 인자로 받는다. `self.session` 을 읽지
  않는다. `with_owning_session`, `reduce_background_session`, `serving_background` 를 지운다.
- 화면용 투영은 "이 session 이 보는 thread 인가"로 가른다. 값은 인자이다.
- 한 PR 에 규칙 묶음 하나씩 옮긴다 (제출, 스트림, 정지, steer, 승인, 평가). 기존 시험과 커널 시뮬레이션이 통과해야
  한다. 동작은 바꾸지 않는다.

### 3.4 자동 후속 차선 (Q5-3)

턴 뒤 평가는 저장소의 planning 큐를 본다. thread 둘이 함께 이어 가면 같은 작업을 둘이 집는다. 그래서 자동 후속은
커널에 차선 하나만 둔다.

- 차선의 주인은 thread 하나이다. 자동 후속을 켠 채 턴을 낸 thread 가 주인이 된다. 주인이 있으면 다른 thread 의 턴은
  평가와 자동 후속 없이 끝난다 (보통의 대화 턴이다).
- 주인의 자동 후속이 끝나면(한도, 멈춤, 큐가 빔) 차선을 놓는다.
- 큐에 남긴 thread 프롬프트는 자리가 나면 낸다.
- 병렬 모드는 지금처럼 따로 돈다. 병렬 모드가 켜지면 main session 의 자동 후속은 slot 에 맡긴다 (지금과 같다).

### 3.5 TUI 가 Gateway 의 커널에 붙는다 (K6)

**대화의 길 (K6-1).** TUI 도 Desktop 처럼 대화를 app-server 에서 직접 읽는다. Gateway 가 중계한다. 커널 프로토콜에는
대화 스트림을 넣지 않는다. 이유: 기록의 주인은 Codex 이다 (plan 20 §3). 대화 스트림을 프로토콜에 넣으면 같은 값의
길이 둘이 된다.

**커널의 길 (K6-2).** TUI 는 `akra/kernel/*` 로 붙는다. Gateway 가 없으면 TUI 가 Gateway 를 띄운다 (Desktop 과 같은
방법). TUI 는 저장소 잠금을 잡지 않는다. Gateway 를 쓸 수 없는 환경은 Q-M5-3 에서 정한다.

**하네스 화면.** 프로토콜에 없는 기능(§2.3)은 셋 중 하나로 간다.

| 기능 | 길 |
| --- | --- |
| 대화 열기, 스트림, 수동 프롬프트 | app-server 중계 (K6-1) |
| 시작 점검, 대화 설정, GitHub 리뷰 폴링 설정 | 커널이 한다. 클라이언트는 조회만 한다 |
| 계획 작성, 큐 변경, directions, 리뷰 센터, 병렬 보드 | Q-M5-1 의 답에 따른다 |

**직접 경로 삭제 (K6-3).** TUI 가 커널을 쥐지 않으므로 K4 의 셋째 갈래(잠금의 쥔 쪽이 `tui` 이면 직접 실행)를
지운다. 잠금의 쥔 쪽 종류에서 `tui` 를 뺀다.

### 3.6 남은 정리 (Q5-4 ~ Q5-6)

- Q5-4 병렬 제어부: core 의 상태 기계와 병렬 제어부를 메시지로 맞추는 자리를 없앤다. K4-9b 와 함께 설계한다.
- Q5-5 루프: `KernelHost` 와 `run_host_cycle` 을 한 겹으로 접는다. TUI 가 `run_host_cycle` 을 쓰지 않게 된 뒤에 한다.
- Q5-6 이름: `NativeClientRuntime` → `Kernel`, `core` 의 결정은 application 으로. 마지막에 한다 (plan 18 §5.9).

### 3.7 하지 않는 것

- 커널을 새로 쓰지 않는다 (KP-01). 모든 조각은 기존 시험과 커널 시뮬레이션을 통과한다.
- Q5-1 에서 사용자 설정을 더하지 않는다. 기본값 1 을 바꾸지 않는다.
- thread 마다 OS 스레드를 두지 않는다. "액터"는 thread 별 상태 기계와 입력의 주인 찾기이다. 커널 스레드는 하나이다.
- TUI 를 지우지 않는다 (Q-K3 의 답, KP-06).

## 4. 조각 (PR 단위)

| 조각 | 내용 | 동작 | 확인 |
| --- | --- | --- | --- |
| Q5-1 (구현 2026-11-17, §7) | `TurnConcurrencyPolicy`. 턴을 받는 규칙이 도는 턴의 수를 정책 값과 견준다 | 유지 (기본값 1) | core 시험: 값 1 은 지금과 같다. 값 2 는 thread 둘의 턴을 받고 셋째를 거절한다. 같은 thread 의 둘째 턴은 값과 관계없이 거절한다 |
| Q5-2a (구현 2026-11-17, §7) | session 의 자리(`SessionSlot`)와 찾기. steer, 승인 결정, runtime 알림, 작업 공간 변경의 규칙을 옮긴다 | 유지 | 기존 시험, 커널 시뮬레이션 |
| Q5-2b (구현 2026-11-18, §7) | 남은 규칙을 옮긴다 (제출, 스트림, 정지, 평가, 경로, 모든 session 의 정책 변경). `mem::swap` 을 지운다 | 유지 | 기존 시험, 커널 시뮬레이션 |
| Q5-2c (구현 2026-11-26, §7) | session 을 맵으로 둔다. 키는 커널이 주는 session 번호이다 (처음 제안은 thread id). 보는 자리는 맵의 키이다 | 유지 | 기존 시험, 커널 시뮬레이션 |
| Q5-2d (구현 2026-11-27, §7) | 승인 검토 저장 실패를 알릴 턴을 session 마다 둔다. 정지가 뒤의 session 의 턴도 대상으로 적는다 | 유지 (기본값 1). 정지의 기록만 값 1 에서도 다르다 (§7) | core 시험 둘, 커널 시뮬레이션 |
| Q5-3 (구현 2026-11-27, §7) | 자동 후속 차선. 큐 프롬프트를 자리가 날 때 낸다. 시뮬레이션에 값 2 를 넣는다 | 확장 (값 2 이상에서만 다르다) | 시뮬레이션의 새 불변식: 자동 후속을 이어 가는 thread 는 하나이다 |
| K6-1 (구현 2026-11-27, §7) | TUI 의 대화를 app-server 중계로 읽는 client. 화면은 바꾸지 않는다 | 유지 (Gateway 가 없으면 지금과 같다) | 가짜 Gateway 시험. 모델 턴은 없다 |
| K6-2a (구현 2026-11-27, §7) | Gateway 의 커널이 저장소를 쥐면 TUI 가 잠금 없이 그 커널에 붙어 값을 읽는다. 하네스 화면과 대화·명령을 이유와 함께 잠근다 | 확장 (전에는 끝났다) | 가짜 wire 시험, TestBackend 화면 |
| K6-2 의 나머지 | 대화(중계)와 명령(`submit_prompt`, `set_auto_follow_max_turns`, `stop_all` 등)을 커널 client 에 잇는다. Gateway 가 없으면 TUI 가 띄운다. TUI 가 늘 커널 client 로 연다 | 전환 | 터미널 검증 계약 (`docs/validation/terminal-ui-testing-methodology.md`) |
| K6-3 | 직접 경로와 잠금의 `tui` 종류를 지운다 | 유지 | 입구 경계 시험 |
| Q5-4 | 병렬 제어부를 같은 체계로 | K4-9b 의 답에 따른다 | K4-9b 문서 |
| Q5-5 | 루프를 한 겹으로 | 유지 | 시뮬레이션 |
| Q5-6 | 이름 | 유지 | 구조 시험 |

- Q5-1, Q5-2 는 답을 기다리지 않는다. Q5-3 은 Q-M5-2, K6-2 는 Q-M5-1 과 Q-M5-3 의 답이 필요하다. 답이 없으면
  기본값으로 간다.

## 5. 사용자가 정할 것

| 번호 | 질문 | 기본값 | 다른 선택 |
| --- | --- | --- | --- |
| Q-M5-1 | 얇은 TUI 에 남길 하네스 화면은 무엇인가 (plan 18 Q-K4). 계획 작성, 큐 변경, directions, 리뷰 센터, 병렬 보드 | 대화, 세션 목록, 승인, 자동 후속, thread 요약, 병렬 상태 한 줄만 남긴다. 나머지는 Desktop 과 admin 이 맡는다 | (a) 모두 남긴다. 화면마다 프로토콜의 조회와 명령을 더한다 (K6-2 가 화면 수만큼 커진다). (b) 큐 변경만 더 남긴다 (K4-9d 의 저장 명령을 쓴다) |
| Q-M5-2 | 동시 턴 수를 2 이상으로 쓸 것인가 | 쓰지 않는다. 1 을 유지하고 Q5-3 은 시험과 시뮬레이션에서만 2 를 쓴다 (plan 19 Q-Q3) | 설정으로 연다. 실제 app-server 에서 턴 둘을 돌려 확인해야 한다 (모델 턴이 든다) |
| Q-M5-3 | Gateway 를 쓸 수 없는 환경(SSH 로 한 번만 쓰는 서버)의 TUI 는 어떻게 하는가 | TUI 가 Gateway 를 같은 사용자의 로컬 프로세스로 띄운다. 따로 정하지 않는다 | TUI 프로세스 안에 커널을 두는 방식을 선택으로 남긴다 (S4 가 닫히지 않는다) |
| Q-M5-4 | TUI 와 Desktop 을 같은 저장소에서 함께 쓰게 되는가 | 된다. K6-2 뒤에는 둘이 같은 커널에 붙는다 | - |

답 (2026-11-18):

- Q-M5-1: 기본값으로 한다. 다만 뺀 화면의 코드는 지우지 않는다. K6-2 는 그 화면의 명령을 잠근다
  (`InlineShellCommandAvailability::Locked` 에 "커널 프로토콜에 아직 없다"는 이유를 더한다). 뒤에 필요한 화면부터
  하나씩 연다: 그 화면이 쓰는 조회나 명령을 프로토콜에 더하고 잠금을 푼다. 한 화면이 한 PR 이다. 읽기만 하는
  화면(리뷰 센터, 병렬 보드)이 가장 작다. 리뷰 센터는 Desktop 에 없으므로 그동안에는 admin 웹(`/admin/reviews`)과
  Telegram 만 보여 준다.
- 확인한 것은 리뷰 센터 하나이다: 읽기 로직은 core 에 있고 (`src/core/app/review_center.rs`) TUI 는 그 값을 그린다.
  나머지 네 화면이 같은 모양인지는 K6-2 를 시작할 때 본다.

## 6. 위험

| 위험 | 대응 |
| --- | --- |
| Q5-2 는 `conversation_turn_reducer.rs` (2,223줄)의 규칙 대부분을 건드린다 | 규칙 묶음마다 PR 하나. 동작을 바꾸지 않는다. 시뮬레이션의 seed 수를 늘려 돌린다 |
| 값 2 에서 app-server 가 턴 둘을 함께 받지 못할 수 있다 (§2.1 의 확인하지 않은 것) | 기본값 1 을 유지한다. 설정으로 열기 전에 사용자 허락을 받고 실제 턴으로 확인한다 |
| K6-2 는 사용자가 쓰는 TUI 화면을 없앨 수 있다 | Q-M5-1 의 답 전에는 K6-2 를 시작하지 않는다 |
| K6 전에는 S3, S4 와 K4-9b 가 닫히지 않는다 | 순서를 이 문서의 §3.1 로 고정한다 |
| TUI 가 Gateway 를 띄우면 TUI 를 끝내도 Gateway 가 남는다 | Gateway 의 한가한 커널 내리기(K3-5)와 Gateway 의 종료 규칙을 K6-2 에서 다시 본다 |

## 7. 조각의 결과

### Q5-1 (2026-11-17)

- `TurnConcurrencyPolicy` (`src/core/app/turn_submission.rs`). 값은 모든 thread 를 통튼 턴의 수이고 기본값은 1 이다.
  0 은 1 로 읽는다.
- 턴을 받는 두 자리가 정책을 본다: `admit_turn_submission_in_session` (다른 session 의 턴이 정책의 수만큼 있으면
  `RejectedActive`)과 `thread_rejects_new_prompt` (thread 프롬프트의 `RejectedBusy`). thread 하나의 턴은 값과
  관계없이 하나이다.
- `CoreController::with_turn_concurrency` 가 값을 받는다. `CoreController::new` 는 기본값을 쓴다. 커널의 조립은 아직
  `new` 만 부른다. 그래서 제품에서 값은 항상 1 이다.

확인: core 시험 둘. 값 2 의 core 가 thread 둘의 턴을 받고 셋째를 거절한다. 같은 thread 의 둘째 턴은 거절한다. 값 0
은 1 과 같다. 기존 core 시험, 커널 호스트 시험, 커널 시뮬레이션, 구조 시험이 통과한다. 모델 턴은 없다.

남긴 것:

- 값을 바꾸는 설정은 없다 (Q-M5-2).
- 값이 2 이상일 때 §2.1 의 나머지 자리(큐에 남긴 프롬프트, 턴 뒤 경로, 자동 후속 설정)는 한 턴의 규칙 그대로이다.
  Q5-3 에서 정한다.
- 커널 시뮬레이션은 값 1 만 돌린다.

### Q5-2a (2026-11-17)

- `SessionSlot` (`Viewed`, `Background(index)`)과 `find_session`, `settle_session` 을 더했다
  (`src/core/app/conversation_turn_reducer.rs`). 찾는 차례는 `with_owning_session` 과 같다: 보는 session 이 먼저이다.
  규칙을 처리한 뒤에 일이 끝난 뒤의 session 을 지우는 것도 같다.
- 규칙 여섯을 옮겼다: `apply_correlated_runtime_notice`, `apply_turn_workspace_change`, `admit_turn_steer`,
  `complete_turn_steer`, `admit_approval_decision`, `complete_approval_decision`. 이 규칙들은 session 을 바꿔 넣지
  않고 그 자리의 session 을 직접 고친다.
- steer 와 승인 결정을 받는 규칙은 `ThreadSession` 의 함수가 되었다. 세대 번호는 인자로 받는다.
- "보지 않는 thread 의 결과는 화면에 내지 않는다"는 `serving_background` 대신 `SessionSlot::is_viewed` 로 가른다.

확인: 동작을 바꾸지 않았다. 새 시험은 없다. 기존 core 시험, 커널 호스트 시험, 런타임 시험, 커널 시뮬레이션, 구조
시험이 통과한다.

남긴 것:

- `with_owning_session` 을 부르는 자리가 8곳 남았다 (제출, 스트림, 정지, 평가, 경로, 병렬 재무장). 이 규칙들은
  reducer 의 다른 값(승인 검토 줄, 기록, 다른 session 의 턴)을 함께 읽는다. Q5-2b 부터 옮긴다.
- `mem::swap`, `reduce_background_session`, `serving_background` 는 그대로 있다.

### Q5-2b (2026-11-18)

- 남은 규칙을 모두 옮겼다: 턴 제출(`admit_turn_submission`, `admit_thread_turn_submission`), 스트림
  (`apply_correlated_turn_stream_event`), 정지 완료(`complete_stop_request`), 평가(`start_due_post_turn_evaluation`,
  `admit_post_turn_evaluation`, `complete_post_turn_evaluation`), 경로(`resolve_post_turn_route`), 모든 session 에 거는
  정책 변경(`set_auto_follow_max_turns`, `pause_post_turn_continuation`, `set_parallel_post_turn_rearm`).
- `with_owning_session`, `reduce_background_session`, `serving_background` 를 지웠다. 파일에 `mem::swap` 이 없다
  (목표 S2 의 확인). 보는 session 을 뒤로 보낼 때의 `mem::replace` 는 남는다. 그것은 자리를 옮기는 일이다.
- thread 하나만 보는 규칙은 `ThreadSession` 의 함수가 되었다 (13개). reducer 의 값이 필요하면 인자로 받는다: 세대
  번호, 승인 검토 저장의 줄, "보는 대화를 여는 중"이라는 값.
- 턴을 받는 규칙은 다른 session 의 턴과 정지를 본다. reducer 가 그것을 먼저 읽어서 넘긴다 (`OtherSessionBlocks`).
  "다른 session"의 차례는 이전과 같다 (`other_sessions`): 바꿔 넣던 때에 뒤의 목록에서 보이던 차례이다. 거절이 대는
  턴과 정지가 이 차례로 정해지기 때문이다.
- 한 PR 로 했다. §6 은 규칙 묶음마다 PR 하나라고 했다. 옮기는 방법이 모든 묶음에서 같고 (session 의 함수로 옮기고
  자리로 부른다) 묶음을 나누면 `mem::swap` 이 PR 사이에 남기 때문이다.

확인: 동작을 바꾸지 않았다. 새 시험은 없다. 기존 core 시험, 커널 호스트 시험, 런타임 시험, 구조 시험이 통과한다.
커널 시뮬레이션은 seed 2,000개로 돌렸다 (`AKRA_SIM_SEEDS=2000`). 구조 시험 둘은 옮긴 함수를 `ThreadSession` 에서 찾는다. 새 시험 하나가 "다른 session"의 차례를 값 2 에서 본다
(`a_turn_without_room_names_the_first_turn_of_the_other_sessions`).

코드 리뷰 (2026-11-18). 동작의 차이는 찾지 못했다. 반영한 것:

- 평가 완료를 받는 session 의 함수가 "보는 대화를 여는 중"을 인자로 받지 않는다. reducer 가 먼저 거른다. session 은
  자기 thread 만 안다.
- "다른 session"의 차례에 시험을 더했다. 기본값 1 에서는 이 차례가 보이지 않는다.
- 문서 상태 줄을 고쳤다. Q-M5-1 의 답은 따로 commit 했다.

반영하지 않은 것:

- 턴을 넣을 때마다 다른 session 을 미리 읽는다 (전에는 필요할 때만 읽었다). session 수가 적다. Q5-2c 의 맵에서
  다시 본다.
- reducer 와 `ThreadSession` 에 같은 이름의 함수가 있다. 구조 시험의 `top_level_impl_method_source` 는 이름만 본다.
  그 도우미로 찾는 이름(`accept_post_turn_evaluation`)은 하나뿐이다.
- 규칙 묶음마다 commit 을 나누지 않았다. 옮긴 함수들이 서로를 부르므로 묶음 하나만 옮긴 상태는 컴파일되지 않는다.

남긴 것:

- session 은 아직 "보는 것 하나와 뒤의 목록"이다. thread id 의 맵으로 바꾸는 일은 Q5-2c 이다.
- 승인 검토 저장의 줄은 모든 thread 가 하나를 쓴다. 정지는 보는 session 만 받는다 (`admit_stop_request`). 둘 다
  값 2 이상에서 고쳐야 한다 (Q5-3, §2.1).

### Q5-2c (2026-11-26)

- `ThreadSessions` (`src/core/app/conversation_turn_reducer.rs`): session 의 맵(`by_key`)과 보는 자리(`viewed`)이다. 보는
  자리는 맵의 키(`SessionKey`)이다. reducer 의 `session` 과 `background_sessions` 를 이 값 하나(`sessions`)로 바꿨다.
  `SessionSlot` 을 지웠다. 규칙은 키로 session 을 찾아(`ThreadSessions::find`) 그 session 만 고친다.
- 계획과 다른 점: 키는 thread id 가 아니고 커널이 주는 session 번호이다. 이유는 셋이다.
  1. thread id 는 턴을 받은 뒤에 정해진다 (`TurnStreamEvent::ThreadPrepared`). 그 전에 다른 대화를 열거나 새 대화를
     시작하면, 일이 남은 session 이 thread id 없이 뒤로 간다. 그러면 thread id 없는 session 이 둘이 된다. 처음 제안의
     `draft` 는 하나뿐이다.
  2. `ThreadPrepared` 와 대화 무효는 맵 안의 session 의 thread id 를 정하거나 바꾸거나 지운다. thread id 가 키이면 스트림
     사건마다 키를 다시 정해야 한다. 보는 자리도 같다.
  3. thread id 의 차례(`BTreeMap`)는 지금의 "뒤로 보낸 차례"와 다르다. 이 차례로 정하는 것이 셋이다: 거절이 대는 턴과
     정지(`other_sessions`), 먼저 시작할 평가(`next_post_turn_host_step`), 정책 변경의 차례(`ThreadSessions::for_each`).
- 차례는 이전과 같다. 보는 session 이 먼저이고, 뒤의 session 은 뒤로 보낸 차례이다. 뒤로 보낼 때 새 번호를 주므로, 다시
  연 thread 를 또 뒤로 보내면 차례의 끝으로 간다 (이전 목록의 `remove` 와 `push` 와 같다).

확인: 동작을 바꾸지 않았다. 새 core 시험 셋: 보는 자리가 옮겨도 thread 의 주인 키는 그대로이다, 뒤의 차례는 뒤로 보낸
차례이다, 정리는 일이 없는 뒤의 session 만 지운다. 기존 core 시험, 커널 시뮬레이션, 구조 시험이 통과한다.

남긴 것:

- 정지는 아직 보는 session 만 받는다 (`admit_stop_request`). 승인 검토 저장의 줄은 모든 thread 가 하나를 쓴다. Q5-3 과
  #2481 A05 의 둘째 항목이다. (Q5-2d 에서 고쳤다.)
- 턴을 넣을 때마다 다른 session 을 미리 읽는 것(Q5-2b 의 리뷰)은 그대로 두었다. session 수가 적다.

### Q5-2d (2026-11-27, #2796)

- 승인 검토 저장: 실패를 알릴 턴(`approval_review_turn`)을 `ThreadSession` 에 둔다. 턴을 받을 때 그 session 의 값만
  바꾼다. 쓰기의 줄(`ApprovalReviewPersistenceCoordinator`)은 그대로 모든 thread 가 하나를 쓴다. 실패는 그 턴과
  thread 가 같은 session 에 알린다 (`apply_approval_review_persistence_failure`). 보지 않는 session 이면 화면용 투영은
  내지 않는다 (다른 runtime 알림과 같다). 뒤에서 돌던 thread 를 다시 열 때 턴을 다시 적던 자리는 지웠다. 값이 session
  과 함께 온다.
- 정지: 정지 요청이 뒤의 session 의 도는 턴을 모두 대상으로 적는다 (`record_stop_for_background_turns`). 이미 시작한 턴은
  정지 자체가 닿으므로 시작 뒤 동기화를 하지 않는다. 시작하지 않은 턴은 #2304 처럼 시작하면 그 턴만 멈춘다. 이 기록은
  시도가 없으므로 새 턴을 막지 않는다. 그 thread 를 다시 열면 새 정지가 이 기록을 바꾼다 (정지가 실패했어도 다시 멈출 수
  있다).
- 기본값 1 의 동작: 승인 검토 저장 실패의 알림은 같다. 정지는 보지 않는 thread 의 시작한 턴에도 기록을 남긴다. 그 기록은
  새 턴을 막지 않는다. 바뀐 것 하나: 다시 연 thread 에 시도 없는 기록(#2304 의 시작 전 정지 의도 포함)이 있으면, 전에는
  새 정지를 거절했고 이제는 받아서 그 기록을 바꾼다. 새 정지도 시작 전이면 시작 뒤 동기화를 하므로 멈추는 턴은 같다.

남긴 것:

- 정지가 실패해도 뒤의 session 의 시작 전 정지 의도는 남는다 (#2304 부터의 동작). 같은 thread 의 다음 턴은 이전 턴의 저장
  실패를 알리지 않는다 (이전의 자리 하나와 같다).
- 뒤의 session 이 일을 마치고 지워진 뒤에 온 저장 실패는 알릴 곳이 없다. 화면이 보지 않는 thread 의 알림을 보여 주는 길은
  주의함(A02)에서 다룬다.

### Q5-3 (2026-11-27, #2797)

아래 규칙은 동시 턴 수가 2 이상일 때만 쓴다. 값 1 의 코드 경로는 바꾸지 않았다.

- 자동 후속 차선: `ThreadSession::holds_auto_follow_lane`. 턴을 받을 때 정한다. 일이 남은 다른 session 이 차선을 가졌으면
  이 턴은 차선을 받지 않는다. 아니면 이 턴이 차선을 받는다. 차선은 일이 남은 동안만 뜻이 있다. 그래서 자동 후속이
  끝나면(한도, 멈춤, 큐가 빔) 따로 놓지 않아도 풀린다. 차선을 가진 session 은 많아야 하나이다.
- 계획과 다른 점: §3.4 는 "자동 후속을 켠 채 턴을 낸 thread"가 주인이 된다고 했다. 자동 후속 설정은 모든 thread 가 같은
  값이므로 (§3.4 의 "차선에 하나") 켜짐은 따로 보지 않는다. 주인이 없을 때 턴을 받은 thread 가 주인이다.
- 차선이 없는 thread 의 턴: 평가는 돈다. planning 정산은 하고, 문맥의 `continuation_paused` 를 켜고 `can_queue_next` 를
  끈다. 평가기가 그래도 프롬프트를 답하면, 경로 단계가 그 평가를 이어 가지 않고 끝낸다 (`SettleWithoutContinuation`).
  병렬 경로도 타지 않는다. 계획은 "평가와 자동 후속 없이"라고 했다. 평가를 남긴 이유: 그 턴의 planning 작업 정산이
  평가에서 일어난다.
- 경로를 기다리는 평가가 둘 이상이면: 차선이 없는 것들을 먼저 끝내고, 차선을 가진 하나만 경로를 고른다. 값 1 은 지금처럼
  모두 끝낸다.
- 큐에 남긴 thread 프롬프트: 큐의 머리가 들어갈 자리가 나면 낸다. 자리: 그 thread 가 쉬고, 도는 턴이 정책의 수보다 적고,
  끝나지 않은 정지가 없다 (턴을 받는 규칙과 같다). 같은 planning 작업(`task_id`)을 일이 남은 다른 session 이 쥐고 있으면
  기다린다. 큐는 남긴 차례대로 낸다. 머리가 기다리면 뒤의 것도 기다린다. 턴 뒤 일이 먼저이다 (지금과 같다).
- 결함 수정 (값 2 이상): 사람이 낸 턴이 뒤의 session 의 thread 를 가리키면 거절한다 (`RejectedUnavailable`). 보는
  session 에 넣으면 그 thread 에 턴이 둘 돌 수 있다. 시뮬레이션이 찾았다 (클라이언트가 다른 대화를 연 뒤에 이전 thread
  로 보낸 늦은 프롬프트). 값 1 에서는 지금처럼 받는다 (`next_submission_recovers_pre_start_failure_...` 시험의 흐름).

확인: core 시험 넷 (차선 하나만 이어 간다, 큐 프롬프트는 자리가 나면 나간다, 같은 planning 작업을 다른 thread 가 쥐면
큐 프롬프트가 기다린다, 뒤의 thread 를 가리키는 사람 턴은 거절한다).
커널 시뮬레이션에 값 2 의 시험을 더했다 (`kernel_simulation_keeps_the_invariants_with_two_concurrent_turns`). 가짜
app-server 는 턴을 thread 마다 하나씩 동시 턴 수까지 받는다 (I1). 새 불변식 I12: 자동 후속 턴을 받을 때 다른 thread 의
자동 후속 턴이 돌거나 그 평가가 진행 중이면 안 된다. 기본 seed 는 턴 둘이 함께 돌고 그때 자동 후속이 나와야 한다. 값 1
과 값 2 를 seed 2,000개씩 돌렸다. 결함 주입: 차선 구분을 빼면 값 2 의 seed 149개가 I12 로 실패한다. 값 1 의 seed 는
난수를 더 쓰지 않으므로 이전과 같은 시나리오이다. 모델 턴은 없다.

남긴 것:

- 값 1 에서는 두 thread 의 자동 후속 사슬이 겹칠 수 있다. 한 thread 의 자동 후속 평가가 도는 동안 다른 thread 의 턴 뒤
  자동 후속이 나간다. 턴은 하나씩 돌고 늦은 쪽의 제출은 거절된다. 값 1 의 동작을 바꾸지 않는다는 이 조각의 조건대로
  두었다. I12 는 값 2 이상에서만 본다.
- 차선을 가진 thread 의 자동 후속이 큐 머리로 집는 작업을, 다른 thread 의 thread 프롬프트 턴이 이미 하고 있는 경우는
  planning 큐의 상태(진행 중 작업)에 맡긴다. 커널은 반대 방향(큐 프롬프트가 기다림)만 막는다.
- 동시 턴 수를 사용자 설정으로 여는 일 (Q-M5-2, 미등록 항목). 실제 app-server 에서 턴 둘을 돌려 확인해야 한다.

### K6-1 (2026-11-27, #2798)

- TUI 가 시작할 때 이 사용자의 Gateway endpoint 에 접속해 본다 (2초 상한). 받으면 TUI 의 app-server adapter 는 자기
  app-server 대신 Gateway 중계를 쓴다. 받지 않으면 지금처럼 자기 app-server 를 띄운다. 결정은 시작 때 한 번이다.
- 전송: `AppServerTransport::GatewayRelay`. 연결마다 `akra __gateway-app-server-relay <endpoint>` 자식을 띄운다. 자식은
  stdio 와 Gateway endpoint 사이에서 바이트만 나른다 (`src/adapter/outbound/app_server/gateway_relay.rs`). 연결은 데몬
  커널의 `GatewayProxy` 와 같은 WebSocket 전송(`proxy_transport`)을 쓴다. 그래서 연결 코드와 재연결 규칙은 그대로이고,
  Gateway 세션은 TUI 를 Desktop 의 접속처럼 중계한다 (기록기, 턴 관찰 #2850 포함).
- 자식은 `akra` 자신이다. Gateway 의 bridge 명령(`akra-gateway desktop-*-bridge`)처럼 다른 실행 파일을 찾지 않는다. Unix 의
  `codex app-server proxy --sock` 는 Windows 의 named pipe 에 붙지 못한다.
- 커널 이름 정리: `GatewayUpstreamSocket` 을 `SharedAppServer` (`Upstream`, `Relay`) 로 바꿨다. 커널 프로토콜은 바꾸지 않았다.
- 연결 실패와 복구: 중계가 열리지 않으면 app-server 호출이 `the Akra Gateway relay did not open; start the Gateway (Akra
  Desktop or akra-gateway) and retry` 로 실패한다. 다음 호출이 다시 연결하고 `reconnected to the Akra Gateway after the
  previous relay closed` 알림을 남긴다. 둘 다 기존 오류·경고 표시를 지난다.

확인: relay 단위 시험 다섯 (두 방향 바이트, 입력이 닫히면 Gateway 접속을 닫음, Gateway 찾기, 자식 명령의 모양, 시험 빌드는
개발자의 Gateway 를 찾지 않음). `tests/gateway_relay.rs`: 실제 `akra` relay 자식과 가짜 Gateway 로 `initialize`,
`thread/start`, `turn/start`, 스트림 알림을 주고받고, Gateway 의 close 1012 뒤 자식이 0 으로 끝난다. Gateway 가 없으면
자식이 이유를 적고 1 로 끝난다. `tests/tui_terminal_disconnect.rs` (unix): 가짜 Gateway 가 도는 동안 실제 TUI 가 PTY 에서
시작한다. 시작의 app-server 호출이 Gateway 로 가고, TUI 는 자기 app-server 를 띄우지 않고, TUI 가 끝나면 중계 세션도
끝난다. 모델 턴은 없다.

남긴 것:

- Gateway 가 없을 때 TUI 가 Gateway 를 띄우는 일은 K6-2 이다. 지금은 TUI 가 도는 동안 Gateway 가 사라지면, Gateway 가
  다시 뜰 때까지 대화 호출이 실패한다. TUI 를 다시 시작하면 자기 app-server 로 돈다.
- 중계를 쓰면 TUI 의 app-server 는 Gateway upstream 이다. upstream 은 TUI 와 같은 고정 실행 파일과 보안 설정으로 뜬다.
  TUI 프로세스의 환경 변수는 upstream 에 가지 않는다 (데몬 커널과 같다).
- TUI 의 PTY 시험은 시작과 종료만 본다. 수동 프롬프트와 스트림은 relay 통합 시험이 본다. 화면에서 실패·복구 문구를
  보는 PTY 시험은 없다.

### K6-2a (2026-11-27, #2799 의 첫 조각)

K6-2 는 PR 하나로 끝나지 않는다. TUI 는 `NativeClientPort` 하나로 core 를 읽고 쓰므로, 그 port 를 커널 프로토콜로
구현하는 client 를 먼저 두고 기능을 하나씩 옮긴다. 이 조각은 읽기만 한다.

- 시작: 저장소 잠금을 다른 프로세스가 쥐었고 그 주인이 TUI 가 아니면 (`RepositoryKernelLockHeld.owner`), Gateway 에
  `sessions` 조회를 보낸다 (transient attach). 답이 오면 Gateway 가 이 저장소의 커널을 돌린다. TUI 는 잠금을 잡지
  않고 `GatewayKernelClientRuntime` 으로 연다 (`production::attach_native_tui_to_gateway_kernel`). 답이 없으면 지금처럼
  잠금 오류로 끝난다. 잠금이 비어 있으면 지금처럼 자기 커널로 연다.
- client: 스레드 하나가 Gateway 에 붙어 있는다 (`akra/kernel/attach`, client 이름 `akra-tui`, transient 아님). `sessions`,
  `auto_follow`, `thread_activity` 를 읽고 그 이벤트를 따라간다. 첫 연결과 세션 목록 새로 고침 요청에서 읽기 명령
  `load_sessions` 와 조회를 보낸다. 기존 명령은 ensure-loaded 이므로 이미 Ready 인 provider 목록의 강제 재조회는
  보장하지 않는다. `resync` 면 조회를 다시 보낸다. pipe/socket 연결과 WebSocket handshake 는 5초, attach 와 초기 조회,
  이후 동기화는 각 15초 상한을 둔다. 시작 전 탐색 조회도 10초 상한을 둔다. 정상 연결의 이벤트 대기에는 제한 시간이 없다.
  연결이 끊기면 시작 점검과 세션 목록에 실패/재접속 이유를 보이고 1초 뒤 다시 붙는다. 초기 조회가 모두 응답한 뒤에만
  읽기 전용 연결을 Ready 로 표시하며, 직접 하지 않은 app-server/account 진단은 미확인으로 표시한다.
  값은 TUI 의 core 투영으로 바꾼다 (`SessionCatalogChanged`, `ThreadActivityChanged`, `conversation_runtime.auto_follow`).
- 잠금 (Q-M5-1 의 답): 프로토콜에 조회나 명령이 없는 화면(`:work`, `:parallel`, `:peek`, `:reviews`, `:queue`,
  `:directions`, `:doctor`, `:planning`, `:reset`)은 `KernelProtocolMissing` ("not in the kernel protocol yet; use Akra
  Desktop or the admin web")으로 잠근다. 화면의 코드는 그대로이다. 프로토콜에 명령은 있지만 이 조각이 아직 잇지 않은
  것(`:turns`, `:stop`, `:new`, 프롬프트, 세션 열기와 이름 바꾸기)은 `KernelClientReadsOnly` ("read-only Gateway kernel
  client")로 거절한다. 팔레트는 LOCKED 와 이유를 보이고, 직접 친 명령은 상태 줄과 대화 기록에 이유를 남긴다.

검증 범위: 가짜 wire 시험 (Idle 목록의 로드 명령, 세 값 조회, 갱신/재동기화, attach/초기 조회 제한 시간, 정상 idle 연결).
client 시험 (세션/활동/자동 후속 투영, 동기화 전 Loading, 단절 후 Failed, 재연결 후 Ready, 새로 고침 전달). 잠금 표 시험. 시작 판정 시험
(TUI 가 쥔 잠금과 Gateway 가 없는 경우는 지금처럼 오류). TestBackend 화면 시험: 팔레트의 LOCKED 이유, `:queue` 의 거절
상태 줄, 프롬프트 거절, 커널의 세션 목록 (`[running]`). 모델 턴은 없다.

남긴 것:

- 대화: 세션을 열고 스트림을 읽는 길 (K6-1 의 중계를 커널 client 쪽에서 쓴다), 프롬프트 (`submit_prompt`).
- 명령: `set_auto_follow_max_turns`, `pause_auto_follow`, `stop_all`, `decide_approval`, `rename_session`.
- Gateway 가 없을 때 TUI 가 Gateway 를 띄우고, 잠금이 비어 있어도 커널 client 로 여는 전환. 그 뒤 TUI 가 끝난 다음의
  Gateway 종료 규칙(§6)을 다시 본다.
- 실제 Gateway 와 PTY 로 도는 시험. 이 조각은 가짜 wire 와 TestBackend 로만 확인했다.
- 프로토콜의 승인 요약에는 method 와 종류가 없다. 세션 목록은 승인이 기다린다는 것만 그린다.
