# Akra 커널 아키텍처 수정안

## 문서 상태

- 상태: **Approved architecture change, 구현 중**. 결정 KP-01~07 을 2026-11-07 에 승인했다 (§7). K0~K3 를
  완료했다 (K3 는 2026-11-10, §6). 이 문서는 목표 구조를 설명한다.
  현재 구조의 기준은 [architecture.md](../reference/architecture.md) 이다.
- 기준일: 진단(§3)은 2026-11-06, 진행 상태(§5.6, §6)는 2026-11-10 (Asia/Seoul)
- 기준 커밋: 진단은 `8e64736cdb2050da2b2ffbce06b231da86186753`, 진행 상태는 `80c11441` (`origin/prerelease`)
- 근거: 2026-11-06 코드 조사 4건 (§3). 불안정 테스트 조사 결과는 §3.5 에 있다.
- 관련 계획: [17 Akra Desktop](17-akra-desktop/README.md), [19 Akra 커널 성질 목표](19-akra-kernel-quality-goals.md),
  [20 Akra 관찰-제안 구조](20-akra-observe-and-offer.md) (일반 대화를 관찰하고 Akra 기능을 제안하는 전체 그림)
  (K4~K6 의 완료 기준이 되는 성질 P1~P5 와 진행 순서, 2026-11-09 제안)
- 목적: Akra 를 "사람이 TUI 로 조작하는 도구"에서 **사람과 AI 에이전트가 동등하게 쓰는 양방향 커널**로 바꾸는
  구조 수정안을 정한다.

## 1. 목표

1. 사람(TUI, Desktop)과 AI 에이전트(Codex, Claude)가 **같은 명령과 같은 이벤트**로 Akra 를 쓴다.
2. 에이전트가 Akra 의 기능을 호출하고, Akra 도 에이전트에게 알리고 깨운다 (양방향).
3. 화면이 없어도 커널이 돈다. 화면이 여러 개여도 상태와 루프는 하나이다.
4. 명령 하나의 정의에서 모든 입구(프로토콜, CLI, MCP, SDK, 스킬 문서)를 만든다. 스펙이 AI 에게 명확히 공개되고
   그대로 실행된다.

## 2. 배경

### 2.1 목표의 변화

Akra 는 TUI 중심으로 설계되었다 ([AGENTS.md](../../AGENTS.md): "Optimize the TUI/app-server flow first").
그 목표에서는 아래 선택이 맞았다. 새 목표에서는 제약이 된다.

- 런타임을 TUI 프로세스에서 실행한다.
- 조작자는 한 명이고, 열린 대화는 하나이다.
- 다른 입구(CLI, admin, Telegram)는 보조 기능이다.

### 2.2 참고한 기존 설계 (AI 이전부터 검증된 것)

| 사례 | 가져올 것 |
| --- | --- |
| Neovim, LSP/DAP | 헤드리스 core, 모든 화면은 RPC 클라이언트, 양방향 요청, 기능 협상 |
| Xi editor (실패 교훈) | core 와 화면 사이의 메시지를 너무 잘게 나누면 복잡도와 지연이 커진다. 화면용 메시지는 굵게 설계한다 |
| Docker, Kubernetes | 상주 데몬이 유일한 제어면. CLI 와 화면은 동등한 클라이언트. watch + 커서, reconcile 루프, RBAC, admission(정책 검사), 감사 기록 |
| Temporal, Erlang OTP | 오래 도는 실행에 signal·query 보내기, 우편함, 감독과 재시작 |
| VS Code 명령, Blender operator | 모든 화면 동작은 등록된 명령이다. 화면, 단축키, 스크립트가 같은 명령을 부른다 |
| Google AIP-151, Stripe API | 오래 걸리는 연산은 Operation 으로 반환한다. 멱등 키, 이벤트, 버전 |
| git plumbing / porcelain | 기계용 안정 인터페이스와 사람용 출력을 분리한다 |

AI 때문에 새로 필요한 것은 네 가지뿐이다: 출력의 토큰 효율, 자연어 안내(스킬 문서, 복구 방법이 적힌 오류),
예산과 재귀 깊이 제한, 출력 속 지시문 경계(프롬프트 주입 방지).

### 2.3 용어: 현재의 `core`

`src/core` 는 Akra 고유의 이름이다. 코드와 문서는 이것을 **"비즈니스 헥사곤 밖의 클라이언트 런타임"** 으로
정의한다 (`src/core/mod.rs` 주석, [architecture.md](../reference/architecture.md) 계층 표). 일반적인 헥사고날
용어에서 core 는 헥사곤 안쪽(application + domain)을 뜻한다. 이 문서는 혼동을 막기 위해 현재의 `src/core` 를
**"클라이언트 런타임(현재 core)"** 이라고 쓴다.

## 3. 진단 (2026-11-06 코드 조사)

### 3.1 문제없음: 좋은 기반

