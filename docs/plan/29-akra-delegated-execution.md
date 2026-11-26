# 29. 위임 실행 (delegated execution)

상태: **부분 구현** (2026-11-26, §14.1). D1·D2·D5 와 D3·D4·D6 의 대부분은 PR #2595, #2639 로 들어왔다. 현재 동작의 기준은
[current-product](../reference/current-product.md) 의 위임 절과 [PR 리뷰 백엔드](../reference/pr-review-backend.md) 이다. 나머지
문장은 제안이다. · 처음 기준일: 2026-11-21 (Asia/Seoul) ·
기준 커밋: `33a12b72` (`origin/prerelease`), akra-tunnel `7cd0de6` (`master`, 다른 저장소) ·
관련: [plan 17 Akra Desktop](17-akra-desktop/README.md) ([R3](17-akra-desktop/roadmap/R3-browser-panel.md),
[R9](17-akra-desktop/roadmap/R9-agent-browser-and-screenshots.md), [결정 D-13, D-14, D-17, D-19](17-akra-desktop/03-decisions.md)),
[plan 18 kernel](18-akra-kernel-architecture.md), [plan 19 kernel 성질 목표](19-akra-kernel-quality-goals.md),
[plan 20 관찰-제안](20-akra-observe-and-offer.md), [plan 23 주의 항목](23-akra-attention-items.md),
[plan 25 클라우드 에이전트 호스트](25-akra-cloud-agent-host.md), [plan 28 이슈 자동화](28-akra-issue-automation.md)

