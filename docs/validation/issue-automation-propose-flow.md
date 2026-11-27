# 이슈 Propose 흐름의 결합 검증

대상: [#2555](https://github.com/RefinedStone/codex-exec-loop/issues/2555), plan 28의 IA4-5.

이 문서는 수집한 이슈가 실제 application service와 SQLite/Git 경계를 거쳐 작업·PR 전달·상태 댓글로 연결되는 회귀의 범위와, 2026-11-22 사용자 허가를 확보한 실제 모델 확인 계획을 구분한다. 아래 단언은 테스트 코드가 검사하는 계약이며, 자동 회귀의 실제 명령·환경·결과는 마지막 검증 기록에 남긴다. 실제 모델 확인은 별도다.

**실제 모델 확인과 native Start는 미실행이며, 실제 모델 호출·시험 PR·상태 댓글은 각각 0회다. live 준비로 시험 이슈 [#2644](https://github.com/RefinedStone/codex-exec-loop/issues/2644), 시험 base branch와 선택 라벨은 생성했다.** 자동 회귀의 URL·remote 응답·turn ID는 fixture 데이터다.

## 실제 경계와 대체한 경계

공통 조립은 [support.rs](../../src/application/service/parallel_mode/tests/issue_automation_flow/support.rs)에 있다.

| 경계 | 회귀에서 사용하는 것 | 이 시험으로 확인하지 않는 것 |
| --- | --- | --- |
| 저장소와 remote | 임시 Git 저장소·slot worktree·로컬 bare remote, 실제 Git commit/push/ref 조회 | github.com으로의 Git push, 원격 인증·네트워크 |
| 이슈 authority | 실제 `SqliteIssueAutomationStoreAdapter`와 파일 기반 SQLite | 메모리 FakeStore로 상태를 대신하지 않음 |
| planning·task·permit | 실제 `SqlitePlanningAuthorityAdapter`, `AkraIssueWorkExecutionAdapter`, `ParallelModeService` | 가짜 task ID 반환으로 등록을 대신하지 않음 |
| application 흐름 | 실제 collection·triage·decision·coordinator·intervention·feedback 서비스 | Gateway/Desktop 전체 실행 |
| 정책 | fixture 전용 파일을 읽는 policy source와 실제 정책 해석 서비스 | 실제 사용자의 설정·자격 증명 |
| 원격 이슈·댓글·PR | 상태와 호출 횟수를 보관하는 fake source/provider/GitHub adapter | 실제 GitHub API의 응답·권한·지연·장애 |
| 사람 시작 승인 | 현재 결정에 `prepare_confirmation`을 호출한 뒤 테스트가 `VerifiedHumanConfirmation::from_verified` capability를 주입 | 실제 native 확인 창, helper 등록, proof 생성·Gateway 검증 |
| worker 완료 | 테스트가 실제 slot에 파일과 commit을 만든 뒤 실제 완료 상태·queue API를 호출 | 구현 모델 턴, host의 전체 worker 종료 orchestration, 숨은 planning 모델 턴 |
| 시간과 장애 | 고정 tick 시각과 호출 경계에 주입한 오류 | 실제 대기 시간, OS process kill, 전원 장애 |

초기화는 실제 `commit_planning_authority_documents`를 사용해 active direction 1개, task 0개, 빈 queue와 accepted 빈 result-output 문서(`Some("# Result Output\n\n")`)를 하나의 SQLite transaction으로 저장한다. 실행 관찰에 필요한 full-document read가 가능하고 task가 비어 있는지도 단언한다. task·work-order는 미리 삽입하지 않으며 시작 승인 이후 실제 등록 경로가 만든다.

서비스 재생성은 같은 workspace와 private runtime 경로를 유지한 채 기존 `FlowServices`를 drop하고, 새 coordinator·SQLite adapter·실제 execution adapter·publication journal adapter를 조립하는 방식이다. 초기화는 `FlowFixture::new`에서 한 번만 수행하며 서비스 재조립에서는 반복하지 않는다. 이미 저장한 task·work-order·ACK 상태를 그대로 읽고, 원격 서비스를 대신하는 fixture 상태와 로컬 bare remote도 유지한다.

### 승인 자동화와 실제 모델 확인의 구분

위 9개 application 결합 시험과 별도로 [사람 확인 경로의 회귀](issue-human-confirmation.md#자동-회귀의-경계)가 있다.
Desktop E2E는 실제 Electron IPC·utility process에 dialog·helper·Gateway 대역을 연결한다.
Rust 회귀는 실제 helper 구현·private IPC·Gateway 검증·SQLite를 한 OS 프로세스에서 조립한다.
추가로 `native_helper_process_confirms_replays_and_revokes_on_eof`는 빌드된 Rust helper를 별도 OS 프로세스로
실행하여 Gateway 등록·intent·proof 소비·명령 재전송·EOF 폐기를 검사한다.
이 시험은 명시적으로 준비한 실행 파일과 임시 `AKRA_HOME`을 요구하며 기존 9개의 통과 수에 합산하지 않는다.

제품은 OS가 물리적인 사람의 클릭을 인증한다고 보장하지 않는다. 따라서 시험 harness가 승인 응답을
제공하는 자동화 검증은 가능하며, 사용자의 직접 조작을 모든 개발·검증의 선행 조건으로 두지 않는다.
자동 회귀와 달리 실제 실행의 Start는 정식 native 경로를 거치며, 시험 capability·위조 proof·직접 DB 수정을 사용하지 않는다.
실제 모델·GitHub PR·상태 댓글 확인은 여전히 별도 완료 기준이며 위 자동 회귀의 성공으로 대체하지 않는다.

## 플랫폼과 실행 준비

초기 고정 실행본 `b7de0bcd`까지의 publication journal은 [PrivateRuntimeLease](../../src/adapter/outbound/filesystem/private_runtime_lease.rs)의 Unix 구현을 사용했고, 당시 Windows 실제 경로는 미지원이었다. 아래 초기 Windows 차단 기록은 그 소스에 대한 기록이다.

현재 후속 기준 `48b6d49a344890fa53fc2c046694e55484b00156`의 [IssuePublishIntentJournalAdapter](../../src/adapter/outbound/filesystem/issue_publish_intent_journal.rs)는 Windows에서 별도 `windows::PublicationLease`와 private SQLite transaction을 사용한다. 따라서 현재 제품 전체에 Windows publication이 없다고 설명하면 안 된다. 이 변경만으로 실제 Propose 전체 흐름이 검증된 것은 아니다.

#2857에서 [parallel_mode tests 등록부](../../src/application/service/parallel_mode/tests.rs)를
Unix/Windows 양쪽으로 열고, policy fixture에 Windows의 실제 owner-private ACL 설정·검증을
추가했다. ACK 유실 회귀도 Unix의 lock 파일과 Windows의 SQLite reservation 파일을 각각
확인하며, Windows에 금지된 legacy lock 파일이 생기지 않는지 단언한다. 현재 9개 시나리오는
PowerShell Full/Rust 및 Windows CI Bash inventory에 포함된다.
원래의 Unix 전용 제한 이유였던 publication lease는 Windows 전용 구현으로 이미 대체됐다.
`issue-decisions.md`가 기록한 별도 direct planning filesystem 시험의 실패는 이 suite의
제품 경계를 뜻하지 않는다. 이 fixture는 실제 SQLite authority API로 문서를 초기화한다.

실제 live 시험은 기존 Linux 고정 실행본·시험 base 계획을 유지한다. Windows의 자동 결합 회귀는
사람 확인 capability·worker 완료·외부 provider를 대체하므로 실제 native Start, 모델 턴 또는
외부 GitHub 쓰기를 수행한 증거가 아니다.

다음 WSL Ubuntu 준비 상태는 확인했다. 이는 실행 호스트 준비 정보이며 결합 시험의 통과 기록은 아니다.

| 항목 | 확인한 값 |
| --- | --- |
| 커널·아키텍처 | `6.18.58.1-microsoft-standard-WSL2` · `x86_64` |
| libc | glibc `2.39` |
| 사용자 | UID `1000`, home `/home/akra` |
| Rust 도구 | cargo·rustc `1.95.0` |
| Git·C 컴파일러 | Git `2.43.0`, GCC `13.3` |
| Linux 실행 도구 | Node `24.14.1`, npm `11.11.0`, Codex `0.146.0`, gh `2.45.0` — 버전을 읽기 전용으로 확인 |
| 소스 접근 | 기존 workspace 접근 가능 |
| WSLg 연결 준비 | `DISPLAY` 환경 변수와 display socket의 존재를 읽기 전용으로 확인 |
| GUI·인증 | Linux Desktop의 WSLg 창 표시, GitHub `RefinedStone`·Codex ChatGPT 로그인 확인. native Start와 실제 모델 호출은 미실행 |

초기 검증 코드 `3f437930`의 결합 시험 7개는 이 Linux 환경 내부의 독립 clone에서 모두 통과했다. 관련 SQLite·수명 회귀를 포함한 당시 31개 시험과, 후속 최종 소스 `ad5f7509`의 43개 관련 시험 결과를 아래 소스별 검증 기록에 구분한다.

## 회귀 시나리오

전체 필터는 `cargo test issue_automation_flow -- --nocapture`이다. 실제 실행 명령·환경·결과는 아래 기록란에 남긴다.

| 파일·테스트 | 검사하는 연결 |
| --- | --- |
| [issue_automation_flow.rs](../../src/application/service/parallel_mode/tests/issue_automation_flow.rs) · `propose_flow_collects_approves_creates_pr_and_returns_one_status_comment` | 수집 → 시작 결정 → 실제 task 하나 → slot/전달 queue → 리뷰 대기 PR·통합 0회 → push된 exact SHA 승인 → 공식 queue 재개·동일 PR 통합 → 실행 상세·동일 댓글 소유권 |
| 같은 파일 · `propose_flow_process_opt_in_never_replaces_start_or_pull_request_human_approval` | process opt-in만으로 task를 만들지 않음. 시작 승인 후 PR 단계는 진행하지만 별도 PR 승인 없이 통합하지 않음 |
| [control_boundaries.rs](../../src/application/service/parallel_mode/tests/issue_automation_flow/control_boundaries.rs) · `issue_automation_flow_waiting_hold_stale_and_source_changes_fence_real_slot_admission` | 승인 후 대기 중 hold·필드 변경·이슈 closed·선택 라벨 제거·원격 읽기 실패가 실제 planning permit과 slot admission을 차단. 재조립 후에도 유지 |
| 같은 파일 · `issue_automation_flow_takeover_at_source_push_preflight_fences_the_real_delivery` | source push 직전 takeover가 실제 전달을 차단하고, 재조립 후에도 원격 refs와 operator 소유 작업 결과를 보존 |
| [registration_recovery.rs](../../src/application/service/parallel_mode/tests/issue_automation_flow/registration_recovery.rs) · `issue_automation_flow_registration_before_commit_recovers_one_task_after_event_retention` | 등록 호출 직전 실패 → 서비스 재조립 → 실제 task 하나 생성 → 이벤트 보관 정리·재전달 후에도 같은 task |
| 같은 파일 · `issue_automation_flow_registration_ack_loss_keeps_one_task_after_event_retention` | 실제 등록 commit 뒤 ACK 유실 → 서비스 재조립 → 이미 저장된 task 하나 복구 → 이벤트 보관 정리 후 중복 없음 |
| [feedback_recovery.rs](../../src/application/service/parallel_mode/tests/issue_automation_flow/feedback_recovery.rs) · `issue_automation_flow_feedback_reconciles_lost_comment_ack_after_sqlite_restart_without_resending` | 댓글 생성 효과 뒤 ACK 유실·첫 readback 실패 → 같은 SQLite 경로에서 서비스 재조립 → readback만으로 같은 intent/comment 확정 |
| registration_recovery.rs · `issue_automation_flow_process_death_after_registration_commit_keeps_one_task_and_one_comment` | 등록 commit 직후 프로세스 종료(오류 처리 없음) → 서비스 재조립 → task 1개, 댓글 1개 |
| feedback_recovery.rs · `issue_automation_flow_process_death_after_comment_create_keeps_one_comment` | 원격 댓글 생성 직후 프로세스 종료, 기록은 `sent`에서 멈춤 → 서비스 재조립 → readback만으로 확정, 댓글 1개 |

정상 흐름은 PR 생성과 승인 후 통합을 다음 순서로 분리해 검사한다.

1. fake PR 리뷰가 `REVIEW_REQUIRED`인 상태에서 실제 전달 queue를 처리해 source를 로컬 bare remote에 push하고 PR을 만든다.
2. queue가 리뷰 부족으로 `Blocked`이고 통합 push는 0회이며 integration ref가 그대로인지 검사한다.
3. 실제로 push된 branch HEAD를 읽어 worker source SHA와 정확히 같은지 확인한 뒤, 그 SHA에 대한 `APPROVED` 응답을 fake에 제공한다.
4. 공개 queue API인 `process_delivery()`를 다시 호출해 정상 리뷰 차단 복구 경로로 재개한다. 테스트가 queue 상태·재시도 시각·permit·task 기록을 직접 바꾸지 않는다.
5. 같은 queue item과 같은 PR 번호·URL로 로컬 bare remote에 통합됐는지 검사한다. 재개 과정의 PR ensure 호출은 동일 PR을 다시 확인한다.

이 흐름은 실제 GitHub 사람 리뷰나 운영 브랜치 통합의 증거가 아니다. live 계획은 아래와 같이 PR까지만 진행하며 통합하지 않는다.

상세 query 회귀는 이미 저장된 실행·feedback 결과를 읽을 때 source resync, provider write, feedback readback의 호출 횟수가 증가하지 않음을 함께 검사한다. 대기 중 차단 회귀의 unrelated task 검사는 permit이 workspace 전체를 막지 않는지 확인하는 범위다. unrelated task의 실제 전달까지 수행했다고 해석하지 않는다.

### 등록 전 실패와 commit 뒤 ACK 유실

[InterruptedSubmission](../../src/application/service/parallel_mode/tests/issue_automation_flow/registration_recovery/fault_execution.rs)은 실제 `IssueWorkExecutionPort`를 감싸는 얇은 wrapper다.

- `BeforeCommit`: 첫 `submit`을 실제 adapter에 전달하기 전에 오류를 반환한다. planning task는 0개이며 issue 쪽에는 등록 의도가 남는다.
- `AfterCommit`: 실제 adapter의 `submit`이 SQLite 등록을 마친 뒤 결과 대신 오류를 반환한다. planning에는 task 1개가 있지만 issue checkpoint는 ACK를 받지 못해 task ID를 아직 기록하지 못한다.
- 나머지 operation은 실제 adapter에 위임한다. 가짜 task를 반환하거나 SQL로 기대 task를 주입하지 않는다.

각 경우 서비스 재조립 후 authority의 task 목록, work-order의 task 목록, task에서 work-order로의 역방향 연결을 함께 검사한다. commit 뒤 ACK 유실에서는 복구 전후 task ID가 정확히 같아야 한다.


### ACK 유실 복구에서 발견한 outbox 누락

실제 등록 adapter가 task를 commit한 뒤 ACK가 유실되면, 다음 coordinator tick은 일반 등록 복구보다 실행 관찰을 먼저 수행한다. 기존 `execution::apply`는 관찰한 task 계보를 빈 checkpoint에 연결했지만 `Submitted` outbox fact는 만들지 않았다. 이후 등록 복구는 task가 이미 연결됐다고 보고 건너뛰므로 등록 사실이 영구적으로 누락됐다. 기존 단위 시험은 실제 실행 관찰을 조립하지 않아 이 순서를 거치지 않았다.

수정은 [execution.rs](../../src/adapter/outbound/db/sqlite_issue_automation_store/execution.rs)에서 처음 빈 task 목록이 실제 관찰 계보로 채워질 때 공통 `events::append_fact`로 `Submitted`만 기록한다. checkpoint·execution progress·receipt·catalog·outbox를 같은 기존 SQLite transaction에 저장한다. 일반 `events::append` 전체를 호출하지 않아 불필요한 permit fact를 추가하지 않는다. 기존 계보와 보완 task 추가에는 등록 이벤트를 소급 생성하지 않는다.

실제 DB 회귀는 [registration_recovery.rs](../../src/adapter/outbound/db/sqlite_issue_automation_store/execution/tests/registration_recovery.rs)에 있다.

- 최초 복구, adapter 재생성, 같은 관찰 반복, ACK 이후 실행 진행·보완 계보 관찰에서 같은 `Submitted` 한 행을 유지한다.
- 이미 task 계보가 있는 checkpoint에는 `Submitted`를 소급 생성하지 않는다.
- catalog revision을 SQLite 최대 정수로 만들어 receipt·progress·checkpoint 쓰기 뒤 catalog 증가에서 실패시킨다. 새 adapter로 읽었을 때 checkpoint·progress·receipt·tracked 상태가 모두 롤백되고 정상 재시도에서 한 번만 기록되는지 검사한다. 이는 late transaction 실패 시험이며 outbox INSERT 자체의 실패 주입은 아니다.

등록 결합 시험의 최종 ACK 뒤 단언은 원래 event ID뿐 아니라 모든 `WorkSubmitted`가 다시 나타나지 않을 것을 요구한다.

### 10,001개 이벤트와 durable receipt

등록 fact를 실제 `KernelEventLogPort::append_kernel_events`로 저장한 다음, 합성 `TurnStarted` 이벤트 **10,001개를 추가**한다. 이 이벤트들은 저장 부하를 위한 데이터이며 모델 실행을 뜻하지 않는다.

실제 append transaction과 production retention DELETE를 사용해 최신 10,000개만 남는지, 원래 `issue_work_submitted` 이벤트는 보관 창에서 사라졌는지 검사한다. 직접 SQL DELETE로 보관 정리를 흉내 내지 않는다.

이때 issue outbox ACK는 일부러 남겨 둔다. 서비스 재조립 후 다음을 검사한다.

1. 보관 창에서 원래 이벤트가 사라져도 별도의 durable issue-work receipt가 남는다.
2. 같은 outbox fact가 다시 전달되지만, 같은 event ID의 append는 새 sequence를 만들지 않는다.
3. task는 여전히 하나이며 등록 당시 ID와 양방향 연결이 유지된다.
4. outbox를 실제 acknowledge한 뒤 다시 조립하면 해당 fact를 더 이상 반환하지 않는다.

이 범위는 호출 실패와 서비스 재생성에 대한 복구다. DB commit 도중 OS를 강제 종료하거나 WAL·파일시스템 손상을 주입한 crash 시험은 수행하지 않았다.

### 댓글 ACK 유실과 실제 journal

fake provider는 매 `CreateComment`마다 새 comment ID를 만든다. fake의 멱등 처리로 중복 전송을 숨기지 않는다. 첫 write에서는 원격 효과를 보관한 뒤 `Unconfirmed`를 반환하고, 첫 readback도 실패시킨다.

테스트는 별도의 read-only SQLite connection으로 `issue_feedback_intents`를 직접 조회한다. 재조립 전에는 같은 intent가 `NeedsConfirmation`이고 receipt·comment owner가 없는지, publication attempts가 1인지 검사한다. 이 상태의 추가 sync가 intent를 바꾸거나 write를 재전송하지 않는지도 검사한다.

기존 feedback·store·publication adapter의 `Arc`가 해제됐음을 확인한 뒤 같은 경로에 새 서비스를 조립한다. 이후 readback은 다음을 만족해야 한다.

- 새 write 없이 같은 intent를 `Confirmed`로 변경한다.
- 확정 receipt의 issue·actor·intent·comment ID·body hash가 원래 원격 효과와 일치한다.
- `CreateComment`는 전체 1회, 재조립 후 추가 mutation은 0회다.
- publication attempts는 1을 유지하고, 확정 outbox fact는 하나다.
- 반복 reconcile은 이미 확정한 intent를 다시 읽거나 쓰지 않는다.

댓글 intent와 receipt는 **실제 SQLite feedback journal**에 저장한다. 실제 파일 기반 `IssuePublishIntentJournalAdapter`는 collection·feedback이 공유하는 publication lease에 사용한다. 별도의 이슈 제목·본문 publication 기록은 이 댓글 시험에서 비어 있고 변경되지 않는지 확인한다. 댓글 receipt가 그 파일 journal에 저장된다고 주장하지 않는다.

### 성공 직후 응답만 잃는 프로세스 종료

#2555 리뷰는 "외부 효과는 성공했지만 그 사실을 기록하기 전에 프로세스가 죽는 경우"를 가장 중요한 연결 검증으로 지목했다. 위 ACK 유실 시험은 오류를 반환하므로 호출자의 오류 처리가 실행된다. 등록에서는 `SubmissionUnavailable`을 저장하고, 댓글에서는 `NeedsConfirmation`을 저장한다. 실제 프로세스 종료에서는 이 저장이 일어나지 않는다. 그래서 두 시험을 추가했다.

- 외부 효과 바로 뒤에서 `panic!`을 일으킨다. 테스트는 `catch_unwind`로 잡고 모든 서비스를 drop한다. 효과 뒤의 어떤 기록 코드도 실행되지 않는다.
- 같은 workspace, 같은 SQLite 파일, 같은 원격 대역 위에 새 서비스를 조립하고 tick을 두 번 실행한다.

| 경계 | 종료 시점의 영속 상태 | 복구가 중복을 막는 방법 | 결과 |
| --- | --- | --- | --- |
| task 등록 | planning task 1개 commit. issue checkpoint는 `task_ids` 비어 있음, `SubmissionUnavailable` 없음 | work-order ID가 이슈에서 결정된다 (`issue_work_order_id`). 복구는 새 등록 전에 `task_ids(work_order_id)`로 실행기의 영속 연결을 먼저 읽는다 | task 1개, 등록 ID 같음, 댓글 1개 |
| 댓글 생성 | 원격 댓글 1개. intent는 원격 쓰기 전의 durable CAS 상태 `sent` | `sent`·`needs_confirmation` intent는 새 쓰기를 막는다. 복구는 다시 쓰지 않고 intent ID marker로 읽기만 한다 | `CreateComment` 1회, 같은 intent가 `confirmed`, receipt의 comment ID 같음 |

두 경계 모두 현재 설계가 중복을 막았다. 제품 코드는 바꾸지 않았다. `panic` unwind는 Rust `Drop`을 실행하므로 OS kill과 같지 않다. 효과 뒤의 응용 기록이 없다는 점만 같다. SQLite commit 도중 종료, WAL 손상, 실제 GitHub 응답 유실은 이 시험의 범위가 아니다.

## 이슈 완료 기준과 증거 대응

| #2555 완료 기준 | 자동 회귀의 증거 | 남은 범위 |
| --- | --- | --- |
| process opt-in만으로 통합하지 않음 | 정상 흐름의 `REVIEW_REQUIRED` 차단과 `propose_flow_process_opt_in_never_replaces_start_or_pull_request_human_approval` | 실제 native 승인·GitHub 리뷰는 live에서 별도 확인 |
| 대기 중 hold·stale, push 직전 takeover로 해당 작업 차단; 다른 작업은 계속 | 새 control 경계 2개와 기존 `work_order_permit_wait_does_not_starve_an_unrelated_task`, `work_order_revoked_after_cherry_pick_preserves_its_result_and_allows_unrelated_integration` | 새 control 시험의 unrelated-ID 단언만으로 실제 다른 task 전달까지 판정하지 않음 |
| 등록 전후 오류·ACK 유실·10,000개 초과 이벤트에서 task 하나 | 새 registration 결합 회귀 2개, 실제 DB 회귀 3개, 10,001개 합성 이벤트와 production retention, 등록 commit 직후 종료 회귀 1개 | 호출 경계 오류·`panic` 종료·서비스 재생성 범위. OS process kill은 미실시 |
| 이슈 closed·라벨 제거·원격 읽기 실패 뒤 낡은 자격으로 실행하지 않음 | 대기 상태 5개 변형의 실제 permit 및 slot admission 차단 | 실제 GitHub 장애·네트워크 지연은 미검증 |
| 통합 후 검증 계약 부재로 구현을 다시 실행하지 않음 | 기존 `post_merge_contract_opens_human_recovery_and_never_reimplements`, `integrated_queue_recovery_is_blocked_even_with_zero_execution_cap`, `integrated_observation_recovery_requires_zero_execution_cap_and_preserves_attestation`, `trusted_integration_is_recorded_once_and_later_contract_recovery_stays_done` | live는 통합하지 않는 범위로 계획 |
| 통합 상태와 외부 닫힘 분리 | 정상 흐름의 Done + 외부 Open, 기존 `external_closure` DB 회귀 4개; Desktop 표현은 [#2554 검증](desktop-issue-execution-tracking.md) | 실제 GitHub 닫힘 쓰기는 수행하지 않음 |
| 댓글 생성 ACK 유실·readback 실패에서 중복 댓글 없음 | 새 feedback 회귀의 실제 SQLite intent·receipt·owner와 provider CreateComment 전체 1회, `sent` 상태 종료 회귀 1개 | 실제 GitHub 응답 유실은 미검증 |
| 실제 확인 SHA·OS·명령·결과·skip 기록 | 아래 자동 회귀 기록과 명시적인 미실행 항목 | **사용자 허가는 확보했으며 실제 실행 준비 중** |

## 실제 실행 준비에서 확인한 Gateway 제어 공백

Gateway의 저장소 kernel은 이슈 observer를 구성하지만 병렬 풀은 꺼진 상태에서 시작했다. 공개 kernel 명령과 Desktop 화면에 실행을 켜는 경로가 없어 Propose의 Start 승인으로 task·permit을 등록해도 실행할 수 없었다. 별도 TUI는 동일 저장소 lock을 공유하지 못하므로 실제 이슈 관찰·전달 경로를 끝까지 확인하는 대안이 되지 않았다.

후속 수정은 `parallel` scope의 `enable_parallel`·`disable_parallel` 명령과 Desktop 커널 패널의 시작·중단 버튼을 추가한다. 기존 NativeClientEvent 경로를 사용하며 readiness, 공개 저장소 opt-in, native Start 승인, work permit, PR 사람 리뷰를 그대로 검사한다. Propose 선택이나 Start 승인으로 전체 풀을 자동으로 켜지 않는다.

명령은 요청 접수(accepted)만 응답한다. Desktop은 실제 query·후속 이벤트에서만 활성화 상태를 반영하고, 실패·조회 단절은 결과 불명으로 표시하며 자동 재전송하지 않는다. 기존 overview·resync·workspace 전환과의 경합도 회귀로 검사한다.

| 후속 Gateway 제어 검증 | 결과 |
| --- | --- |
| 후속 코드 SHA | `896781dcbb55884d65ac473158aab55146e950ad` — 아래 로컬 검증을 통과한 Gateway·Desktop 코드 |
| 명세 생성·일치 | `the_kernel_spec_file_matches_the_spec` 1 passed |
| Rust 제어·권한·재연결 | `parallel_control explicit_parallel_command kernel_simulation_propose_coordinator a_kernel_attached_from_a_linked_worktree_works_in_the_primary_checkout` 11 passed. 하위 폴더·linked worktree에서 대표 checkout의 풀을 제어하는 회귀 포함 |
| Desktop check·전체 회귀 | check 통과. Vitest 963 passed, 기존 7 skipped. 오래된 overview·resync·event 경쟁의 회귀 3개 포함 |
| Windows Full | `powershell -NoProfile -File scripts/check_native_pr.ps1 -Mode Full` 통과. 기존 Git 시험 2개의 5초 제한 초과 뒤 `VITEST_MAX_WORKERS=2`로 같은 게이트를 재실행했다. 시험 코드·시간 제한은 유지했고 native contracts·check·clippy·Desktop 검사 모두 통과 |
| Desktop 전체 E2E | `npm run e2e` 성공. 25개 spec 중 24개 실행 통과, 기존 Windows terminal spec 1개 skip. 시작·중지, pending, 오류, readiness 거부, 후속 상태 이벤트를 실제 Electron에서 확인 |
| 실제 모델 턴 | 0회 |

## 실제 수집과 실행 추적의 경합에서 발견한 Start 질문 누락

2026-11-22에 Linux 시험 workspace에서 #2644가 `Eligible / Ready / Propose`이고 Start 관문이 Human인데도 질문이 생성되지 않았다. 실제 SQLite의 tracked·policy·runtime selection·authorization epoch는 현재 판과 일치했고 작업·결정은 없었다.

검증 소스 `ad5f7509f318c2272c7e6becd9f310342c7769c7`에 동작을 바꾸지 않는 메타데이터 계측만 추가한 진단 Gateway로 다음 순서를 재현했다. 실행 파일 SHA-256은 `19175a3c7d6ee5b3e516e6a5b96dc9e743b9cfa0bd3caa82c238724b98c5e863`이다. 진단용 이벤트는 static 이름과 숫자·boolean만 기록하며 원문·credential·proof는 기록하지 않았다. 진단 코드는 제품 실행본에 포함하지 않는다.

| 시각(UTC) | 관측 |
| --- | --- |
| 13:39:38.587 | 독립 상태 추적 worker가 coordinator gate 취득 |
| 13:39:38.696 | 이슈 74건 추적 시작, 다음 페이지 없음 |
| 13:39:44.692 | 수집 완료. 폐기·취소 없음, coordinator 존재 |
| 13:39:45.817 | 분류 완료 |
| 13:39:45.818 | full tick 진입 직후 `WouldBlock`; 실제 질문 판단 없이 정상 결과 반환 |
| 13:39:53.480 | 추적 worker 종료. gate 보유 구간에 포함된 실행 시간 14,893ms |

full tick의 gate 취득·시험 이슈 action 계산·질문 저장 이벤트는 0건이었다. 이 실행에서 publication lease 대기가 원인이라고 판단할 증거는 없었다. 직접 확인한 원인은 **추적이 공유 gate를 가진 동안 수집 후 tick이 `try_lock()` 경합을 성공으로 반환하며 생략되는 것**이다.

두 경로는 각각 observer의 OS worker에서 호출된다. 수집은 publication lease를 놓고 tick을 호출하며 gate 재진입도 없다. 수정은 두 worker가 gate를 기다려 순서대로 처리하도록 하고, 취득 직후 취소를 다시 검사해 취소된 worker가 복구 쓰기를 하지 않도록 한다. poisoned gate도 정상 완료로 숨기지 않는다.

[실제 재현 메타데이터](issue-2555-live-gate-starvation.json)에 해당 순서와 실행 상태를 보관했다. 수정과 회귀의 결과는 아래 `b7de0bcd` 검증 기록에서 확인한다. 위 진단은 실제 native Start나 모델 → PR → 상태 댓글의 성공 증거가 아니다. 진단 종료 시 planning revision 4, task 0개, pool 비활성, 열린 결정·결정 이력·실제 모델 턴은 모두 0이었고 진단 Gateway와 observer를 정상 종료했다.

## 실제 모델 확인 계획: 허가 확보, 준비 중

[AGENTS.md](../../AGENTS.md)의 검증 지침은 `turn/start`·`turn/steer` 전에 사용자 허가를 요구한다. #2555의 실제 모델 확인은 자동 회귀와 별도로, 대상 저장소·변경 범위·모델 실행 범위·외부 쓰기를 구체화했으며, 2026-11-22 사용자가 해당 계획을 승인했다. 일반 개발·PR 작업 허가를 실제 모델 검증 허가로 바꾸어 해석하지 않는다.

또한 제품의 시작 결정은 [정식 native 확인 경로](issue-human-confirmation.md)를 거쳐야 한다. 위 테스트 capability를 실제 실행에 주입하거나 proof를 직접 만들지 않는다. runtime 수준을 올릴 때와 해당 이슈의 Start를 승인할 때 제품이 요구하는 native 확인을 거친다. 현재 파일 정책은 이미 Propose이므로 이번 준비에는 수준 상향이 필요하지 않다.

### 모델 실행 수의 제한

현재 정상 완료 경로는 다음 추가 모델 실행을 포함한다.

- 구현 worker 1회
- 공식 완료를 반영하는 planning worker 1회 이상

[parallel_mode/orchestrator_loop.rs](../../src/application/service/parallel_mode/orchestrator_loop.rs)의 `run_parallel_dispatch_official_completion`은 [planning worker orchestration](../../src/application/service/planning/worker/orchestration.rs)을 거쳐 [AppServerPlanningWorkerAdapter](../../src/adapter/outbound/app_server/planning_worker.rs)를 호출한다. [app_server 구현](../../src/adapter/outbound/app_server/mod.rs)의 `run_hidden_planning_thread_stream`은 실제 모델 턴을 시작한다. 조사 시점의 planning 모델 상수는 `gpt-5.4`다.

`budget.max_attempts = 1`은 구현 task의 실행 시도 제한이며 이 planning 턴까지 합산하는 전역 예산이 아니다. `:turns`는 단일 세션 auto-follow를 제어하고 parallel continuation은 별도다. 현재 설정·공개 명령으로 **총 모델 턴 1회, 또는 총 2회의 확실한 상한과 모든 자동 재시도 0회**를 보장한다고 말할 수 없다. 완료 시점에 수동 중단을 보내는 것도 다음 턴과 경쟁하므로 상한 장치가 아니다.

이미 승인된 live 계획에 따라 구현·planning 경로를 확인하고, 실제 모델 호출 수를 결과에 별도로 기록한다. 추가 리뷰 모델 요청을 피하기 위해 [PR 리뷰 backend](../reference/pr-review-backend.md)는 `off`로 설정했다. `observe`도 모델을 호출한다.

### 승인된 대상과 쓰기 범위

| 항목 | 승인 범위 |
| --- | --- |
| 저장소 | `RefinedStone/codex-exec-loop` |
| 실행본 | `b7de0bcdc2411f90ee33768fa23f988eba598374`의 Linux `akra`·`akra-gateway`. 고정 바이너리·hash·검증 범위는 아래 경합 수정 기록 참조. Desktop 코드는 `ad5f7509` 대비 변경 없음 |
| 실행 환경 | Linux에서 같은 사용자로 Desktop·Gateway·확인 helper 실행, 사람이 사용할 수 있는 GUI |
| 작업공간 | Linux 내부의 독립 clone 1개, 위 프로세스가 공유하는 시험 전용 `AKRA_HOME` |
| 시험 base | `codex/issue-2555-live-base-20261122`를 `3f437930`에서 생성하고 `ad5f7509`를 거쳐 `b7de0bcdc2411f90ee33768fa23f988eba598374`로 fast-forward했다. Linux 시험 clone도 같은 SHA다 |
| 선택 대상 | 시험용 이슈 1건만 선택하는 `akra-ia4-5-live-20261122` 라벨을 생성하고 [#2644](https://github.com/RefinedStone/codex-exec-loop/issues/2644)에만 붙였다 |
| 이슈 | [#2644](https://github.com/RefinedStone/codex-exec-loop/issues/2644) 생성 완료. `docs/validation/issue-2555-live-smoke-20261122.md` 한 파일만 추가하는 기준 |
| PR | 실제 task branch를 push하고 시험 base 대상으로 PR 생성 목표 1회 |
| 상태 댓글 | 새 댓글 1개 및 같은 댓글의 상태 갱신. 전체 수정 횟수의 설정 상한은 없음 |
| 통합과 닫힘 | 운영·시험 base로의 결과 통합 0회, PR merge 0회, 이슈 close 0회 |

`parallel.integration_branch`는 시험 branch로 설정할 수 있다. [설정 계약](../reference/configuration.md)과 readiness는 해당 원격 branch의 사전 생성·fetch를 요구한다. 대상이 달라지면 frozen target 검사로 차단하며 운영 branch로 fallback하지 않는다.

유효 정책은 `propose`, 시험 선택 라벨 하나, 확인된 stable actor ID, `budget.max_attempts = 1`, 사용자 허가 `feedback.comment = true`, `feedback.labels = false`, `feedback.transition = false`, `close.when = "never"`로 제한한다. PR mode는 `required`, autonomous delivery opt-in은 끈 상태로 둔다. 공개 저장소 실행에는 별도의 parent-process public-repository opt-in을 확인한다.

live 최소 구성은 같은 Linux 사용자·`AKRA_HOME`·로컬 파일시스템과 IPC를 공유하는 Desktop, Gateway, confirmation helper와 작업공간이다. Desktop이 로컬 `akra-gateway desktop-confirmation-bridge`를 시작하고 Gateway가 자기 `AKRA_HOME/run`의 파일과 Unix socket을 검증한다. 해당 이슈의 Start는 승인된 시험 범위 안에서 실제 GUI의 native 확인 창을 거쳐 응답한다. 정식 경로의 확인과 OS가 물리적 클릭을 인증한다는 주장은 구분한다. Windows Desktop의 원격 host 조회 기능은 이 로컬 승인 경로를 제공하지 않는다.

WSLg의 `DISPLAY`와 display socket, Linux의 GitHub 계정 `RefinedStone` 및 Codex ChatGPT 로그인 상태를 확인했다. 최종 소스의 Linux Desktop 창 표시, 한글 font 보완과 trace 재시작, 시험용 0-turn thread의 UI 표시도 확인했다. 이 준비 확인은 native 승인이나 실제 모델 접근 성공을 뜻하지 않는다. native Desktop/Gateway 연결과 Propose·Start 확인, 실제 worker profile·모델 접근 권한, 설정을 덮어쓰는 환경변수, active direction·빈 task authority 및 PR 생성으로 작동할 CI·외부 리뷰를 실행 전에 점검한다. 자격 증명 원문은 출력하지 않는다.

### 예정된 관찰과 종료 기준

아래는 실행 결과가 아닌 확인 계획이다. `$LiveWorkspace`는 허가받은 시험 clone 경로다.

```powershell
akra kernel query planning --workspace "$LiveWorkspace" --json
akra kernel query issue_automation_policy --workspace "$LiveWorkspace" --json
akra kernel query issue_automation --workspace "$LiveWorkspace" --json
akra kernel query parallel --workspace "$LiveWorkspace" --json
akra kernel query issues --workspace "$LiveWorkspace" --json
```

수집은 `akra kernel issue sync_issues`로, 실제 Start 승인은 Desktop native 확인으로 진행한다. 준비 조건이 충족된 뒤 같은 Gateway에 `akra kernel parallel enable_parallel`을 보내거나 Desktop의 명시적 시작 버튼으로 실행한다. 하위 폴더·linked worktree에서 접속해도 명령은 대표 checkout의 풀을 제어한다. Start 승인 자체는 병렬 풀을 켜지 않는다. 상세 query에는 목록이 반환한 전체 `IssueEventReference`를 사용한다.

성공 기준은 실제 task 하나, 지정 파일만의 변경, exact active validation lineage의 PR URL, 시험 base의 일치, 동일 status comment ID와 확정 journal이다. PR 사람 리뷰를 승인하지 않고 결과 통합을 수행하지 않으므로 integration 기록과 `Done`을 요구하지 않는다. 외부 이슈는 Open으로 남아야 한다.

관찰 뒤 `akra kernel parallel disable_parallel --workspace "$LiveWorkspace" --json`로 병렬 풀을 중지하고, `akra kernel thread stop_all --workspace "$LiveWorkspace" --json`로 남은 실행을 중단한다. 이 명령은 실행 종료 수단이며 모델 호출 수의 사전 상한이 아니다. 실제 호출 수·댓글 수정 수·게시 영수증·PR URL을 결과에 별도로 기록한다.

## 검증 기록 — 소스별 자동 회귀와 live 준비 상태

### 초기 결합 회귀 — `3f437930`

| 항목 | 기록 |
| --- | --- |
| 실행일 | 2026-11-22 (Asia/Seoul) |
| 검증 코드 SHA | `3f437930f45d925372448cf6b4955411fbeb2b64` — 실행한 소스 14개 파일의 SHA-256 일치를 확인한 뒤 commit. 이 검증 문서는 후속 문서 commit에 포함 |
| 선행 기준 SHA | `0eadc69069f0183a9ea1bd6025086514f04c28b0` — [PR #2642](https://github.com/RefinedStone/codex-exec-loop/pull/2642) |
| Linux 환경 | 위 WSL Ubuntu 환경. Windows PATH를 상속하지 않는 Linux 전용 PATH와 독립 Linux clone·target 사용 |
| Linux 실행 바이너리 | `/home/akra/.cache/akra-validation/codex-exec-loop-target/debug/deps/codex_exec_loop_native-dda04b32f448d3e9` |
| Linux 결합·관련 회귀 | **31 passed, 0 failed, 0 ignored**, 시험 33.70초. 새 결합 7개·새 DB 3개 포함. 아래 명령의 겹치는 필터는 한 시험으로 실행 |
| Windows DB 회귀 | `cargo test --locked --all-features --lib sqlite_issue_automation_store::execution::tests` — **15 passed, 0 failed** |
| planner 결과 | `node scripts/agent-plan.mjs` → Full. `git diff --check`와 `powershell -NoProfile -File scripts/check_native_pr.ps1 -Mode Full` 모두 성공 |
| Windows Full 세부 | native contracts, build·check·clippy, Desktop 검사 성공. Vitest **950 passed, 7 skipped**; 기존 skip 유지 |
| UI E2E — 기존 검증 SHA | `3f437930`에는 자체 UI 변경이 없었다. 선행 [#2554 검증](desktop-issue-execution-tracking.md)의 전체 25 spec 통과와 기존 Windows terminal skip 1개를 재사용했다. 후속 Gateway 제어 UI는 위 별도 기록으로 검증한다 |
| 포맷·차이 검사 | `cargo fmt --all`, `git diff --check` 성공 |
| Windows native 결합 suite | Unix-only publication lease 때문에 `#[cfg(unix)]`로 제외. Windows에서 이 7개가 통과했다고 보고하지 않음 |
| OS process kill 시험 | 미실시 |

Linux에서 실행한 결합·관련 회귀 명령:

```bash
cargo test --locked --all-features --lib -- \
  issue_automation_flow \
  sqlite_issue_automation_store::execution::tests \
  work_order_permit_wait_does_not_starve_an_unrelated_task \
  work_order_revoked_after_cherry_pick_preserves_its_result_and_allows_unrelated_integration \
  trusted_integration_is_recorded_once_and_later_contract_recovery_stays_done \
  integrated_queue_recovery_is_blocked_even_with_zero_execution_cap \
  integrated_observation_recovery_requires_zero_execution_cap_and_preserves_attestation \
  post_merge_contract_opens_human_recovery_and_never_reimplements \
  external_closure \
  --nocapture
```

실행 환경의 PATH는 `/home/akra/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin`으로 제한했다. Windows PATH를 그대로 상속하면 Git의 trusted-executable 경로 검증이 Windows mount를 반복 조회해 지연됐으므로, Linux 실행 환경만 바로잡고 production 경로 검증은 바꾸지 않았다.

원본 결과 로그는 실행 호스트의 `C:/dev/akra-queue/analysis/issue-2555-flow-fixed.log`와 `C:/dev/akra-queue/analysis/issue-2555-final-full-gate.log`에 남겼다. PR의 최종 commit에 대한 공식 CI 결과는 PR 본문에서 연결한다.

자동 회귀 결과와 live 결과는 따로 기록한다. 자동 회귀 통과만으로 실제 native 승인, 모델 수행, GitHub 전달 또는 OS crash 복구까지 성공했다고 결론 내리지 않는다.

### 제어 기능 소스 검증 — `ad5f7509`

이 기록은 위 초기 `3f437930` 결합 회귀와 `896781dc` Gateway 제어 검증 이후의 실행 소스를 대상으로 한다. 관련 회귀, 전체 CI 실패와 후속 수정의 로컬 검증 결과를 구분한다.

| 항목 | 기록 |
| --- | --- |
| 최종 Linux 검증 소스 | `ad5f7509f318c2272c7e6becd9f310342c7769c7` |
| Linux 바이너리·관련 회귀 | `akra`·`akra-gateway` build 성공. 관련 시험 **43 passed, 0 failed, 0 ignored** |
| Linux Desktop | check·build 통과. 같은 소스의 Desktop이 WSLg에서 창을 표시함 |
| 고정 실행 바이너리 | `/home/akra/.cache/akra-validation/issue-2555-live-binaries-20261122/ad5f7509f318/akra`, 같은 디렉터리의 `akra-gateway` |
| 바이너리 hash manifest | `C:/dev/akra-queue/analysis/issue-2555-live-session.json`의 `validated_source_sha`, `akra_binary`, `gateway_binary`, `binary_sha256` 참조 |
| 시험 base·clone | 둘 다 `ad5f7509f318c2272c7e6becd9f310342c7769c7`로 고정 |
| Linux 전체 CI | [run 45022050145](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45022050145): **5064 passed, 1 failed, 14 ignored**. `the_spec_entry_of_a_command_carries_its_meaning_and_its_input`의 예상 command family 목록에서 `parallel`이 빠진 실패 |
| 후속 기대값 수정·로컬 검증 | 예상 목록에 `parallel` 한 줄을 추가했다. 관련 명세 시험 **2개 통과**, planner가 지정한 **Windows Full 재실행 통과(exit 0)** |
| 후속 CI | `8d743a30dc1d0323456c18348ab70f012c942928`의 [run 45024197035](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45024197035) 통과(attempt 2). 첫 시도의 기존 app-server 종료·응답 관찰 경합 시험 1건 실패 뒤, 코드·시험을 바꾸지 않은 같은 head의 1회 재실행이 통과. 이후 경합 수정의 CI와 구분 |

최종 Linux build·관련 시험·Desktop 결과는 `C:/dev/akra-queue/analysis/issue-2555-final-linux.log`, 전체 CI 실패는 `issue-2555-ci-45022050145-rust.log`, 기대값 수정 후 로컬 Full 결과는 `issue-2555-ci-spec-fix-full.log`에 남겼다. 이 고정 실행본의 증거는 `ad5f7509`까지이며 후속 commit의 Linux 검증을 대신하지 않는다.

### 2026-11-22 실제 시험 준비 결과

- 시험 이슈 [#2644](https://github.com/RefinedStone/codex-exec-loop/issues/2644), 시험 base branch와 단일 선택 라벨을 생성했다.
- Linux 독립 clone과 시험 전용 `AKRA_HOME`을 만들고 사용자 정책 파일을 owner-only 권한으로 저장했다.
- 기존 검증 코드 `3f437930`의 준비용 CLI로 기본 direction을 시험 문서 범위로 갱신했다. planning revision은 4이며 active/proposed/skipped task가 모두 비어 있다. 준비 작업은 모델을 호출하지 않았다.
- Linux에서 repo-local `RefinedStone` identity의 `scripts/gh-akra.sh auth write-status`와 시험 base로의 `git push --dry-run`을 통과했다.
- `feature_bindings`의 PR review는 `off`이고, 정책 query는 `ready`를 반환했다. Propose 상한·선택 라벨·trusted actor·1/1/1 예산·댓글만 허용·close never가 사용자 파일에서 해석됐음을 확인했다.
- 후속 Gateway 제어를 포함한 `ad5f7509`로 Linux 실행 바이너리와 Desktop을 갱신했다. 시험 base와 clone도 같은 SHA로 맞췄으며, 준비용 `3f437930` CLI와 실제 실행용 고정 바이너리를 구분했다.
- 이 시점의 실제 모델 턴·시험 PR·상태 댓글은 0회다. native Start 확인과 실제 모델 전달 흐름은 아직 미실행이다.

후속 로컬 원본 로그: `C:/dev/akra-queue/analysis/issue-2555-parallel-control-rust-r2.log`, `issue-2555-parallel-control-desktop-r2.log`, `issue-2555-parallel-control-full-r4.log`. 원본 로그 경로는 실행 호스트 기준이며 공식 CI 링크는 PR 본문에 남긴다.

### 경합 수정과 실제 시작 질문 복구 — `b7de0bcd`

| 항목 | 기록 |
| --- | --- |
| 검증 코드 SHA | `b7de0bcdc2411f90ee33768fa23f988eba598374` — 검증한 Rust 4개 파일만 변경한 commit |
| Windows 관련 회귀 | `issue_coordinator::tests issue_observer_tests::`: **107 passed, 0 failed, 0 ignored**. 새 gate 회귀 4개 포함 |
| Windows Full | planner가 지정한 Full 통과. native contracts·build/check/clippy·Desktop check, Vitest **963 passed / 7 existing skipped** |
| Linux 관련 회귀 | **139 passed, 0 failed, 0 ignored**, 테스트 73.17초. coordinator·observer·실제 SQLite/Git flow·실행 DB·병렬 제어·대표 checkout 범위 |
| 정식 Linux 실행 파일 | 정확한 commit의 clean checkout에서 `cargo build --locked --bin akra --bin akra-gateway` 성공. 검증한 4파일의 정규화 SHA와 commit Git blob·checkout SHA 일치 |
| 동결 경로 | `/home/akra/.cache/akra-validation/issue-2555-gate-binaries-20261122/b7de0bcdc241`. 디렉터리 0700, 실행 파일 0500 |
| Desktop | `ad5f7509` 대비 코드 변경 0. 기존 check/build 및 전체 E2E 증거 유지. 같은 Desktop에 수정 Gateway를 연결 |
| 시험 base·clone | `b7de0bcdc2411f90ee33768fa23f988eba598374`로 fast-forward, clean 상태. GitHub identity·쓰기 자격·push dry-run과 Codex ChatGPT 로그인 확인 |
| 실제 공개 query | planning revision 4, active/proposed/skipped task 각각 0, pool false. #2644의 열린 Start 질문 **1개**, 결정 이력 0개 |
| 실제 모델·전달 | 조회 시점의 native Start 응답·모델 턴·시험 PR·상태 댓글은 모두 **0회**. 실제 모델 전달 흐름은 미완료 |

추가 회귀는 `collector_tick_waits_for_tracking_and_opens_one_start_decision`, `cancelled_tick_wait_does_not_recover_pending_controls`, `cancelled_tracking_wait_does_not_recover_pending_controls`, `poisoned_gate_is_an_error_for_tick_and_tracking`이다. 실제 gate 경합을 채널로 관찰한 뒤 holder를 해제하며, 질문 1개 유지와 대기 중 취소의 저장·보고·source·execution 효과 0을 검사한다. timeout은 양의 handshake가 누락된 경우의 교착 감지에만 사용한다.

Linux의 첫 실행에는 Windows PATH 54개가 상속돼 파일 검증이 오래 걸렸다. 138개 통과·1개 미완료 상태에서 시험 프로세스만 정상 종료하고 환경 중단으로 보관했다. 소스는 바꾸지 않고 Linux 전용 PATH로 재실행해 139개가 모두 통과했다. 최종 빌드도 같은 Linux 전용 PATH를 사용했다.

최종 로그는 `C:/dev/akra-queue/analysis/issue-2555-gate-serialization-rust.log`, `issue-2555-gate-serialization-full.log`, `issue-2555-gate-serialization-linux-r2.log`에 있다. [재현·복구 메타데이터](issue-2555-live-gate-starvation.json)에 두 실행 파일 SHA-256과 수정 전후의 관측을 기록했다. 임시 진단 Gateway는 종료했고 실제 창에는 정식 동결 실행본을 연결했다. 이 query 확인을 실제 사람 승인이나 모델 → PR → 댓글 성공으로 확대하지 않는다.

### 성공 직후 프로세스 종료 회귀 — 2026-11-26

| 항목 | 기록 |
| --- | --- |
| 기준 SHA | `d960d7d62e810f8cd94c7aa78918b0ba0104b6d6` (`prerelease`). 변경은 시험 코드 4개 파일과 이 문서뿐이다 |
| OS | macOS 26.5 (Darwin 25.5.0), arm64, rustc 1.95.0 |
| 실제 구성요소 | 임시 Git 저장소, 파일 기반 SQLite issue store·planning authority, `AkraIssueWorkExecutionAdapter`, `ParallelModeService`, collection·triage·decision·coordinator·feedback 서비스, 파일 기반 publication journal |
| 가짜 구성요소 | 이슈 출처, 댓글 provider (`CreateComment`마다 새 댓글을 만든다), GitHub PR adapter, 사람 확인 capability, 고정 tick 시각. 종료는 외부 효과 뒤의 `panic`으로 흉내 낸다 |
| 명령 | `cargo test --locked --all-features --lib issue_automation_flow` |
| 결과 | **9 passed, 0 failed, 0 ignored**, 117.71초. 새 시험 2개 포함 |
| 제품 코드 변경 | 없음. 두 경계 모두 중복이 생기지 않았다 |
| 실제 모델 실행·실제 PR·실제 댓글 | **0회**. 실제 한 사이클은 사용자의 진행 결정을 기다린다 |
| 미검증 | OS process kill, SQLite commit 도중 종료, 실제 GitHub 응답 유실, Windows (이 suite는 `#[cfg(unix)]`) |

## 후속 작업 시 실제 준비 상태 재확인

새 기능 작업과 별도로, 이미 승인받은 #2644 단일 시험을 공개 query와 GitHub 읽기로 다시 확인했다. 실행 범위와 고정 실행본은 확장하지 않았다.

| 확인 | 관찰 |
| --- | --- |
| 시험 clone | `b7de0bcdc2411f90ee33768fa23f988eba598374`, clean |
| 원격 시험 base | `codex/issue-2555-live-base-20261122`가 같은 `b7de0bcd`를 가리킴 |
| 실행 바이너리 | `akra` SHA-256 `d8a0200810b0f208c540bb9aa72d004f25167b9bded365af8b28b547c685af24`, `akra-gateway` `134827655dbf5b3086ca80df93caa406ece0f88c9bf598ebdab27de05a640342`; 기존 동결 manifest와 일치 |
| Desktop 소스 | `ad5f7509f318c2272c7e6becd9f310342c7769c7`, clean. 기존 Electron과 빌드 entry 존재 |
| Planning | revision 4, active/proposed/skipped task 각각 0 |
| 실행 풀 | disabled, 실행 worker 0 |
| #2644 | Open, 단일 시험 라벨 유지, 연결 PR absent, GitHub 댓글 0 |
| 시작 결정 | 열린 Start 질문 1개, revision 1, 결정 이력 0. 자동화 제어는 held/cancelled/taken_over 모두 false |
| 정책 | Propose, 단일 선택 라벨, trusted actor 고정, max_running/max_started_per_day/max_attempts 각각 1 |
| 외부 쓰기 | comment=true, labels=false, transition=false, close=never |
| 추가 리뷰 모델 | `feature_bindings.pr_review = off` |

이번 확인은 `planning`, `parallel`, `issue_automation_policy`, `issue_automation`, 정확한 IssueEventReference의 `issue`, `feature_bindings` 공개 query를 사용했다. GitHub의 #2644 상태·댓글 수와 시험 remote ref도 다시 읽었다. task 등록, proof 생성·주입, DB 수정, 모델 요청, 시험 PR·댓글 쓰기는 수행하지 않았다.

**제품의 정식 native Start 응답과 아래 런타임 경고 확인이 남아 있다.** 기존 모델 실행 허가는 이미 확보되어 있으므로 일반 작업 허가를 다시 요구하지 않는다. 그러나 제품이 보관하는 Human 결정은 private helper의 정식 native 확인으로 생성해야 한다. 그 후 같은 시험 workspace의 실행 풀을 명시적으로 켜고, 위 성공 기준의 task·단일 파일·시험 base PR·같은 상태 댓글과 실제 모델 호출 수를 확인한다. 자동 승인 대역이나 이번 개발 PR을 실모델 시험의 성공 증거로 세지 않는다.

이 재확인 기록은 #2555의 실제 모델 → PR → 댓글 완료를 의미하지 않으며, 해당 이슈는 실제 증거가 확보될 때까지 열린 상태로 유지한다.

### 실제 Desktop 창 재개와 런타임 경고

동결한 Linux Desktop을 같은 Linux 사용자와 시험 전용 `AKRA_HOME`·`AKRA_DESKTOP_USER_DATA`로 다시 실행했다. launch 인수에 sandbox 해제·remote debugging 옵션을 추가하지 않았으며, Windows의 WSLg 창을 직접 캡처해 화면 표시를 확인했다. 실행한 Electron 프로세스들이 정확한 시험 profile의 파일을 열고 있는 것도 확인했다. Native Start 응답이나 실행 풀 활성화는 수행하지 않았다.

화면에는 생성 프로토콜 기준 `codex 0.159.0`과 연결 Gateway의 `codex-cli 0.146.0` 사이 버전 경고가 표시됐다. 창 표시를 실제 모델 호환성 검증으로 해석하지 않으며, live 실행 전 이 경고의 영향과 정식 native Start 경로를 확인해야 한다. 이 준비 과정에서 모델 요청이나 시험 PR·댓글 작성은 하지 않았다.

## Windows native 결합 회귀 재검증 (#2857, 2026-11-27)

기준은 `prerelease@fa72e69a868826e6b61ec0f935260d1e5b9344a8`와 이 PR의 Windows fixture·inventory
변경이다. 실행 환경은 Windows `10.0.26200` x64, Rust `1.95.0-x86_64-pc-windows-msvc`,
`--all-features --target x86_64-pc-windows-msvc`다. production 서비스와 Windows publication
journal/lease는 이 변경에서 수정하지 않았다.

- 최초 `scripts/check_native_pr.ps1 -Mode Full` 실행에서 Desktop typecheck, **1428 passed /
  8 skipped**, build 및 기존 Windows validator **12 passed / 0 failed / 0 ignored**가 통과했다.
- Propose 첫 실행은 **8 passed / 1 failed / 0 ignored**, 338.18초였다. ACK 유실 회귀가 Unix의
  `issue-publish-intents.lock`을 Windows에도 요구한 fixture 오류였다. 실제 Windows reservation은
  `issue-publication-lease.db`다. 따라서 이 최초 Full 실행을 통과라고 표시하지 않는다.
- Windows에서는 실제 SQLite lease 파일 존재와 금지된 legacy lock 파일 부재를 확인하도록 고쳤다.
  Unix의 기존 lock 파일 단언은 유지했다. 이후 Windows Propose는 **9 passed / 0 failed /
  0 ignored**, 294.92초로 모두 통과했다. 임시 저장소의 실제 Git/SQLite, task 등록·원격 효과 뒤
  ACK 유실·서비스 재조립·readback·permit·전달 차단 계약을 그대로 사용했다.
- 재검증은 `scripts/run_native_rust_contracts.mjs`로 Full inventory에서 Propose 이후의 동일 selector를
  실행한다. 변경하지 않은 Node 시험과 먼저 통과한 native 그룹을 전체 반복하지 않는다.
  재개한 64개 selector가 모두 통과했고, 이어서 Clippy `--locked --all-targets --all-features -- -D warnings`도
  exit 0(1분 45초)으로 완료했다. 최초 Full의 성공 범위와 이 재개 결과를 합쳐 확인했으며,
  최초 Full 명령 자체의 실패 기록을 성공으로 바꾸지 않는다.

Propose 9개만 재현하려면 다음을 실행한다.

```powershell
cargo test --locked --all-features --lib --target x86_64-pc-windows-msvc application::service::parallel_mode::tests::issue_automation_flow:: -- --nocapture
```

두 process-death 시나리오의 의도적인 panic 출력은 테스트가 붙잡는 장애 주입이다. 이 자동 회귀를
물리적 프로세스 강제 종료·전원 단절의 증거로 확대하지 않는다. 실제 모델 호출, native Start,
외부 GitHub PR·댓글 작성은 이 검증에서 하지 않았다. #2555의 실제 제품 흐름과 #2704/#2724의
실제 OS 승인·Credential Manager·Electron 원문 창 검증을 이 결과로 닫지 않는다.

338.18초와 294.92초는 이 호스트의 서로 다른 실행 결과이며 성능 개선 비율이 아니다. Windows의
이 결합 suite에 수분이 소요되는 관측은 #2864의 시험별 시간 분석 대상으로 남긴다.

첫 PR CI Scope 실행에서는 기존 native inventory 시험이 selector 수를 124개로 고정하고 있어
새 2개 selector를 포함한 126개 목록을 거절했다. 기대 수를 126개로 갱신하고 새 두 selector의
존재도 검사하도록 수정했다. 실제 CI Scope와 동일한 Node 시험 파일 7개는 **57 passed /
0 failed / 0 skipped**로 통과했다. 이 후속 수정은 정책 시험의 inventory 확인만 바꾸며
위에서 검증한 Rust 코드와 native 실행 목록은 그대로 유지한다.