| 항목 | 근거 |
| --- | --- |
| core, application, domain 이 TUI·터미널 라이브러리를 import 하지 않는다 | `adapter::inbound`, `crossterm`, `ratatui` import 0건 |
| 화면 상태(초안, 스크롤, 오버레이, 선택)는 이미 TUI 에 있다 | `src/adapter/inbound/tui/app.rs:341-391`, `shell_chrome.rs:86-97` |
| 상태와 명령 처리는 클라이언트 런타임에 있다. TUI 는 상태를 직접 바꾸지 못한다 | `CoreController`(`src/core/app/controller.rs:39`), `AppState`(`controller/state.rs:16`), 경계 테스트 `client_runtime_state_mutation_stays_behind_the_runtime_driver` |
| 클라이언트 런타임은 터미널 없이 만들 수 있다 (`Send`, std 스레드 기반, tokio 불필요) | `core_effect_worker.rs:2`, `:109-115` |
| 공유 서비스 안의 안전 검사는 모든 입구에서 같다 | 초기화 guard(`repair/reset.rs:89-117`, `:160-197`), 초안 승격의 기준 데이터·revision 검사(`authoring/init.rs:303-340`) |
| 엔진은 outbound 포트 3개로만 부른다 | `PlanningWorkerPort`, `ParallelAgentWorkerPort`, `InteractiveTurnRuntimePort` |

### 3.2 제약: 새 목표에서 드러난 것

**제약 1. "열린 대화 1개, 실행 중 턴 1개" 전제**

- `AppState` 의 필드는 `current: Arc<AppSnapshot>` 하나이다. 나머지 상태는 `CoreController` 의 reducer 7개에
  있다 (`controller.rs:40-47`).
- 대화를 여는 명령이 공유 런타임을 지운다. `admit_conversation_load`(`conversation_turn_reducer.rs:171-209`)와
  `invalidate_conversation`(`conversation_runtime.rs:445-453`)이 진행 중 턴, 승인, 턴 종료 후 상태를 지우고 정지
  효과까지 낸다. 클라이언트가 여럿이면 한 클라이언트의 대화 선택이 다른 클라이언트의 턴 상태를 지운다.
- 단수형 상태: `TurnStreamState`(`turn_stream.rs:30-41`), `ConversationSnapshot`(`snapshot.rs:11`),
  `planning_runtime`(`projection.rs:19`).
- 조회 모델 슬롯이 하나이다. 마지막 요청이 이긴다 (`read_model_reducer.rs:8-15`).

**제약 2. 결정 규칙과 타이머가 TUI 에 있다**

상태와 "명령이 오면 무엇을 하는가"는 클라이언트 런타임에 있다. **"언제, 어떤 명령을 보낼 것인가"** 는 TUI 에 있다.

| 항목 | 위치 |
| --- | --- |
| 턴 종료 감지 → `EvaluatePostTurn` 생성 | `tui/app/conversation_runtime.rs:1064-1104` |
| 평가 입력(최근 사용자 메시지, 최근 응답)을 TUI 대화 기록에서 꺼냄 | `post_turn_execution.rs:108-114` |
| 평가 결과가 자동 제출이면 `SubmitTurn` 전송 | `conversation_runtime.rs:707-741`, `turn_submission_runtime.rs:83-160` |
| 턴 옵션과 슬롯 lease 넘기기를 TUI 상태로 만듦 | `turn_submission_runtime.rs:182-233` |
| 병렬 tick 타이머 | `parallel_mode.rs:432-445` |
| GitHub 리뷰 폴링 타이머 | `shell_runtime.rs:226` |
| 병렬 제어부 투영을 클라이언트 런타임으로 되돌려 보냄 | `parallel_mode.rs:447-454` |
| 시작 순서 (`RunStartupChecks` → `LoadSessionCatalog`) | `app_runtime.rs:2385`, `:2406` |
| 구동 루프: 최대 100ms 마다 대기 입력 조회 (polling). 깨우기 API 없음 | `ratatui_frontend.rs:96-114`, `driver.rs:79-82` |

TUI 가 없으면 이 루프가 멈춘다. 클라이언트가 여럿이면 중복될 수 있다.

**제약 3. 결과 전달이 화면 하나 기준**

결과는 호출자 한 명에게 반환값으로만 간다 (`native_client_port.rs:21-25`). 스트림 이벤트는 증분이고
(`turn_stream.rs:706`), 대화 기록은 TUI 의 `ConversationViewModel` 이 쌓는다. 계약 타입에 직렬화(serde)가 없다.

**제약 4. 명령에 화면 흐름 단계가 섞였다**

명령 enum 의 실제 이름은 `AppCommand` 이다 (`src/core/app/command.rs:14`, 32개).