조각 문서: [D1 계약과 장부](29-d1-contract-and-ledger.md), [D2 Codex thread 백엔드](29-d2-codex-thread-backend.md),
[D3 ChatGPT 채팅 백엔드](29-d3-chatgpt-chat-backend.md), [D4 경로와 대체](29-d4-routing-and-fallback.md),
[D5 기능 바인딩과 PR 자동 리뷰](29-d5-feature-binding-and-pr-review.md),
[D6 에이전트 입구와 Desktop 화면](29-d6-agent-entry-and-desktop.md),
[D7 akra-tunnel-core 분리](29-d7-akra-tunnel-core.md) (새 저장소 `RefinedStone/akra-tunnel-core` 와 akra-tunnel 의 일.
이슈: [akra-tunnel-core #1](https://github.com/RefinedStone/akra-tunnel-core/issues/1))

## 한 줄 요약

Akra 의 기능은 일 하나를 "위임 작업"으로 낸다. 경로 정책이 백엔드(ChatGPT 채팅, Codex thread)를 우선순위로 고른다.
모든 백엔드는 같은 계약으로 전달, 진행, 중단, 결과를 알린다. 백엔드를 쓰지 못하면 다음 백엔드로 넘어간다.

## 1. 사용자가 원하는 것 (요청 2026-11-21)

- akra-tunnel 과 ChatGPT 대화 조회를 만든 이유는 ChatGPT 채팅에 일을 시키는 것이다.
- API 로는 임시 대화와 특정 대화 이어가기를 하지 못한다. 그래서 보내기는 Akra Desktop 의 앱 안 브라우저로 한다.
  목록 조회 같은 읽기는 일부 API 를 쓴다.
- 일을 보내는 방법, 진행 중, 비정상 중단은 Akra 가 정의한 interface 로 나타낸다.
- 예: LLM PR 자동 리뷰. 저장소마다 켜고 끈다. 실행은 ChatGPT 채팅(tunnel)이 하거나, 지정한 LLM 이 한다.
  우선순위가 있다. 토큰이 없거나 서비스가 비정상이면 다음 것으로 넘어간다.
- ChatGPT 를 단방향(지시와 결과) 서브에이전트로도 쓴다.
- 지금의 ChatGPT 코드는 조회만 확인한 시험 구현이다. 구조를 위해 고쳐도 된다.

## 2. 용어

| 용어 | 뜻 | 기존 코드와 계획의 대응 |
| --- | --- | --- |
| 위임 작업 (delegation) | 기능이 백엔드에 맡기는 일 하나. 지시가 들어가고 결과가 나온다 | plan 20 의 "위임 thread" 를 넓힌 것 |
| 백엔드 (agent backend) | 위임 작업을 실행하는 곳: `chatgpt_chat`, `codex_thread`. 뒤에 다른 것 | plan 18 §4.1 의 `AgentRuntimePort` 자리 |
| 시도 (attempt) | 작업 하나를 백엔드 하나에서 한 번 실행한 것 | akra-tunnel 의 `Attempt` |
| 세션 (session) | 백엔드 안의 대화 자리: ChatGPT 대화, Codex thread | - |
| 관찰 (observation) | 백엔드의 지금 상태를 읽은 값 하나 | akra-tunnel 의 `Observation` |
| 경로 (route) | 기능 하나의 백엔드 우선순위 목록 | - |
| 대체 (fallback) | 시도가 끝나지 못해 다음 백엔드로 넘어가는 것 | - |
| 기능 바인딩 (feature binding) | 저장소 하나와 기능 하나의 연결: 수준과 경로 | plan 28 의 바인딩 |
| akra-tunnel-core | akra-tunnel 에서 나눈 라이브러리. 페이지 읽기와 입력, 세션 감시와 복구, tunnel 런타임 서비스를 가진다. Electron 을 import 하지 않는다. Akra 의 `src/core` 와 다른 것이다. 이 문서는 `core` 로 줄여 쓰지 않는다 | akra-tunnel 의 `src/` 와 `launcher/*-dom.mjs` ([D7](29-d7-akra-tunnel-core.md)) |
| tunnel frontend | Desktop main 에서 akra-tunnel-core 를 싣고 `PagePort` 를 구현하는 부품. Akra Desktop 이 만든다. host 임대를 쥐면 host 이다 | R9 의 `BrowserAgentService`, akra-tunnel 의 `continuation-browser-host` |

## 3. 이미 있는 것과 없는 것

코드에서 확인했다 (2026-11-21).

있는 것 (Akra):

| 부품 | 근거 |
| --- | --- |
| ChatGPT 대화 읽기 | `ChatsPort` (`list_chats`, `read_chat`), `cloud_chats.rs`, Gateway `akra/chats/list`, `akra/chats/read`, Desktop `ChatDock.tsx`. HTTP 429 는 `RateLimited` 와 마지막 성공 값이다 |
| 앱 안 브라우저 | `BrowserPanelService` (프로필별 세션 `persist:akra-browser-<id>`), `BrowserAgentService`, `BrowserAutomationPort`, `akra-browser-mcp`, Rust 의 `desktop_browser_bridge` |
| Gateway 가 연결된 Desktop 에 되묻기 | `akra/resourceVersion/current` (`gateway/resource_version.rs`) |
| 고립 Codex thread | `ParallelAgentWorkerPort::run_isolated_new_thread_stream`, `PlanningWorkerPort` |
| 턴의 끝 분류 | `ConversationTurnTerminalOutcome` (`Completed`, `Interrupted`, `Failed`, `Unknown`), `ConversationTurnErrorInfo` (`UsageLimitExceeded`, `ServerOverloaded`, `Unauthorized` 등) |
| 모델을 쓰는 소비자의 자리 | `IssueReviewModelPort` (구현은 Fake 하나). plan 28 의 판정 모델과 중재자, plan 24 §4.5 의 게이트 리뷰어는 제안이다 |
| 명령 스펙, 멱등 키, 영수증, 사건 기록, 마감 타이머 | plan 18 K4, plan 19 Q3 |
| 저장소별 독립 장부의 방식 | 이슈 자동화 장부 (`issue_automation_store_port`) |

있는 것 (akra-tunnel. 다른 저장소이고 따로 도는 제품이다):

| 부품 | 근거 |
| --- | --- |
| tunnel 런타임 | ChatGPT 가 OpenAI Secure MCP Tunnel 로 이 컴퓨터의 파일과 셸을 쓴다. `akra-tunnel status` 는 JSON 을 낸다 |
| 대화 관찰과 이어가기 | `continuation-contracts.ts`: 관찰 단계 `failed`, `disconnected`, `load_failed`, `running`, `complete`, `stopped`, `blocked`, `unknown`. 보내기 확인 `sending` → `host_acknowledged` → `message_observed` → `progress_observed`, 그리고 `send_uncertain`. 재개 한도 |
| 브라우저 작업자 | `chatgpt-image-contracts.ts`: 대화 방식 `worker`, `temporary`, `current`. `requestId` 멱등, 작업 상태, FIFO 큐 |
| 진행 근거 세 가지 | DOM 관찰 (`continuation-dom.mjs`), 클라우드 스냅숏, tunnel 도구 호출의 대화 태그 |
| 영속 작업 | `task_submit`, `task_checkpoint`. `interrupted` 는 "부작용이 없다"의 증거가 아니다 (`DURABLE_TASK_RUNTIME.md`) |

없는 것:

| 없는 것 | 지금 |
| --- | --- |
| 백엔드 공통 계약 | 소비자마다 port 가 따로이다. 전달, 진행, 중단, 결과의 공통 모양이 없다 |
| ChatGPT 로 보내기 | Akra 에 없다. Gateway 는 보내지 않는다 (plan 25, sentinel 을 우회하지 않는다) |
| 경로와 대체 | 없다 |
| 백엔드의 건강 상태 | 없다. Akra 는 `account/rateLimits/*` 를 부르지 않는다 |
| 저장소별 기능 켜기의 공통 자리 | 기능마다 따로이다 (`akra.prValidationMode`, 이슈 자동화의 바인딩) |
| 위임 작업의 장부 | 없다 |
| 에이전트가 다른 백엔드에 일을 맡기는 명령 | 없다 |

결론: 계약, 장부, 경로, tunnel frontend 가 새 일이다. akra-tunnel-core 의 분리는 다른 저장소의 일이다 (D7). Codex thread 실행, 브라우저 보기, 명령 스펙은 있는 것을 쓴다.

## 4. 원칙

1. **소비자는 백엔드를 모른다.** 기능은 위임 작업을 내고 결과를 받는다.
2. **작업의 판단은 kernel 에 있다.** 경로, 대체, 예산, 마감, 결과의 수락은 Rust 의 순수 함수가 고른다 (AGENTS.md:
   클라이언트는 얇다). 세션 하나를 살리는 일(새로 고침, 재개, 전달 확인)은 백엔드 안에 있다. ChatGPT 에서는
   akra-tunnel-core 가 한다. Codex thread 를 app-server 가 맡는 것과 같은 자리이다.
3. **상태는 닫힌 목록이다.** 백엔드의 고유 신호는 adapter 가 공통 값으로 옮긴다.
4. **전달과 결과는 다른 사실이다.** "보냈다", "백엔드가 받았다", "일한다", "끝났다"를 따로 적는다.
5. **결과를 모르면 다시 보내지 않는다.** 읽어서 확인한다 (plan 28 §6, akra-tunnel 의 규칙).
6. **대체는 부작용 등급을 따른다.** 전달 전의 대체는 항상 된다. 전달 뒤의 자동 대체는 읽기 전용 작업만 된다 (§7.3).
7. **백엔드의 결과는 자료이다.** 지시문이 아니다. 밖으로 쓰는 일(댓글 등)은 기능이 기존 일지 경로로 한다.
8. **내용을 받을 백엔드는 사용자가 정한다.** 저장소의 파일은 줄일 수만 있다 (plan 17 D-14, plan 28 §7.3).
9. **우회하지 않는다.** akra-tunnel-core 는 사용자가 로그인한 공식 웹 페이지를 조작한다. 로그인, 챌린지, 한도는 사람에게 넘긴다
   (R9 §9).
10. **시험은 모델을 부르지 않는다** (D-13). 가짜 백엔드, 가짜 페이지, 가짜 시계를 쓴다.
11. **주 실행 경로를 바꾸지 않는다.** 사용자 대화와 병렬 슬롯 전달은 그대로이다 (plan 28 원칙 7). 위임은 보조 작업의
    길이다.

## 5. 구조

### 5.1 구성 요소

```text
 기능 (소비자)                 위임 모듈 (저장소 kernel 안)                      백엔드 (adapter)
 PR 자동 리뷰 ────┐        ┌───────────────────────────────────┐
 이슈 판정, 중재자 ┼─ 작업 ─▶│ 접수 ─▶ 장부 ◀─▶ 조정기            │              ┌─ codex_thread ─ app-server (고립 thread)
 이슈 초안 검토 ──┤        │            ├─ 경로 선택 (정책, 건강) │─ AgentBackend ┼─ chatgpt_chat ─ Gateway ─ tunnel frontend (akra-tunnel-core) ─ chatgpt.com
 에이전트 ────────┘◀ 결과 ─│            ├─ 다음 행동 (순수 함수)  │    Port      │                    └ (선택) tunnel ─ 이 컴퓨터
      ▲                    │            └─ 주의 항목 (사람 필요)  │              └─ fake (시험)
      │ 명령, 조회, 이벤트   └───────────────────────────────────┘
 Desktop · CLI · 에이전트                    ▲ 콜백: 체크포인트, 결과 보고 (akra kernel delegation …)
 (kernel spec 의 범위 delegation)            └──────────── tunnel 이나 Codex 의 셸에서 백엔드가 실행한다
```

### 5.2 경계 (port)

모듈은 `domain/delegation` 과 `application/service/delegation` 이다.

| port | 방향 | 하는 일 | adapter |
| --- | --- | --- | --- |
| `DelegationPort` | inbound | 접수, 취소, 조회, 콜백 | kernel 프로토콜 (범위 `delegation`) |
| `AgentBackendPort` | outbound | 능력, 건강, 보내기, 관찰, 멈춤, 결과 읽기 (§5.3) | `codex_thread`, `chatgpt_chat`, fake |
| `DelegationStorePort` | outbound | 장부 | SQLite. 저장소 runtime 폴더의 새 파일이다 |
| `DelegationPolicySourcePort` | outbound | 경로 정책 파일 | 파일 |
| `ChatSessionPort` | outbound | tunnel frontend 에 내는 호출 (§8.2). akra-tunnel-core 의 `ChatSessionService` 를 프로세스 너머로 부른다. tunnel 의 상태도 이 길로 온다. `chatgpt_chat` adapter 만 쓴다 | Gateway 의 되묻기 |
| `ChatsPort` (있음) | outbound | 대화 목록과 한 건 읽기. Desktop 의 화면만 쓴다. 위임은 쓰지 않는다 | `cloud_chats` |

- 모듈은 planning, parallel, `src/core` 의 타입을 import 하지 않는다. architecture test 가 이것을 막는다 (plan 28 §5.2 와 같다).
- 기존 `ParallelAgentWorkerPort`, `PlanningWorkerPort`, `InteractiveTurnRuntimePort` 는 바꾸지 않는다.
- `IssueReviewModelPort` 같은 소비자 port 는 남는다. 그 adapter 가 위임 작업을 낸다. 소비자의 코드는 바뀌지 않는다.

### 5.3 `AgentBackendPort` (중심 계약)

```text
capabilities()                  -> { sessions[new|temporary|continue], local_tools, structured_result, self_recovery, stop, max_concurrent }
health(now)                     -> ready | degraded{reason} | unavailable{reason, retry_at?}
dispatch(job, attempt, session) -> { delivery, session_ref? }
observe(attempt)                -> { phase, recovery, evidence[], progress_key?, observed_at }
stop(attempt)                   -> ()
collect(attempt)                -> { text, structured?, usage? }
```

- 세션의 복구는 백엔드가 안에서 한다. 관찰 값의 `recovery` 는 `none`, `in_progress`, `exhausted` 이다. kernel 은
  `exhausted` 뒤에만 대체를 고른다.

- 모든 호출은 짧게 끝난다. 긴 일은 조정기가 마감 시각마다 `observe` 를 부른다 (KP-02).
- 호출의 응답을 잃으면 그 결과는 "모름"이다. adapter 는 성공이나 실패로 꾸미지 않는다.
- 스트림을 주는 백엔드(Codex)는 adapter 가 스트림을 관찰 값으로 접는다.
- 새 백엔드는 이 port 의 adapter 하나이다. 조정기와 소비자는 바뀌지 않는다.

### 5.4 실행 자리

| 질문 | 답 (제안) | 이유 |
| --- | --- | --- |
| 명령은 어디로 들어오는가 | kernel spec 의 새 범위 `delegation` | CLI `akra kernel delegation … --json`, 생성 skill, 멱등 키, 영수증, `caller` 기록을 그대로 얻는다 |
| 타이머는 누가 돌리는가 | 런타임 호스트 | KP-02. 관찰 주기, 마감, 재시도는 모두 마감 시각이다 |
| 장부는 어디에 두는가 | 저장소별 SQLite 의 새 파일 | planning DB 의 schema version 을 올리지 않는다 (plan 28 §5.2) |
| 백엔드 자원은 누가 나누는가 | tunnel frontend 의 줄 | host 는 home 하나에 하나이다. 여러 저장소의 kernel 이 같은 줄에 선다. kernel 은 lease 를 두지 않는다 |
| tunnel frontend 는 어디에 있는가 | Desktop main | 로그인한 브라우저 세션이 거기 있다. Desktop 이 없으면 `chatgpt_chat` 은 `unavailable` 이다 |
| 기록은 어디에 쓰는가 | `kernel_events` 에 `delegation_*` 종류로 쓴다 | plan 20 §4.1 "사건은 한 곳" |
| 지시와 결과의 원문 | 장부에만 둔다. 사건 기록에는 해시와 크기만 둔다 | 재시작 뒤에 보내려면 지시가 필요하다. 보존 기간은 §15 Q-DE9 |

## 6. 상태 모델

이 절이 "진행 중"과 "비정상 중단"을 나타내는 interface 이다. 모든 백엔드가 같은 값을 쓴다.

### 6.1 위임 작업

```text
delegation: { id, feature, origin{ repository, thread?, caller, depth },
              request{ instruction, inputs[], result_schema? },
              effect: read_only | idempotent | side_effecting,
              needs{ local_tools, session }, route[], deadline,
              state, flags[], attempts[], result?, attention? }
```

| 상태 | 뜻 | 나가는 길 |
| --- | --- | --- |
| `queued` | 접수했다. 백엔드를 아직 고르지 않았다 | `dispatching`, `failed` (쓸 백엔드가 없다), `cancelled` |
| `dispatching` | 백엔드를 골랐고 지시를 보내는 중이다. 전달을 아직 확인하지 못했다 | `running`, `queued` (대체), `awaiting_human` |
| `running` | 전달을 확인했다. 백엔드가 일한다 | `succeeded`, `interrupted`, `awaiting_human`, `expired` |
| `interrupted` | 백엔드가 끝나기 전에 멈췄다. 비정상 중단이다. 백엔드가 복구하는 중일 수 있다 | `running` (백엔드가 복구했다), `queued` (대체), `awaiting_human`, `failed` |
| `awaiting_human` | 사람이 필요하다: 로그인, 승인, 챌린지, 결과 모름의 확인 | 사람의 행동 뒤에 앞의 상태로 간다 |
| `succeeded` | 결과를 받았고 모양을 확인했다 | 끝 |
| `failed` | 경로를 다 썼다. 또는 대체가 금지이다 | 끝. 사람은 다시 낼 수 있다 |
| `cancelled` | 사람이나 소비자가 취소했다 | 끝 |
| `expired` | 마감을 넘겼다 | 끝 |

상태와 따로 표시 두 개가 있다.

- `stalled`: `running` 인데 진행 근거가 기준 시간 동안 없다. 상태를 바꾸지 않는다. 화면과 정책이 본다.
- `outcome_unknown`: 보내기나 결과를 확인하지 못했다. 이 표시가 있으면 조정기는 새로 보내지 않는다.

### 6.2 시도

시도는 값 세 개를 따로 가진다.

**전달 (delivery).** 지시가 백엔드에 닿았는가.

| 값 | 뜻 |
| --- | --- |
| `not_sent` | 보내지 않았다. 대체해도 안전하다 |
| `sending` | 보내는 중이다 |
| `accepted` | 백엔드가 받았다고 답했다 |
| `observed` | 세션에서 내 지시(표식)를 봤다 |
| `uncertain` | 보냈는지 모른다. 다시 보내지 않는다 |

**단계 (phase).** 관찰이 준 값이다.

| 단계 | 뜻 | ChatGPT 채팅의 근거 | Codex thread 의 근거 |
| --- | --- | --- | --- |
| `working` | 일하는 중이다 | 멈춤 버튼, tunnel 도구 호출, 체크포인트 | 스트림 사건, 진행 중인 턴 |
| `complete` | 끝났다 | 끝 표시와 표식 뒤의 답, 결과 콜백 | `Completed` |
| `stopped` | 누가 멈췄다 | "응답이 중지됨" | `Interrupted` |
| `failed` | 백엔드가 오류로 끝냈다 | "생각 실패", 오류 알림 | `Failed` |
| `disconnected` | 페이지나 스트림의 연결이 끊겼다. 백엔드는 계속 일할 수 있다 | "연결이 해제되었습니다", 전송 시간 초과 알림 | `ResponseStreamDisconnected` |
| `blocked` | 사람이 필요하다 | 승인 버튼, 로그인 화면, 챌린지, 한도 알림 | 승인 요청, `Unauthorized` |
| `unreachable` | 관찰하지 못했다 | 페이지 로드 실패, Desktop 없음 | app-server 없음 |
| `unknown` | 읽었지만 가리지 못했다 | 모르는 DOM | `Unknown` |

**끝 원인 (end reason).** 시도가 성공 없이 끝난 이유이다. 대체는 이 값을 본다.

| 분류 | 예 | 같은 백엔드에서 | 다음 백엔드로 |
| --- | --- | --- | --- |
| `unavailable` | Desktop 이 꺼짐, 로그아웃, tunnel 없음, app-server 없음 | 하지 않는다 | 간다 (전달 전이다) |
| `quota` | HTTP 429, 사용 한도, 한도 알림 | `retry_at` 까지 쉰다 | 간다 |
| `transient` | 과부하, 네트워크, 페이지 로드 실패 | 간격을 늘려 다시 한다 | 한도 뒤에 간다 |
| `interrupted` | 생각 실패, 스트림 끊김, 앱 종료 | 백엔드가 안에서 복구한다 (새로 고침, 재개. 한도는 `max_resumes`) | 복구가 끝난 뒤에 부작용 등급에 따른다 (§7.3) |
| `needs_human` | 로그인, 챌린지, 승인 | 주의 항목을 연다 | 정책의 마감 뒤에 간다 |
| `rejected` | 정책 거절, 잘못된 요청 | 하지 않는다 | 기본은 가지 않는다 |
| `result_invalid` | 결과가 요구한 모양이 아니다 | 한 번 다시 묻는다 | 간다 |
| `unknown` | 보내기의 응답을 잃음 | 읽어서 확인만 한다 | 확인 전에는 가지 않는다 |

### 6.3 진행 근거

근거의 출처는 다섯이다: `browser` (DOM), `tunnel` (도구 호출), `cloud` (페이지가 받은 대화 응답), `callback` (체크포인트,
결과 보고), `stream` (app-server).

- 근거 하나가 `working` 이면 `stalled` 가 아니다. 예: 페이지는 `disconnected` 인데 tunnel 도구 호출이 계속 온다. 이때
  백엔드는 일하는 중이다 (akra-tunnel `600423b`).
- `cloud` 는 로그인한 페이지가 이미 받은 응답을 읽는 것이다. 따로 요청하지 않는다. 그래서 조회의 계정 한도를 쓰지 않고,
  계정이 어긋나지 않는다 (akra-tunnel 의 `continuation-cloud-snapshot.mjs`).
- 관찰 값은 출처와 시각을 가진다. 화면은 "무엇을 근거로 진행 중인가"를 보인다.

### 6.4 다음 행동

- `next_action(작업, 시도들, 백엔드 건강, 정책, 시각) → 행동` 은 domain 의 순수 함수이다.
- 행동은 닫힌 목록이다: `select_backend`, `dispatch`, `observe`, `stop`, `collect`, `fall_back`, `ask_human`, `finish`,
  `none`. 새로 고침과 재개는 이 목록에 없다. 그것은 백엔드 안의 일이다.
- 입력은 저장된 값이다. 사건 하나를 놓쳐도 다음 주기에 같은 답이 나온다. 재시작해도 이어진다 (plan 19 P1).
- 밖으로 나가는 행동(`dispatch`)은 먼저 장부에 적는다. 응답을 잃으면 `outcome_unknown` 이다.

## 7. 경로와 대체

### 7.1 정책 파일

- 자리: 사용자 파일 `$AKRA_HOME/delegation.toml`, 저장소 파일 `.akra/delegation.toml`.
- 백엔드의 허락, `tool_access`, 예산의 최대값은 사용자 파일에서만 온다. 저장소 파일은 경로에서 백엔드를 빼거나 기능을
  끌 수만 있다.
- 깨진 파일은 `policy_blocked` 이다. 내장 값으로 대신하지 않는다.

아래는 모양의 예이다. 키 이름은 D4 에서 정한다.

```toml
schema_version = 1

[backend.chatgpt_chat]
enabled = true
tool_access = "none"          # none | tunnel
session = "new"               # new | temporary
max_concurrent = 1
max_resumes = 2               # 백엔드 안의 재개 한도

[backend.codex_thread]
enabled = true
model = ""                    # 비면 thread 의 기본값
sandbox = "read-only"

[feature.pr_review]
mode = "observe"              # off | observe | on
route = ["chatgpt_chat", "codex_thread"]
fall_back_on = ["unavailable", "quota", "transient", "interrupted"]
deadline = "30m"

[budget]
max_running = 2
max_started_per_day = 20
```

같은 종류의 백엔드를 이름을 달리해 여럿 둘 수 있다 (예: 모델이 다른 `codex_thread` 두 개). 이것이 "특정 LLM 을 골라
실행한다"의 자리이다.

### 7.2 선택

- `select_backend(경로, 작업의 요구, 능력, 건강, 회로 상태, 시각) → 백엔드 | 없음(이유)` 은 순수 함수이다.
- 요구와 능력을 맞춘다. 예: 작업이 `local_tools` 를 요구하고 tunnel 이 없으면 그 백엔드를 건너뛴다.
- 회로: `quota` 는 `retry_at` 까지 그 백엔드를 닫는다. `transient` 가 연속으로 기준 횟수를 넘으면 정한 시간 동안 닫는다.
- 회로 상태는 장부에 둔다. 재시작 뒤에도 남는다.
- 쓸 백엔드가 없으면 작업은 `failed` 이고 이유는 `route_exhausted` 이다. 조용히 기다리지 않는다.

### 7.3 전달 뒤의 대체

| 부작용 등급 | 뜻 | 전달 뒤의 자동 대체 |
| --- | --- | --- |
| `read_only` | 백엔드는 읽고 글만 낸다 (리뷰, 판정) | 한다. 비용은 사용량뿐이다 |
| `idempotent` | 다시 해도 결과가 같다 | 앞의 시도를 멈춘 뒤에 한다 |
| `side_effecting` | 백엔드가 파일, git, 외부 서비스를 바꾼다 | 하지 않는다. 사람에게 묻는다 |

등급은 기능이 정한다. 작업이 등급을 올릴 수는 있고 내릴 수는 없다.

## 8. ChatGPT 채팅 백엔드

### 8.1 보내는 순서

| 누가 | 하는 일 |
| --- | --- |
| kernel | 1. 건강을 확인한다: Desktop 연결, 로그인, host 임대, 회로, 필요하면 tunnel |
| kernel | 2. 지시를 만든다: 지시, 자료 구역, 결과 형식, 콜백 방법. 작업 ID 와 함께 `submit` 한다 |
| tunnel frontend | 3. 줄을 세운다. 한 번에 하나만 akra-tunnel-core 에 넘긴다 |
| akra-tunnel-core | 4. 세션을 연다: `new` (작업자 대화), `temporary` (임시 대화), `continue` (지정한 대화) |
| akra-tunnel-core | 5. 표식을 붙여 보내고 전달을 확인한다 (`sending` → `accepted` → `observed`) |
| kernel | 6. 대화 참조를 장부에 적는다. 마감 시각마다 `observe` 한다. 근거를 합친다 (§6.3) |
| akra-tunnel-core | 7. 중단되면 복구한다: 새로 고침 (연결 끊김), 재개 (실패, 멈춤) |
| kernel | 8. 끝나면 결과를 받는다. 콜백이 먼저이고, 없으면 표식 뒤의 마지막 답이다 |
| kernel | 9. 복구 한도를 넘으면 대체를 고른다 |

- 표식은 akra-tunnel-core 의 것이다. akra-tunnel-core 가 붙이고 akra-tunnel-core 가 찾는다. kernel 은 작업 ID 만 준다.
- 동시 실행은 tunnel frontend 가 줄을 세운다. host 는 home 하나에 하나이므로 모든 저장소의 작업이 한 줄에 선다. kernel 은
  따로 lease 를 두지 않는다. 줄에서 기다리는 시도의 전달은 `not_sent` 이다.
- 페이지의 "다시 시도" 버튼은 누르지 않는다. 그 버튼은 메시지를 다시 보낼 수 있다 (akra-tunnel `18b4aae`).
- 끝난 작업자 대화는 지우지 않는다 (§15 Q-DE6).

### 8.2 tunnel frontend (Desktop main)

Akra Desktop 은 akra-tunnel-core 의 frontend 이다 (사용자의 방향, 2026-11-21. [D7](29-d7-akra-tunnel-core.md)). 이 문서는 그
부품을 "tunnel frontend" 라고 부른다. host 임대를 쥔 frontend 가 host 이다.

| 쪽 | 가지는 것 |
| --- | --- |
| akra-tunnel-core (라이브러리) | 페이지 읽기와 입력, 세션 감시와 복구, 표식, 전달 확인, 결과 읽기, broker, 세션의 저장 |
| tunnel frontend (Akra Desktop 이 만든다) | 전용 partition 의 보기, 숨긴 창, 이동 제한, `PagePort` 의 구현, 작업의 줄, 화면 |

`ChatSessionPort` 는 kernel 쪽의 port 이다. akra-tunnel-core 의 `ChatSessionService` 를 프로세스 너머로 부른다. 호출은
다섯 개이다. tunnel frontend 는 이것을 akra-tunnel-core 의 호출로 옮긴다.

| 호출 | 하는 일 |
| --- | --- |
| `status` | frontend 가 있는가, host 를 쥐었는가, 로그인했는가, 계정이 맞는가, 줄의 길이, tunnel 이 준비됐는가 |
| `submit(job, session, instruction)` | 줄에 넣는다. 차례가 오면 대화를 열고 보낸다. 같은 작업 ID 는 한 번만 보낸다 |
| `observe(job)` | 전달, 단계, 근거, 복구 상태, 대화 참조를 돌려준다 |
| `read_result(job)` | 표식 뒤의 마지막 답을 돌려준다 |
| `stop(job)` | 멈춘다. 줄에 있으면 뺀다 |

- tunnel frontend 는 전용 partition 의 전용 보기를 쓴다. 사용자의 브라우저 패널과 섞지 않는다. 사용자는 그 보기에서 한 번
  로그인한다. akra-tunnel launcher 의 로그인은 넘어오지 않는다. partition 은 Electron 앱마다 따로이다.
- tunnel frontend 는 akra-tunnel 의 home 을 쓴다. `$AKRA_HOME` 밖이다. MCP 런타임의 신호가 그 home 의 pipe 로 오기
  때문이다 (D3 의 Q-D3-6).
- 계정은 둘일 수 있다. Desktop 의 대화 목록은 Codex 의 토큰으로 읽는다 (`ChatsPort`). tunnel frontend 는 웹 로그인으로
  본다. 계정이 다르면 목록에서 고른 대화를 `continue` 로 열지 못한다. `status` 가 같다와 다르다를 알린다. kernel 은 계정의
  값을 받지 않는다.
- Rust 는 이 라이브러리를 import 하지 못한다. 그래서 akra-tunnel-core 는 Desktop main 에서만 돈다. Desktop 이 꺼지면 세션 감시도 멈춘다.
- 상태 저장소는 둘이다. kernel 의 장부는 작업을 가진다. akra-tunnel-core 의 저장소는 세션을 가진다. kernel 은 akra-tunnel-core 의 저장소를 읽지
  않는다.
- DOM 을 읽는 코드는 akra-tunnel-core 에만 있다. DOM 이 바뀌어 읽지 못하면 단계는 `unknown` 이고 백엔드 건강은 `degraded`
  (`dom_contract_changed`) 이다.

### 8.3 결과가 돌아오는 길

| 길 | 내용 | 언제 |
| --- | --- | --- |
| 콜백 | 백엔드가 셸에서 `akra kernel delegation report … --json` 을 실행한다. 작업별 토큰을 쓴다 | tunnel 이 있을 때. 구조화된 결과와 체크포인트에 맞다 |
| 대화 읽기 | akra-tunnel-core 가 표식 뒤의 마지막 답을 읽는다 | 항상 |

끝의 확인에 `ChatsPort` 를 쓰지 않는다. akra-tunnel-core 가 페이지의 응답에서 끝을 읽는다 (§6.3 의 `cloud`).

### 8.4 tunnel 과의 관계

- tunnel 은 선택이다. 글만 주고받는 작업은 tunnel 이 필요 없다.
- tunnel 의 연결과 감시는 akra-tunnel 에 남는다 (launcher 나 CLI). Akra 는 상태만 읽는다. tunnel frontend 의 `status` 가 akra-tunnel-core 의
  tunnel 상태를 돌려준다. 따로 port 를 두지 않는다. 첫 범위에서 Akra 는 tunnel 을 켜거나 끄지 않는다 (§15 Q-DE16).
- home 하나에 host 는 하나이다. broker 의 pipe 이름이 home 에서 나온다. Akra Desktop 이 host 를 쥐면 MCP 런타임의 도구 호출
  신호 (`call_observed`) 가 Akra Desktop 으로 온다. 그것이 `tunnel` 근거이다. 감시 엔진도 하나가 된다. 그래서 두 앱이 같은
  대화를 재개하지 않는다.
- 대화가 tunnel 앱을 쓰려면 지시에 앱 선택(mention)이 있어야 한다. 이름을 글로 적는 것은 선택이 아니다
  (akra-tunnel `DURABLE_TASK_RUNTIME.md`).
- tunnel 을 쓰는 대화는 이 컴퓨터의 사용자 권한을 가진다. 격리가 없다. 그래서 `tool_access = "tunnel"` 은 사용자
  파일에서만 켜고, 신뢰하지 않는 입력의 작업에는 쓰지 못한다 (§13).

### 8.5 지금의 ChatGPT 코드에서 바꾸는 것

- `ChatsPort` 는 Desktop 의 목록과 읽기 화면만 맡는다. 위임은 이 port 를 쓰지 않는다. 그래서 위임은 조회의 계정 한도를
  쓰지 않는다.
- 설정을 dots 클라우드 플래그에서 떼어 낸다. ChatGPT 기능은 자기 설정을 가진다.
- 읽기 화면에서 그 대화를 tunnel frontend 의 보기로 여는 길을 둔다. 계정이 다르면 이유를 보인다.

## 9. Codex thread 백엔드

- 고립된 새 thread 에서 실행한다. sandbox 는 읽기 전용이고 승인 요청은 거절한다. 슬롯과 worktree 를 쓰지 않는다.
- app-server 와 잇는 길은 기존 것을 쓴다. 턴의 끝 분류를 끝 원인으로 옮긴다.

| Akra 의 값 | 끝 원인 |
| --- | --- |
| `UsageLimitExceeded`, `SessionBudgetExceeded` | `quota` |
| `ServerOverloaded`, `HttpConnectionFailed`, `InternalServerError` | `transient` |
| `ResponseStreamDisconnected`, `Interrupted` | `interrupted` |
| `Unauthorized` | `needs_human` |
| `BadRequest`, `CyberPolicy`, `ContextWindowExceeded` | `rejected` |
| `Unknown` (끝을 가리지 못함) | `unknown` |

- 사용 한도를 미리 읽는 것 (`account/rateLimits/read`) 은 D4 의 뒤 조각이다.

## 10. 기능 바인딩과 첫 기능

- 기능은 코드에 등록한다. 닫힌 목록이다: `{ id, 트리거, 지시 만들기, 결과 모양, 부작용 등급, 결과 처리 }`.
- 저장소가 기능을 켠다. 그 저장소의 kernel 이 떠 있는 동안만 돈다. 수준은 `off`, `observe`, `on` 이다.
- 트리거는 셋이다: 사건 (예: GitHub 관찰자의 PR 사건), 마감 시각 (주기), 사람이나 에이전트의 명령.
- 첫 기능은 **PR 자동 리뷰**이다.
  - 입력: PR 번호, head SHA, diff (크기 한도가 있다).
  - 결과 모양: `findings[{ path, line, severity, body }]`.
  - 부작용 등급: `read_only`.
  - 멱등: `(기능, 저장소, PR, head SHA)` 가 작업 하나이다. 새 push 는 새 작업이고 앞의 작업을 취소한다.
  - 결과 처리: `observe` 는 Desktop 에만 보인다. `on` 은 PR 댓글을 쓴다. 쓰기는 기존 GitHub 쓰기 경로, 마스킹, 일지를
    지난다.
- 다음 소비자: 이슈 초안 검토 (`IssueReviewModelPort`), plan 28 의 판정 모델과 중재자, plan 24 의 게이트 리뷰어.

## 11. 서브에이전트로 쓰기

- 에이전트는 사람과 같은 명령을 쓴다 (KP-04): `akra kernel delegation submit … --json`, `wait`, `status`.
- 방향은 하나이다. 지시가 가고 결과가 온다. 중간 대화는 없다. 중간 신호는 체크포인트뿐이다.
- 작업은 깊이를 가진다. 위임받은 백엔드가 다시 위임하면 깊이가 오른다. 기본 한도는 1 이다 (plan 18 §2.2).
- 콜백은 작업별 토큰을 요구한다. 토큰은 그 작업의 `report` 와 `checkpoint` 만 허락한다.

## 12. kernel 계약 (제안)

이름은 제안이다. 조각마다 다시 정한다. `schema/akra-kernel-spec.json` 과 생성 skill 이 함께 바뀐다.

| 종류 | 이름 | 내용 |
| --- | --- | --- |
| 명령 | `submit_delegation` | 작업을 낸다. 멱등 키가 같으면 같은 작업이다 |
| 명령 | `cancel_delegation`, `retry_delegation` | 취소. 끝난 작업을 다시 낸다 |
| 명령 | `report_delegation_result`, `checkpoint_delegation` | 백엔드의 콜백. 작업별 토큰이 필요하다 |
| 명령 | `resolve_delegation_attention` | 사람이 로그인이나 확인을 끝냈다 |
| 명령 | `set_feature_binding` | 저장소의 기능 수준 |
| 조회 | `delegations`, `delegation` | 목록과 한 건: 상태, 시도, 근거, 결과 |
| 조회 | `delegation_backends` | 백엔드의 능력, 건강, 회로, 줄의 길이 |
| 조회 | `feature_bindings` | 기능, 수준, 유효한 경로와 그 출처 |
| 이벤트 | `delegations`, `delegation_backends` | 바뀜 |
| 기록 | `delegation_submitted`, `delegation_dispatched`, `delegation_delivery_confirmed`, `delegation_interrupted`, `delegation_resumed`, `delegation_fell_back`, `delegation_attention_opened`, `delegation_finished`, `delegation_backend_health_changed` | 해시와 크기만 가진다. 원문이 없다 |
| 이유 | `backend_unavailable`, `backend_quota`, `desktop_not_attached`, `chat_login_required`, `chat_challenge`, `chat_host_busy`, `chat_account_mismatch`, `tunnel_not_ready`, `dom_contract_changed`, `delivery_uncertain`, `no_progress`, `resume_exhausted`, `route_exhausted`, `fallback_forbidden`, `result_invalid`, `budget_exhausted`, `depth_exceeded`, `policy_blocked`, `deadline_exceeded` | `summary` 와 `next` 를 가진다 |

## 13. 안전

| 위협 | 대응 |
| --- | --- |
| PR 이나 이슈의 글 안의 지시문 | 글은 자료 구역으로 감싼다. 신뢰하지 않는 입력의 작업은 `tool_access = "none"` 이다 |
| tunnel 을 쓰는 대화가 이 컴퓨터를 바꾼다 | `tool_access = "tunnel"` 은 사용자 파일에서만 켠다. 그 작업의 등급은 `side_effecting` 이다 |
| 회사 저장소의 내용이 개인 ChatGPT 계정으로 간다 | 백엔드의 허락은 사용자 파일이 정한다. 기본은 꺼짐이다. 켤 때 경고를 보인다 (plan 17 D-14) |
| 같은 지시를 두 번 보낸다 | 전달 단계, 표식, `outcome_unknown`. 결과를 모르면 다시 보내지 않는다 |
| 대체가 부작용을 두 번 낸다 | 부작용 등급 (§7.3) |
| 백엔드의 결과가 kernel 을 조종한다 | 결과는 자료이다. 모양을 확인한다. 밖으로 쓰는 일은 기능이 한다 |
| 계정의 요청 한도와 자동 조작의 위험 | 동시 실행 1, 보내기 간격, 예산, 회로. 챌린지는 사람에게 넘긴다. 우회하지 않는다 |
| 위임이 위임을 낳는다 | 깊이 한도, 예산 |
| 가짜 콜백 | 작업별 토큰. 토큰은 지시 봉투에만 있다 |
| 사용자의 대화를 잘못 조작한다 | akra-tunnel-core 는 Akra 가 연 대화와 사용자가 지정한 대화만 다룬다. 보내기 직전에 대상 주소를 확인한다. 다르면 보내지 않는다 |
| 모델 사용량 | 시험은 모델을 부르지 않는다 (D-13). 제품은 기능의 수준과 예산이 한도이다 |

## 14. 조각과 순서 (제안)

메인 이슈 하나가 이 문서이다. 조각 문서 하나가 서브 이슈의 묶음이다. 하위 번호 하나가 서브 이슈 하나이고 PR 하나이다.

| 조각 | 내용 | 동작 변화 | 선행 |
| --- | --- | --- | --- |
| [D1](29-d1-contract-and-ledger.md) | domain 의 상태와 순수 함수, 장부, `AgentBackendPort`, 가짜 백엔드, 조정기, kernel 범위 `delegation` | 없다. 가짜 백엔드로만 돈다 | - |
| [D2](29-d2-codex-thread-backend.md) | `codex_thread` adapter, 첫 소비자 (이슈 초안 검토) | 이슈 초안 검토가 실제 모델로 돈다 | D1 |
| [D3](29-d3-chatgpt-chat-backend.md) | tunnel frontend, Rust 와 Desktop 의 호출 길, `chatgpt_chat` adapter, ChatGPT 화면의 정리, tunnel | ChatGPT 채팅으로 작업이 간다 | D1, D7 |
| [D4](29-d4-routing-and-fallback.md) | 정책 파일, 건강과 회로, 선택, 대체, 예산 | 백엔드가 막히면 다음으로 넘어간다 | D1. 끝-끝 확인은 D2, D3 |
| [D5](29-d5-feature-binding-and-pr-review.md) | 기능 등록부, 저장소 바인딩, PR 자동 리뷰 | 저장소에서 PR 리뷰를 켠다 | D2 또는 D3, D4 |
| [D6](29-d6-agent-entry-and-desktop.md) | 에이전트 명령과 skill, 콜백, Desktop 화면 | 에이전트가 위임한다. 화면에 보인다 | D1. 콜백은 D3 |
| [D7](29-d7-akra-tunnel-core.md) | 새 저장소와 akra-tunnel 의 일: akra-tunnel-core 와 frontend 나누기, 페이지 키트, host 가르기와 임대, 글 작업 보내기, 배포 | akra-tunnel 의 동작은 같다. Akra 가 akra-tunnel-core 를 import 할 수 있다 | - |

- D7 은 다른 저장소의 일이다. D1 과 같이 시작할 수 있다. D7 의 서브 이슈는 그 일이 있는 저장소에 둔다
  (`RefinedStone/akra-tunnel-core`, `akra-devs/akra-tunnel`).
- D3 은 D7 을 기다린다. D2 와 D4 의 순수 함수는 D1 뒤에 바로 할 수 있다.
- 모든 조각은 kernel 시뮬레이션 시험 (`src/composition/kernel_host/simulation.rs`) 을 통과한다. 시험은 가짜 백엔드와
  가짜 시계를 쓴다. 모델 턴이 없다.
- 실제 계정의 확인은 조각마다 사용자가 허락한 한 번만 한다 (D-13).

### 14.1 조각 상태 (2026-11-26, `origin/prerelease@21a3d0974`)

소스와 문서를 대조한 결과이다. 실행 재현은 하지 않았다. 남은 조각은 이 저장소의 이슈로 추적하고, D7 은 다른 저장소가 추적한다.

| 조각 | 상태 | 근거 | 남은 것 |
| --- | --- | --- | --- |
| D1 계약과 장부 | 구현됨 | `src/domain/delegation.rs` (상태, 전달, 단계, 시도, 근거), `src/application/port/outbound/agent_backend_port.rs`, `src/adapter/outbound/db/sqlite_delegation_store.rs`, kernel 조회 `delegation`·`delegations` | 없음 |
| D2 Codex thread 백엔드 | 구현됨 | `src/adapter/outbound/app_server/codex_thread.rs` (격리 읽기 전용 thread, `UsageLimitExceeded` → `quota`), `akra codex-delegation login`, 이슈 판정 소비자 (`issue_assessment_model.rs`) | 없음 |
| D3 ChatGPT 채팅 백엔드 | 부분 | Desktop 구동기와 tunnel launcher schema 23, 계정 일치, 로그인 보기 (current-product 위임 절) | 켜기가 `AKRA_DESKTOP_DELEGATED_CHAT=1` 에 묶여 있다 (#2879) |
| D4 경로와 대체 | 대부분 | `src/domain/delegation/policy.rs` (백엔드 순서, `tool_access`, 사용자 상한을 넘지 못하는 저장소 값, `max_running`, `max_started_per_day`), `BackendStatus` 의 `circuit`, `Fallback` 이력 | D4-4 미리 보는 건강(계정 사용 한도 읽기)은 #2821·#2859 가 같은 API 를 다룬다 |
| D5 기능 바인딩과 PR 리뷰 | 구현됨 | `src/composition/pr_review.rs`, [PR 리뷰 백엔드](../reference/pr-review-backend.md) (기본 꺼짐, 도구 없는 모델 경로, 명시적 게시 설정) | 없음 |
| D6 에이전트 입구와 화면 | 대부분 | `akra kernel delegation submit_delegation`·`wait`·`status`·`result`, 콜백 `checkpoint_delegation`·`report_delegation_result` 과 작업별 토큰, skill `assets/app-server/hub-skills/akra-kernel/SKILL.md`, Desktop "ChatGPT 위임" 패널과 "내 차례" | 깊이 한도가 없다 (#2880) |
| D7 akra-tunnel-core | 다른 저장소 | akra-tunnel-core v0.1.2, launcher PR #94 (PR #2639 본문) | `RefinedStone/akra-tunnel-core` #1 ~ #8 이 추적한다. D3 의 남은 일(#2879)은 그 저장소의 배포 상태를 확인하고 정한다 |

## 15. 결정 (제안)과 미결정

결정은 모두 제안이다. 사용자가 답하면 상태와 날짜를 고친다.

| ID | 결정 | 상태 |
| --- | --- | --- |
| DE-01 | 이름은 "위임 실행"이다. 용어는 §2 를 쓴다. kernel 범위는 `delegation` 이다 | 제안 |
| DE-02 | 백엔드는 `AgentBackendPort` 하나로 붙는다. 호출은 짧고, 긴 일은 관찰로 본다 | 제안 |
| DE-03 | 상태는 작업, 시도(전달, 단계, 끝 원인), 진행 근거의 닫힌 목록이다 (§6) | 제안 |
| DE-04 | 작업의 판단(경로, 대체, 예산, 마감)은 kernel 의 순수 함수에 있다. 세션의 복구는 백엔드 안에 있다 | 제안 |
| DE-05 | ChatGPT 로 보내기는 Desktop 의 앱 안 브라우저만 한다. Gateway 는 보내지 않는다 | 제안 |
| DE-06 | 대체는 끝 원인과 부작용 등급으로 정한다. 결과를 모르면 다시 보내지 않는다 | 제안 |
| DE-07 | 백엔드의 허락과 `tool_access` 는 사용자 파일에서만 온다 | 제안 |
| DE-08 | 주 실행 경로(사용자 대화, 병렬 슬롯)는 바꾸지 않는다 | 제안 |
| DE-09 | tunnel 은 선택이다. Akra 는 tunnel 의 상태만 읽는다 | 제안 |
| DE-10 | akra-tunnel 을 akra-tunnel-core 라이브러리와 frontend 로 나눈다. Akra Desktop 은 akra-tunnel-core 를 import 하고 자기 frontend 를 만든다. akra-tunnel-core 는 새 public 저장소 `RefinedStone/akra-tunnel-core` 이다 | 사용자의 방향 (2026-11-21) |
| DE-11 | 이름은 `akra-tunnel-core` 이다. 범위는 akra-tunnel 에서 frontend 를 뺀 전부이다. 영역은 subpath (`/chat`, `/tunnel`) 로 나눈다. Akra 의 문서와 코드는 `core` 로 줄여 쓰지 않는다 ([D7](29-d7-akra-tunnel-core.md) 의 "이름") | 제안 |

| ID | 질문 | 기본값 (답이 없을 때) |
| --- | --- | --- |
| Q-DE1 | 세션의 복구(새로 고침, 재개)를 akra-tunnel-core 에 두는가, kernel 에 두는가 | akra-tunnel-core 에 둔다. 있는 규칙을 Rust 로 다시 쓰지 않는다. kernel 은 복구 상태만 본다 |
| Q-DE2 | 장부와 조정기의 주인은 저장소 kernel 인가, Gateway (머신) 인가 | 저장소 kernel. `chatgpt_chat` 의 줄과 한도만 머신 전체이고 tunnel frontend 가 가진다 |
| Q-DE3 | Desktop 이 꺼져 있을 때 `chatgpt_chat` 은 어떻게 하는가 | `unavailable` 이고 다음 백엔드로 간다. 숨긴 창을 스스로 띄우지 않는다 |
| Q-DE4 | `tool_access = "tunnel"` 을 언제 허락하는가 | 기본은 꺼짐. 사용자 파일에서 기능별로 켠다. 신뢰하지 않는 입력에는 쓰지 못한다 |
| Q-DE5 | 기본 세션은 무엇인가 | `new` (작업자 대화). 임시 대화는 D7-5 (akra-tunnel-core #7) 에서 동작을 확인한 뒤에 연다 |
| Q-DE6 | 끝난 작업자 대화를 지우는가 | 지우지 않는다 |
| Q-DE7 | `interrupted` 뒤에 재개가 먼저인가, 대체가 먼저인가 | 재개 2회가 먼저이다. 그 뒤에 등급에 따라 대체한다 |
| Q-DE8 | 주기 트리거(스케줄)를 첫 범위에 넣는가 | 넣지 않는다. 사건과 명령만 한다. 주기는 D5 뒤이다 |
| Q-DE9 | 지시와 결과의 원문을 얼마나 두는가 | 끝난 뒤 7일. 크기 한도가 있다 |
| Q-DE10 | PR 리뷰의 결과를 PR 에 쓰는가 | 첫 수준은 `observe` 이다. Desktop 에만 보인다 |
| Q-DE11 | 어느 저장소의 내용을 ChatGPT 계정으로 보내도 되는가 | 사용자 파일이 저장소별로 허락한 것만 |
| Q-DE12 | akra-tunnel launcher 와 Akra Desktop 이 같이 떠 있을 때 누가 대화를 조작하는가 | host 를 쥔 쪽 하나이다. home 하나에 host 하나이다. 다른 쪽은 client 이다 (D7-4) |
| Q-DE13 | 병렬 슬롯의 작업 지시도 위임 백엔드로 보내는가 (plan 28 IA9) | 아니요. 뒤에 따로 정한다 |
| Q-DE14 | `chatgpt_chat` 의 동시 실행 수 | 1 |
| Q-DE15 | 사람의 확인(로그인 끝남 등)을 무엇으로 증명하는가 | plan 28 Q-IA11 과 같다: Desktop main 의 확인을 지난 값 |
| Q-DE16 | Akra Desktop 이 tunnel 의 연결과 감시도 맡는가 | 아니요. 첫 범위에서 tunnel frontend 는 대화 세션만 맡는다. tunnel 은 akra-tunnel 의 launcher 나 CLI 가 맡는다 |

## 16. 하지 않는 것

- sentinel, Turnstile, CAPTCHA 의 우회. 지문 위장. 비공개 보내기 API 의 직접 호출.
- 사용자가 평소 쓰는 Chrome 의 조작. 자격 증명의 자동 입력.
- 백엔드와의 다중 턴 대화. 위임은 지시 하나와 결과 하나이다. 재개는 복구이다.
- tunnel 런타임의 설치, 시작, 중지.
- 여러 백엔드의 결과를 합치는 것 (합의, 투표).
- 사용자 대화와 병렬 슬롯 실행을 위임으로 바꾸는 것.
- webhook 서버.
- Desktop 없이 도는 tunnel frontend (headless 브라우저).

## 17. 확인하지 않은 것

- Orca 의 스케줄러 구현. 읽지 않았다. 이 문서는 사용자의 말("특정 LLM 을 골라 실행한다")만 반영했다.
- Akra Desktop 의 Electron 보기에서 chatgpt.com 에 로그인하고 보낼 수 있는가. akra-tunnel 의 launcher 는 Electron
  내장 브라우저로 한다. Akra Desktop 에서는 시험하지 않았다.
- 숨긴 보기. akra-tunnel 의 주석은 `setVisible(false)` 와 화면 밖 보기가 ChatGPT 의 DOM 갱신을 멈춘다고 적는다. 그래서
  akra-tunnel 은 숨긴 창 안에 보이는 보기를 둔다 (`continuation-browser-host.mjs`). Akra Desktop 에서는 시험하지 않았다.
- 임시 대화에서 앱(tunnel)과 도구를 쓸 수 있는가.
- 한도 알림과 챌린지 화면의 DOM. akra-tunnel 의 정규식은 한국어와 영어 일부만 확인했다.
- `desktop_browser_bridge` 를 tunnel frontend 의 호출 길로 다시 쓸 수 있는가. 머리 부분만 읽었다.
- host 를 쥔 Akra Desktop 에 `call_observed` 가 실제로 오는가. 코드의 경로만 읽었다 (`continuation-call-context.ts`,
  `continuation-browser-host.mjs`).
- akra-tunnel 의 `image-browser-host.mjs`. import 와 일부 패턴만 읽었다. 새 대화를 여는 흐름이 거기에 있다.
- akra-tunnel 의 파일을 옮길 때 런타임 overlay 의 해시 검증이 받는 영향 (D7).
- `account/rateLimits/read` 의 모양. Akra 는 아직 부르지 않는다.
- GitHub 관찰자가 PR 사건을 kernel 사건 기록에 내는가. plan 20 §4.1 의 그림만 봤다.
- 서비스 약관과 계정 위험. 사용자가 판단한다. 켤 때 경고를 보인다 (plan 25 §8).
- 구현 크기. 조각의 닿는 곳만 정했다.
