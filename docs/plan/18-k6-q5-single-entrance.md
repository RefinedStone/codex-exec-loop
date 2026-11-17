# 18-K6·Q5. 입구 하나와 동시성 정책

## 문서 상태

- 상태: **구현 중**. Q5-1 과 Q5-2a 를 구현했다 (2026-11-17, §7). 나머지 조각은 제안이다. 현재 제품 동작의 기준은
  `docs/reference/` 이다.
- 상위 문서: [18 Akra 커널 아키텍처](18-akra-kernel-architecture.md) §5.4, §5.8, §5.9, §6 의 K6. 결정 KP-03, KP-04,
  KP-06. [19 Akra 커널 성질 목표](19-akra-kernel-quality-goals.md) §3.2, §5.3, §6 의 Q5, 성질 P4.
- 이 문서를 따로 둔 이유: plan 18 은 K6 을 한 줄로, plan 19 는 Q5 를 한 줄로 적었다. 두 줄은 서로 묶여 있다. TUI 가
  Gateway 의 커널에 붙으려면 커널이 thread 를 클라이언트와 무관하게 다뤄야 하고(Q5), 병렬 제어의 전환
  ([18-K4-9b](18-k4-9b-parallel-control.md))은 K6 을 기다린다.
- 기준일: 2026-11-17 (Asia/Seoul)
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
| 정지 | `stop_all` 은 app-server 의 모든 턴에 닿는다. 끝나지 않은 정지가 있으면 새 턴을 받지 않는다 | `admit_turn_submission_in_session` 의 `RejectedStopPending` |

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
| 정지 | 그대로: 모든 턴을 멈춘다 | 그대로 |

### 3.3 thread 별 상태 (Q5-2)

`ConversationTurnFeatureReducer` 의 session 을 맵 하나로 둔다 (K2 §3.2 의 모양).

```rust
struct ThreadSessions {
    by_thread: BTreeMap<String, ThreadSession>,
    draft: ThreadSession,   // thread id 가 아직 없는 새 초안
    viewed: ViewedThread,   // Draft 또는 Thread(id). K6-2 에서 클라이언트로 옮긴다
}
```

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
| Q5-2b ~ | 남은 규칙 묶음을 하나씩 옮긴다 (제출, 스트림, 정지, 평가). `mem::swap` 을 지운다. 그 뒤에 session 을 맵으로 둔다 | 유지 | 기존 시험, 커널 시뮬레이션 |
| Q5-3 | 자동 후속 차선. 큐 프롬프트를 자리가 날 때 낸다. 시뮬레이션에 값 2 를 넣는다 | 확장 (값 2 이상에서만 다르다) | 시뮬레이션의 새 불변식: 자동 후속을 이어 가는 thread 는 하나이다 |
| K6-1 | TUI 의 대화를 app-server 중계로 읽는 client. 화면은 바꾸지 않는다 | 유지 | 가짜 Gateway 시험. 모델 턴은 없다 |
| K6-2 | TUI 가 `akra/kernel/*` 로 붙는다. 잠금을 잡지 않는다. 하네스 화면은 Q-M5-1 의 답대로 한다 | 전환 | 터미널 검증 계약 (`docs/validation/terminal-ui-testing-methodology.md`) |
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