| 분류 | 명령 |
| --- | --- |
| 도메인 | `RunStartupChecks`, `LoadSessionCatalog`, `RenameSession`, `ResetPlanningWorkspace`, `StageSimplePlanningDraft`, `PromoteSimplePlanningDraft`, `MutatePlanningEditor`, `SubmitQueueMutation`, `SubmitTurn`, `SteerTurn`, `RequestStopAllSessions`, `SubmitApprovalDecision`, 설정 명령 5개 |
| 도메인 (조회) | `LoadReviewCenter`, `LoadQueueAuthority`, `LoadDirectionsMaintenance`, `RefreshPlanningRuntime` |
| 도메인, 내부 루프 | `EvaluatePostTurn`, `PollGithubReview`, `ResolvePostTurnContinuation` — 클라이언트가 아니라 스케줄러가 보내야 한다 |
| 화면 흐름 | `LoadConversation`, `InvalidateConversationLoad`, `LoadParallelPeekConversation`, `StagePlanningEditor`, `LoadSimplePlanningEditor`, `PrepareManualPrompt`, `CancelManualPromptPreparation` |

에이전트에게는 한 번에 끝나는 명령이 필요하다. 예: `PrepareManualPrompt` → 제출의 2단계를 하나로 합친 명령.

### 3.3 결함: 목표와 관계없이 고칠 것

모두 검사가 공유 서비스가 아니라 어댑터나 프로세스 메모리에 있어서 생겼다.

| ID | 내용 | 확실성 | 근거 |
| --- | --- | --- | --- |
| K-D1 | **초기화 확인 절차가 입구마다 다르다.** TUI·admin HTML 은 확인을 받는다. CLI `akra reset all`, Telegram `/reset_all`, JSON `POST /api/planning/reset` 는 확인 없이 directions, prompts, drafts 를 지운다. 문서의 "explicit reset confirmation" 과 맞지 않는다 | 확정 | `tui/app/planning/controller.rs:408-417`, `cli.rs:140-147`, `telegram_bot/message.rs:148-154`, `admin_api/api.rs:770-786`, `docs/plan/14-codex-for-oss-application.md:27-28` |
| K-D2 | **admin 작업 수정에 revision 검사가 없다.** 폼을 연 사이에 작업자가 바꾼 값(예: `blocked_by`)이 저장 때 예전 값으로 덮어써진다. TUI 큐 변경은 `expected_planning_revision` 을 검사한다 | 확정 | `application/service/planning/admin/crud.rs:262-279`, `task_mutation.rs:316-362`, `:465-478` |
| K-D3 | admin 의 병렬 정지가 다른 프로세스(TUI)의 루프에 전달되지 않을 수 있다. 실행 상태가 프로세스 메모리에만 있다 | 추정 | `automation_guard.rs:4-7`, `control_plane/mod.rs:957-1032`, `parallel_mode/admin.rs:135-151` |
| K-D4 | 초기화 감사 기록이 없다. admin 작업 삭제는 공통 commit audit 경로를 거치지 않는다 | 확정 (경미) | `task_mutation.rs:343-347`, `documents.rs:167-180` |
| K-D5 | 데몬과 TUI 를 함께 실행하면 PR 검증 스케줄러와 병렬 제어가 두 번 돌 수 있다 | 추정 | `production.rs:614-628` |
| K-D6 | **macOS 에서 정상 종료된 하위 프로세스(git 등)가 실패로 바뀐다.** 자식이 종료·회수된 뒤에도 프로세스 그룹에 `SIGKILL` 을 보내고, 결과가 ESRCH 일 때만 성공으로 본다. macOS 는 이때 가끔 EPERM 을 돌려준다. git 어댑터는 이 실패를 "결과 없음"으로 바꾼다. 병렬 하네스의 무작위 오류("git repository is unavailable", "worktree list reload failed" 등)의 원인이다 | 확정 (제품 버그, 신뢰도 높음) | `src/subprocess.rs:1301`, `:1633`, `:234`, `:375-389`, `adapter/outbound/git/parallel_mode_runtime.rs:262-311` |
| K-D7 | 테스트가 하위 프로세스 제한 시간(`CODEX_EXEC_LOOP_SUBPROCESS_TIMEOUT_SECS=1`)과 `HOME` 을 프로세스 전체에 바꾸면서 공용 환경 잠금을 잡지 않는다. 동시에 도는 다른 테스트가 1초 제한을 받는다 | 추정 (테스트 하네스) | `adapter/outbound/github/automation.rs:3311`, `:3304`, `:3020`, `test_utils.rs:18`, `subprocess.rs:1259-1265` |

### 3.4 정리

- 구조는 근본적으로 잘못되지 않았다. 경계 분리가 깨끗하다.
- 새 목표의 핵심 장벽은 **"조작자 1명, 열린 대화 1개"** 전제와 **TUI 에 있는 결정 규칙·타이머**이다.
- 결함 K-D1~D5 는 모두 명령 경로가 여러 개라서 생겼다. **명령 경로를 하나로 통일하는 커널 방향이 이 종류의
  결함을 구조적으로 막는다.**
- 결함 K-D6 은 구조와 관계없는 하위 프로세스 계층의 버그이다. 병렬 하네스를 커널의 기반으로 쓰기 전에 고친다.

### 3.5 불안정 테스트 조사 결과

2026-11-05 기준선: 전체 테스트 3,126개 중 25개 실패, 실행마다 다른 테스트가 실패했다. 2026-11-06 재조사 결과는
다음과 같다 (release 빌드, 샌드박스 밖, macOS).

| 실행 | 모드 | 결과 |
| --- | --- | --- |
| `parallel_mode` 4회 | 기본 병렬 (8 스레드) | 매번 2~4건 실패. 실패 집합이 실행마다 다름 |
| `parallel_mode` 1회 | `--test-threads=1` | **575건 모두 통과** (약 2,132초) |
| Telegram 3건, admin 1건 단독 | 반복 실행 | 모두 통과 |
| outbound 어댑터 + admin_api (695건) 3회 | 기본 병렬 | 매회 1건 실패 (Telegram 토큰 마스킹 테스트, 1초 제한 초과) |

| 실패 묶음 | 판정 | 신뢰도 |
| --- | --- | --- |
| `parallel_mode` (pool, lease, distributor, blocked, recovery, orchestrator_loop, pr_validation_e2e) | 제품 버그 K-D6. 전체 실행에서는 K-D7 이 실패 수를 늘림 | 높음 |
| control_plane coverage 테스트 2건 | 테스트 하네스 (5초 대기 제한, 부하가 클 때만) | 중간 |
| Telegram `execute_json_request_*` | 테스트 하네스 (1초 제한, 동시 실행 의존). 가끔 K-D6 | 중간 |
| admin PR 검증 보드 | 재현 안 됨. 테스트 하네스로 추정 | 낮음 |

근거:

- 독립 재현 (`process_group(0)` → 회수 → 그룹 `SIGKILL`): 16 스레드 3,200회에서 EPERM 14~15건(약 0.45%),
  git 종료 코드는 모두 0. 1 스레드 1,900회에서는 0건.
- 진단 로그를 넣은 빌드에서 git 관련 실패 테스트마다 그룹 kill 의 EPERM 이 기록되었다.
- 공유 임시 디렉터리 가설은 기각했다. 테스트 저장소와 기준 데이터 저장소는 테스트마다 고유하다
  (`parallel_mode/tests.rs:103-167`, `workspace_paths.rs:304-313`).
- 실제 앱도 git 을 동시에 실행하므로 사용자 환경에서도 같은 무작위 실패가 날 것으로 본다 (추정).
- 동시 실행에서 EPERM 이 나오는 정확한 커널 원인과, 보안 소프트웨어의 영향은 확인하지 못했다 (추정).

수정 방향: 자식이 정상 종료·회수된 뒤의 그룹 kill 결과가 EPERM 이면 macOS 에서는 무시하거나 그룹이 비었는지
다시 확인한다. K-D7 은 해당 테스트가 공용 환경 잠금(`process_environment_mutex`)을 잡도록 고친다.

**K-D7 결과 (2026-11-07):** 1초 제한을 거는 테스트는 이미 잠금을 잡고 있었다. 누수는 잠금 없이 같은
환경 변수를 읽는 다른 테스트의 하위 프로세스 쪽이었다. 테스트 전용 제한 시간을 프로세스 환경 변수 대신 그
테스트의 스레드에만 두도록 고쳤다 (`TestSubprocessTimeoutGuard`). `HOME` 변경은 git 테스트가 저장소마다
사용자 정보를 직접 설정해서 영향이 작다고 보고 고치지 않았다.

## 4. 목표 구조

### 4.1 구성

```text
 driving (커널을 움직이는 쪽)                               driven (커널이 움직이는 쪽)
 TUI, Desktop, CLI(생성), MCP(생성),                       AgentRuntimePort → codex app-server / claude
 SDK(생성), Telegram, admin, 에이전트                       (턴 시작 = 에이전트 깨우기)
        |                                                  EventLogPort (영구 기록, 커서)
        v                                                  NotificationPort (구독자 분배)
 [응용 프로토콜: 명령 / 조회 / 구독 + 호출자 + 멱등 키]       Git, GitHub, CI, DB 엔진, 브라우저 엔진
        |                                                  계획 기준 데이터 (SQLite)
        v                                                          ^
 커널 (데몬)                                                       |
   application: 세션 상태 머신(thread 별) + 정책(결정 규칙, 권한, 예산)
                + 연산 관리 + 승인 + 주의(우편함, 깨우기)  ---------+
   domain: 계획, 작업, 슬롯 + 호출자, 연산, 승인, 우편함
   런타임 호스트: 구동 루프, 시계, 입력함, effect 실행, 투영 분배
```

같은 에이전트가 왼쪽(명령을 보냄)과 오른쪽(커널이 깨움)에 동시에 있다. 이 고리가 양방향 구조이다.
헥사고날 구조와 의존 방향(`adapter → application → domain`)은 그대로이다.

### 4.2 계층별 책임

| 계층 | 책임 | 현재 위치 |
| --- | --- | --- |
| 응용 프로토콜 (inbound 포트) | 명령·조회·구독 계약. 모든 클라이언트의 공통 입구 | 없음 (TUI 는 `AppCommand`, 다른 입구는 서비스 직접 호출) |
| application: 세션 상태 머신 | thread 별 진행 중 턴, 승인 대기, 턴 종료 후 상태. 모든 클라이언트가 공유 | 클라이언트 런타임 (단수형) |
| application: 정책 | 자동 후속 턴, 턴 종료 후 평가, 턴 옵션, 권한, 예산 | TUI 와 클라이언트 런타임에 나뉨 |
| 런타임 호스트 | 구동 루프, 시계(tick 입력), 시작 순서, 입력함(블로킹 수신), effect 실행, 투영 분배 | TUI (`ratatui_frontend.rs`) 와 composition |
| 클라이언트 (inbound 어댑터) | 입력을 명령으로, 투영을 화면으로. 클라이언트별 화면 상태 | TUI (화면 상태는 이미 분리됨) |

### 4.3 AI 상호작용 기본 기능

| # | 기능 | 원형 |
| --- | --- | --- |
| 1 | 비동기 연산 + 영수증 (`op_id`, 상태, `next`) | Google AIP-151, HATEOAS |
| 2 | 커서가 있는 영구 이벤트 기록 | Kubernetes watch, Kafka |
| 3 | 우편함과 깨우기 (커널이 에이전트 스레드에 턴을 시작) | Erlang, Temporal signal |
| 4 | 묻기·답하기, 넘기기 | Orca orchestration (`ask`, `worker_done`) |
| 5 | 승인을 독립 객체로 (사람, 에이전트, 자동 정책으로 보냄) | Kubernetes admission, BPMN user task |
| 6 | 자기 설명 (`describe`, 스키마, 버전) | D-Bus introspection, LSP capabilities |
| 7 | 안전장치 (권한 범위, 예산, 재귀 깊이, dry-run, 감사 기록) | Kubernetes RBAC + audit |

### 4.4 AI 입구: 스펙 하나, 입구 여러 개

```text
명령 스펙 (단일 정의: 이름, 입력·출력 JSON Schema, 권한 범위, 부작용, 멱등 여부, 설명)
  +-> 응용 프로토콜 (JSON-RPC)   정본. Desktop, TUI 등 프로그램이 쓴다
  +-> CLI `akra <그룹> <동작> --json`   에이전트의 기본 경로 (셸만 있으면 된다)
  +-> MCP 도구                   위험한 조작과 셸 없는 클라이언트용. 수를 작게 유지한다
  +-> SDK (TypeScript, Python)   데이터가 많은 다단계 작업용. 중간 결과가 모델 문맥을 거치지 않는다
  +-> 스킬 문서 + `akra describe --json`
```

- Rust 에서는 `serde` + `schemars` 로 스키마를 만들고, 거기서 CLI, MCP, SDK 를 생성한다.
- `--json` 출력은 안정된 형식으로 유지하고, 사람용 출력과 분리한다.
- 오류에는 다음에 실행할 명령을 적는다. 응답에는 `next` 를 넣는다.
- 권한 검사는 입구가 아니라 커널에서 한다.

## 5. 수정안

### 5.1 결정 규칙을 application 으로 옮긴다

| 현재 (TUI) | 수정 후 |
| --- | --- |
| 턴 종료 감지 → 평가 요청 | 세션 상태 머신이 턴 종료 이벤트를 처리하면서 평가 effect 를 직접 낸다 |
| 평가 결과가 자동 제출 → `SubmitTurn` | 정책이 내부 명령으로 제출한다. 호출자는 `system:auto-follow` |
| 턴 옵션, 슬롯 lease 넘기기 | application 정책 객체(예: `TurnOptionsPolicy`)가 세션 상태와 병렬 제어부 정보로 만든다 |

### 5.2 평가에 필요한 최소 기록을 커널이 갖는다

턴 종료 후 평가에는 최근 사용자 메시지와 최근 응답만 필요하다. 세션 상태 머신이 스트림 이벤트에서 thread 별로 이
두 값만 유지한다. 전체 대화 기록은 클라이언트가 그리거나 app-server 의 `thread/read` 로 읽는다.

### 5.3 런타임 호스트를 만든다

| 구성 요소 | 내용 |
| --- | --- |
| 시계 | `CoreInput::Tick { kind }` 형태의 입력을 낸다 (병렬 tick, GitHub 폴링). tick 에 무엇을 할지는 정책이 정한다 |
| 시작 순서 | 부팅 때 `RunStartupChecks` → `LoadSessionCatalog` 를 호스트가 실행한다 |
| 병렬 제어부 연결 | composition 에서 커널 입력함으로 직접 연결한다. TUI 경유를 없앤다 |
| 입력함 | polling 대신 블로킹 수신과 종료 신호를 쓴다 |
| 투영 분배 | `ProjectionSink` 로 구독자 여럿에게 보낸다. TUI 는 같은 프로세스 안의 구독자 하나가 된다 |

클라이언트가 없어도 루프가 돌고, 여럿이어도 한 번만 돈다. K-D5(중복 실행)가 구조적으로 사라진다.

### 5.4 thread 별 세션 상태와 클라이언트별 상태를 나눈다

| 상태 | 수정 후 위치 |
| --- | --- |
| 진행 중 턴, 승인, 턴 종료 후 상태, 스트림 상태 | 세션 상태 머신, **thread 별 맵** |
| 세션 목록, 계획 기준 데이터 작업, 큐 변경, GitHub 리뷰, 설정 쓰기 | 세션 상태 머신 (공유) |
| 보고 있는 대화, 대화 로드 절차, 조회 모델(peek, review center, queue, directions), 수동 프롬프트 준비, 작업자 패널 seed | 클라이언트 세션 (접속마다) |
| `revision` | 커널용과 클라이언트용으로 나눈다 |

- 대화를 여는 일은 **클라이언트 세션의 구독 변경**이 된다. 공유 런타임을 지우지 않는다 (제약 1 해소).
- 조회 모델은 요청자에게 결과를 돌려준다. 슬롯 하나에 덮어쓰지 않는다.

### 5.5 응용 프로토콜을 만든다

- 명령, 조회, 구독. 모든 호출에 호출자 신원과 멱등 키를 붙인다.
- 오래 걸리는 명령은 연산(Operation)을 돌려준다. 완료는 이벤트로 알린다.
- 이벤트는 추가 전용 기록과 커서로 제공한다. 계획 기준 데이터(SQLite)는 상태 저장소로 두고, 그 옆에 이벤트 표를 둔다.
- 화면용 메시지는 굵은 단위로 설계한다 (Xi 의 교훈). 키 입력은 클라이언트 안에서 처리하고 확정된 명령만 보낸다.
- 화면 흐름 명령(§3.2 제약 4)은 프로토콜의 도메인 명령에서 뺀다. 2단계 제출은 한 번에 끝나는 명령으로 합친다.

### 5.6 명령 경로를 하나로 통일한다

CLI, admin, Telegram 이 application 서비스를 직접 부르지 않고 응용 프로토콜(또는 같은 명령 처리기)을 거친다.

| 결함 | 해결 |
| --- | --- |
| K-D1 초기화 확인 | **완료 (2026-11-07):** reset service 가 확인되지 않은 directions/all 을 거절한다. CLI `--confirm`, Telegram `confirm`, admin JSON `"confirm": true`(없으면 428). TUI·커널 프로토콜 경로도 완료 (2026-11-09): core 초기화 명령이 호출자의 확인 플래그를 싣고, effect runner 가 그 값을 서비스에 넘긴다. 확인을 가정하지 않는다. 원래 계획: 확인 규칙을 공유 정책으로 옮긴다. 모든 입구가 같은 확인을 거친다 (예: 확인 토큰이나 `--confirm`) |
| K-D2 revision 검사 | **완료 (2026-11-07):** admin 작업 수정은 폼을 열 때의 planning revision 을 함께 보낸다. 저장 시점의 revision 과 다르면 덮어쓰지 않고 거절한다. 작업별 `updated_at` 은 초 단위라 쓰지 않았다. 원래 계획: 작업 수정 명령에 `expected_planning_revision` 을 필수로 넣는다 |
| K-D3 병렬 정지 전달 | 병렬 실행 상태의 주인을 커널 하나로 둔다. 다른 프로세스 메모리에 두지 않는다. **부분 (2026-11-09, K3-4):** 같은 저장소에서 결정 고리를 돌리는 프로세스(TUI, 데몬 커널)는 저장소 잠금으로 하나뿐이다. admin 은 아직 자기 프로세스의 병렬 제어부를 움직이므로 admin 의 정지는 TUI 에 닿지 않는다. Gateway 연결(K3-5)과 admin 명령의 커널 경로 이동(K4) 뒤에 해소된다 |
| K-D5 중복 실행 | 결정 고리를 커널 하나만 돌린다. **부분 (2026-11-09, K3-4):** 저장소 잠금(`$AKRA_HOME/projects/<namespace>/runtime/kernel.lock`)을 쥔 프로세스만 턴 종료 평가, 자동 후속, 병렬 tick, PR 검증 스케줄러를 돌린다. 같은 저장소의 두 번째 TUI 는 이유를 보이고 끝난다. 잠금은 K3-4 가 만들고, Gateway 가 커널을 올리는 연결은 K3-5 이다. admin 과 Telegram 은 아직 잠금 없이 PR 검증 스케줄러 스레드를 시작한다 (기록별 DB lease 가 같은 PR 의 중복 폴링을 막는다). K4 에서 커널 경로로 옮긴다 |
| K-D4 감사 기록 | 모든 변경 명령이 커널의 감사 기록을 거친다. **부분 (2026-11-11, plan 19 Q3-6):** 커널 프로토콜로 온 변경 명령은 `command_received` 로 기록된다 (종류, thread, 지문). CLI, admin, Telegram 의 명령은 K4 에서 커널 경로로 옮긴 뒤에 기록된다 |

K-D1 과 K-D2 는 커널 작업과 별개로 먼저 고칠 수 있다 (§6 K0).

### 5.7 아키텍처 테스트를 추가한다

- inbound 어댑터는 `ParallelTick`, `PollGithubReview`, `EvaluatePostTurn`, `ResolvePostTurnContinuation` 을 만들 수 없다.
- inbound 어댑터는 주기 타이머를 소유할 수 없다.
- inbound 어댑터는 응용 프로토콜에만 의존한다. application 서비스를 직접 부르지 않는다.

### 5.8 TUI

TUI(`src/adapter/inbound/tui`, 81,325줄)는 바로 잘라내지 않는다. TUI 안에 결정 규칙(§3.2 제약 2)과, 데스크톱에 아직
없는 하네스 화면(계획 작성, 큐, directions, 리뷰 센터, 병렬 보드, 진단, 활동 타임라인)이 있기 때문이다.

| 단계 | TUI |
| --- | --- |
| 결정 규칙을 옮길 때까지 | 그대로 동작한다 |
| 커널 데몬 이후 | **새 기능 동결.** 버그 수정만 한다 |
| 실제로 쓰는 기능을 Desktop·CLI 가 대체한 뒤 | 삭제하거나, 커널 프로토콜 위의 얇은 터미널 클라이언트로 바꾼다 (Q-K3) |

### 5.9 용어

수정이 끝나면 현재의 `core`(클라이언트 런타임)는 독립 계층으로 남을 이유가 없다. 결정은 application 으로, 실행
장치는 런타임 호스트로, 화면 상태는 클라이언트로 간다. 이름 변경은 마지막에 한다. 구조가 바뀐 것을 확인한 뒤에
바꾼다.

## 6. 이행 단계

규모는 조사의 추정이다. 단계 사이에 겹침이 있다.

| 단계 | 내용 | 기존 동작 | 규모 |
| --- | --- | --- | --- |
| K0 | 결함 K-D6(macOS 하위 프로세스 EPERM, 완료 2026-11-06 #2164) 수정을 가장 먼저 한다. 이어서 K-D7(테스트 환경 잠금, 완료 2026-11-07), K-D1(초기화 확인, 완료 2026-11-07), K-D2(revision 검사, 완료 2026-11-07) 수정 | 유지 (완료 2026-11-07) | 작음 |
| K1 (완료 2026-11-07, #2174~) | 결정 규칙과 타이머를 TUI 에서 application·런타임 호스트로 이동 (상세: [K1 설계](18-k1-headless-decision-loop.md)). 같은 프로세스 안에서 헤드리스 실행 | 유지 (TUI 는 구독자) | 4~7일, 옮길 로직과 테스트 800~1,500줄 |
| K2 (완료 2026-11-08, #2181~#2186) | thread 별 세션 상태 맵, 공유·클라이언트별 상태 분리 (상세: [K2 설계](18-k2-thread-sessions.md)). 클라이언트 상태 타입 분리(K2-5)는 K3 로 옮겼다. 결과를 요청자에게 전달하는 일은 클라이언트가 여럿이 되는 K3 로 옮긴다 (제안) | 유지 | 10~15일 (상태 구조 분리만은 3~4일) |
| K3 (완료 2026-11-10) | 응용 프로토콜, 데몬 호스팅, 원격 클라이언트(직렬화, 전송, 분배) (상세: [K3 설계](18-k3-kernel-daemon-protocol.md)). 원격 클라이언트는 thread 를 지정해 명령하므로 K2-5(클라이언트 상태 분리)는 K6 으로 옮겼다 | 전환 | 5~10일 |
| K4 | 명령 경로 통일 (CLI, admin, Telegram). 스펙에서 CLI·MCP·SDK·스킬 생성 (상세: [K4 설계](18-k4-command-spec.md), 제안 2026-11-12) | 확장 | 조각마다 1~4일 (K4 설계 §4) |
| K5 | 연산, 이벤트 기록·커서, 멱등 키(처리한 명령 기억), 승인 객체, 우편함·깨우기 (양방향 고리 완성). 이벤트 기록·커서·승인 기록·연산 영수증·멱등 키는 끝났다 (plan 19 Q3-1~Q3-7, 설계는 §5.1.2). 남은 것: 승인을 보내는 정책, 연산의 끝 상태, 묻기·답하기 (Q4 뒤) | 확장 | 일부 완료 |
| K6 | TUI 동결 → 대체 → 얇은 TUI (Q-K3 답). 클라이언트별 보기(K2-5)와 이벤트 대상 지정을 여기서 한다 | 축소 | 미정 |

K1~K3 합계는 약 3~5주로 추정한다.

plan 19 는 K5 의 사건 기록을 K4 의 스펙보다 먼저 하자고 제안한다 (plan 19 §6, Q-Q1). 사건 목록이 정해져야 명령·사건·
거절 이유 스펙을 한 번에 만들 수 있기 때문이다. 답이 나오면 위 표의 순서를 고친다.

## 7. 결정

| ID | 결정 | 상태 |
| --- | --- | --- |
| KP-01 | 경로는 "커널 추출"로 한다. 새 커널을 따로 만들지 않는다. 경계 분리가 깨끗하고 상태가 이미 클라이언트 런타임에 있기 때문이다 | 승인 (2026-11-07) |
| KP-02 | 결정 규칙과 타이머의 주인은 application 과 런타임 호스트이다. inbound 어댑터는 결정하지 않는다 | 승인 (2026-11-07) |
| KP-03 | 세션 상태는 thread 별로 둔다. 대화를 여는 일은 클라이언트의 구독 변경이다 | 승인 (2026-11-07) |
| KP-04 | 모든 입구는 응용 프로토콜 하나를 거친다. 명령은 단일 스펙에서 정의하고 입구를 생성한다 | 승인 (2026-11-07) |
| KP-05 | 에이전트의 기본 입구는 `akra ... --json` CLI 와 스킬 문서이다. MCP 는 핵심·위험 조작만, SDK 는 데이터 작업용이다 | 승인 (2026-11-07) |
| KP-06 | TUI 는 커널 데몬 이후 새 기능을 동결한다. 대체가 끝나면 커널 프로토콜 위의 얇은 터미널 클라이언트로 남긴다 (Q-K3) | 승인 (2026-11-07) |
| KP-07 | K-D6, K-D7, K-D1, K-D2 는 커널 작업과 별개로 먼저 고친다. K-D6 이 첫 번째이다 | 승인 (2026-11-07) |

사용자가 2026-11-07 에 KP-01~07 을 모두 승인했다. 결정을 바꿀 때는 이 표의 상태와 날짜를 고치고, 바꾼 이유를
적는다.

## 8. 미결 질문

| ID | 질문 | 영향 |
| --- | --- | --- |
| Q-K1 | 주 사용자는 사람인가, 리드 에이전트인가 (사람은 승인·감독만) | **답함 (2026-11-07):** 사람이다. 화면용 명령과 이벤트를 먼저 설계하고, 에이전트도 같은 명령을 쓴다. 승인의 기본 대상은 사람이다 |
| Q-K2 | 작업 자동화를 선언형(원하는 상태 + reconcile)으로 할 것인가, 명령형으로 할 것인가 | 하네스 정책과 이벤트 설계 |
| Q-K3 | 터미널만 되는 환경(SSH, 서버)에서 Akra 를 쓰는가 | **답함 (2026-11-07):** 쓴다. TUI 는 삭제하지 않고 얇은 터미널 클라이언트로 남긴다 (KP-06) |
| Q-K4 | TUI 기능 중 실제로 쓰는 것은 무엇인가 | K6 대체 범위 |
| Q-K5 | 에이전트의 자율성 한계 (에이전트가 띄우는 에이전트의 깊이, 동시 실행 수, 예산) | 정책, 안전장치 |
| Q-K6 | 기기 하나인가, 여러 대(집, 회사)인가 | **답함 (2026-11-07):** 지금은 한 대이고, 나중에 여러 대가 될 수 있다. 커널은 로컬 데몬으로 시작한다. K3 의 전송 계층은 원격 접속을 나중에 더할 수 있게 설계하되, 원격 인증은 지금 만들지 않는다 |
| Q-K7 | 병렬 하네스 테스트 불안정의 원인 (§3.5) | **답함 (2026-11-06):** 하네스 로직이 아니라 하위 프로세스 계층의 macOS 버그(K-D6)이다. K-D6 을 고치면 하네스를 커널 기반으로 쓸 수 있다고 본다 |

## 9. 근거 파일

조사에 쓴 주요 파일이다. 줄 번호는 기준 커밋 기준이다.

- 클라이언트 런타임: `src/core/mod.rs`, `src/core/app/controller.rs`, `src/core/app/controller/state.rs`,
  `src/core/app/command.rs`, `src/core/app/snapshot.rs`, `src/core/app/projection.rs`,
  `src/core/app/conversation_turn_reducer.rs`, `src/core/app/turn_stream.rs`, `src/core/app/read_model_reducer.rs`,
  `src/core/runtime/driver.rs`, `src/core/native_client_port.rs`
- composition: `src/composition/production.rs`, `src/composition/native_client_runtime.rs`,
  `src/composition/core_effect_worker.rs`
- TUI: `src/adapter/inbound/tui/app/conversation_runtime.rs`, `turn_submission_runtime.rs`,
  `post_turn_execution.rs`, `parallel_mode.rs`, `shell_runtime.rs`, `app_runtime.rs`, `ratatui_frontend.rs`
- 다른 입구: `src/adapter/inbound/cli.rs`, `src/adapter/inbound/admin_api/`, `src/adapter/inbound/telegram_bot/`
- 서비스: `src/application/service/planning/repair/reset.rs`, `authoring/init.rs`, `task_mutation.rs`,
  `src/application/service/parallel_mode/`
- 경계 테스트: `tests/architecture_boundaries.rs`
