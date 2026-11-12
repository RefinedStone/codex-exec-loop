# K4 상세 설계: 명령 스펙 하나와 입구 통일

## 문서 상태

- 상태: **Proposed design**. 구현은 없다. 현재 제품 동작을 설명하지 않는다. §6 의 미결 질문에 답이 없으면 기본값으로
  진행한다.
- 상위 계획: [18 Akra 커널 아키텍처](18-akra-kernel-architecture.md) §4.3, §4.4, §5.5, §5.6, §6 K4. 결정 KP-04,
  KP-05 (승인 2026-11-07). [19 Akra 커널 성질 목표](19-akra-kernel-quality-goals.md) §5.4, Q4, 성질 P3.
- 이전 단계: [K3](18-k3-kernel-daemon-protocol.md) (프로토콜 v0, 완료 2026-11-10), plan 19 Q3 (사건 기록, 승인 기록,
  연산 영수증, 멱등 키, 완료 2026-11-11). plan 19 Q-Q1 에 따라 사건 기록(K5 의 기반)을 스펙보다 먼저 했다.
- 기준일: 2026-11-12 (Asia/Seoul)
- 기준 커밋: `aa85888d` (`origin/prerelease`).
- 범위: 커널의 명령, 조회, 이벤트, 기록, 이유 코드, 오류 코드를 한 번 정의한다. 그 정의에서 자기 설명, Desktop 타입,
  에이전트용 CLI, skill 문서를 만든다. CLI, admin, Telegram 의 변경 명령을 커널 경로로 옮긴다. 얇은 TUI(K6), thread
  액터(Q5), 관찰자(plan 20), 묻기·답하기와 승인 정책은 범위 밖이다.

## 1. 목표

| # | 목표 | 확인 방법 |
| --- | --- | --- |
| G1 | 사실 하나는 한 곳에만 적는다. 명령, 조회, 이벤트, 기록 종류, 이유 코드, 오류 코드의 이름과 모양과 뜻이다 | 손으로 적은 이름 목록과 손으로 복사한 타입이 없다 (§2.1 의 표가 빈다) |
| G2 | 커널이 스스로를 설명한다 (P3). 무엇을 할 수 있는가, 입력은 무엇인가, 왜 거절했는가, 다음에 무엇을 하는가 | 문서를 읽지 않은 프로그램이 `describe` 와 기록 조회만으로 프롬프트를 내고, 거절에 대응하고, 끝을 안다 (§3.9 의 시험) |
| G3 | 모든 입구가 같은 명령을 쓴다 (KP-04). CLI, admin, Telegram, 에이전트의 변경 명령이 커널을 거친다 | 입구 adapter 가 변경 서비스를 직접 부르지 않는다 (구조 시험). K-D3, K-D4, K-D5 가 닫힌다 |
| G4 | 에이전트의 기본 입구는 `akra kernel ... --json` 과 생성된 skill 이다 (KP-05) | 에이전트가 셸만으로 커널에 명령하고 안정된 JSON 을 받는다 |

## 2. 현재 상태 (2026-11-12 코드 조사)

### 2.1 같은 사실이 여러 곳에 있다

| 사실 | 적힌 곳 | 문제 |
| --- | --- | --- |
| 명령 이름 | `KernelCommand` enum, `COMMANDS` 목록, 시험의 표본 목록 (`kernel_protocol/mod.rs`, `tests.rs`) | 시험은 개수만 본다. 새 variant 를 목록에 빠뜨려도 통과한다 |
| 조회, 이벤트 이름 | `KernelQuery`, `KernelEvent` enum 과 `QUERIES`, `EVENTS` 목록 | `EVENTS` 에 `history` 가 없다. 시험이 길이를 보지 않는다 |
| 기록 종류 | `domain::kernel_event::KernelEvent`, `kind()`, `KERNEL_EVENT_KINDS` | 표본 시험이 지킨다. 컴파일러가 강제하지 않는다 |
| 프롬프트를 준비 전에 거절한 이유 5개 | `CommandRejectionReason`, `ThreadPromptRejection`, `PromptSubmissionReason` | enum 3개. `ThreadPromptAdmission` 에서 세 번 따로 바꾼다. 설명 문장도 두 곳에 있다 |
| "다음에 무엇을 하는가" | 이유 enum 의 주석 | 주석이다. 클라이언트에 가지 않는다 |
| 프로토콜 타입 | Rust 타입과 `desktop/src/renderer/kernel/protocol.ts` | 손으로 복사했다. `operation`, `history`, `recorded`, `describe` 가 빠졌고 이유 코드가 `string` 이다 |
| 명령의 성질 (읽기인가, 기록하는가, thread 를 가리키는가, 확인이 필요한가) | `descriptor()` 의 match, `into_core` 의 검사, 주석 | 한 표가 없다 |
| 병렬 상태 값 | `ParallelAgent.state` 등 | 자유 글자이다. K3 이 "K4 에서 고친다"고 남겼다 |

스키마 생성과 코드 생성 기반은 없다 (`schemars` 없음, `build.rs` 없음). `schema/` 는 Codex app-server 의 스키마만
가진다. Akra 는 MCP 서버가 아니다. MCP 는 hub 로만 쓴다.

### 2.2 입구가 커널을 거치지 않는다

`akra` CLI, admin HTTP, Telegram bot 은 커널(`KernelHost`, `AppCommand`, `akra/kernel/*`)을 쓰지 않는다. 각자
`composition/production.rs` 로 application 서비스를 조립해 직접 부른다. `akra mcp` 만 Gateway 에 붙는다.

| 일 | 구현된 입구 | 차이 |
| --- | --- | --- |
| planning 초기화 | CLI `reset`, admin `/api/planning/reset`, Telegram `/reset`, 커널 `reset_planning`, TUI | 대상 읽기와 확인 규칙을 네 번 썼다. 끝은 같은 서비스이다 |
| 작업 만들기·고치기 | admin (revision 검사 있음), `akra planning-tool run` (검사 없음, 숨은 planning worker 가 쓴다), TUI | revision 규칙이 입구마다 다르다 |
| 병렬 모드 켜기·끄기·배분 | admin `/api/admin/akra/control`, TUI | 커널 프로토콜에 없다. admin 은 자기 프로세스의 병렬 제어부를 움직인다 (K-D3) |
| 수동 배분 한 번 | CLI `parallel-tick` | CLI 에만 있다 |
| PR 검증 명령 | admin `/api/admin/akra/validations/{k}/commands` (revision 검사) | admin 에만 있다 |
| 초안, 방향, 파일 내보내기·적용 | admin (HTML 과 API), TUI | - |
| 상태·큐 읽기 | CLI `status`, `queue`, Telegram, admin | 읽기 명령도 서비스를 조립하면서 프롬프트 기록 정리(지우기)를 실행한다 |

- 이 입구들은 저장소 잠금(`kernel.lock`)을 잡지 않는다. 그런데 admin, Telegram, `parallel-tick` 은 병렬 제어부를
  조립하면서 PR 검증 스케줄러 스레드를 시작한다 (K-D5 의 남은 부분).
- 기계용 출력이 없다. CLI 에 `--json` 이 없고, Telegram 은 한국어 글, admin 은 경로마다 다른 JSON 이다. 실패는 종료
  코드 1 과 글이다.
- 이 입구의 명령은 판단 사건 기록에 남지 않는다 (K-D4).

### 2.3 이전 단계가 K4 로 넘긴 일

| 넘긴 일 | 출처 | 이 설계의 자리 |
| --- | --- | --- |
| 프로토콜을 스펙으로 만들면서 v1 로 올린다 | K3 §5 | K4-1 |
| 병렬 상태 값을 타입으로 | K3 (`kernel_protocol` 주석) | K4-1 |
| 이유 코드와 다음 행동 | plan 19 §5.4 | K4-2 |
| 완료로 보여 버린 프롬프트의 이유 코드 (`task_finished`) | plan 19 Q2-3 | K4-2 |
| 연산이 끝났는지 | plan 19 Q3-6 | K4-6 |
| 누가 명령했는지, 누가 승인을 결정했는지 | plan 19 Q3-5 | K4-7 |
| 보지 않는 thread 에 낸 턴 제출을 커널이 막지 않는다 | plan 19 Q3-7 | K4-7 (명령의 전제 조건) |
| admin 의 병렬 정지가 커널에 닿지 않는다 (K-D3), 스케줄러가 여러 프로세스에서 돈다 (K-D5), 감사 기록 (K-D4) | plan 18 §5.6 | K4-8 |

## 3. 설계

### 3.1 스펙의 자리: Rust 가 정본이고, 내보낸 JSON 이 데이터이다

두 방법을 비교했다.

| 방법 | 장점 | 단점 |
| --- | --- | --- |
| A. 데이터 파일(YAML 등)을 정본으로 두고 Rust 타입을 생성한다 | 언어에 묶이지 않는다 | 빌드에 Rust 생성 단계가 생긴다. `match` 의 빠짐을 컴파일러가 잡지 못한다. 한 번에 모든 타입을 바꿔야 한다 |
| **B. Rust 타입과 Rust 표를 정본으로 두고, 스펙 JSON 을 내보낸다 (채택)** | 지금 타입을 그대로 쓴다. 컴파일러가 빠짐을 잡는다. 조각마다 옮길 수 있다 | Rust 밖의 대상은 내보낸 JSON 에서 만든다. JSON 이 낡지 않게 지키는 시험이 필요하다 |

채택한 구조:

```text
domain::kernel_spec        명령·조회·이유의 표 (이름, 한 줄 설명, 효과, 대상, 거절 이유, 다음 행동, 끝의 규칙)
kernel_protocol (inbound)  전송 모양 (serde 타입). 타입에서 JSON Schema 를 만든다 (schemars)
        │  spec() = 표 + 스키마
        ▼
akra/kernel/describe  ──▶  schema/akra-kernel-spec.json (저장소에 둔다. 시험이 spec() 과 같은지 본다)
                                   │
        ┌──────────────┬───────────┼──────────────┬───────────────────┐
        ▼              ▼           ▼              ▼                   ▼
   Desktop TS 타입   akra kernel   skill 문서    MCP 도구 (제안)   SDK (제안)
```

- 표는 domain 에 둔다. 효과 등급과 확인 규칙을 커널이 검사하기 때문이다 (plan 18 §4.4: 권한 검사는 입구가 아니라
  커널에서). 전송 모양은 지금처럼 inbound adapter 에 둔다.
- 표의 항목은 enum variant 를 받는 빠짐없는 `match` 로 만든다. variant 를 더하면 표를 채울 때까지 컴파일되지 않는다.
  손으로 적은 `COMMANDS`, `QUERIES`, `EVENTS`, `KERNEL_EVENT_KINDS` 는 지운다.
- `schema/akra-kernel-spec.json` 이 plan 19 §5.4 의 "데이터로 한 번만"이다. Rust 가 이 파일을 만들므로 Rust 와
  어긋날 수 없다. 시험이 파일과 `spec()` 을 견준다. 파일을 고치는 명령은 하나이다 (`scripts/` 에 둔다).
- 새 의존성은 `schemars` 하나이다 (plan 18 §4.4 가 이미 정했다). 미결 Q-K4-1.

### 3.2 스펙의 내용

```text
KernelSpec { spec_version, protocol_version, commands[], queries[], events[], records[], reasons[], errors[] }

Command {
  name            "submit_prompt"
  group           "thread"                  CLI 의 묶음이다: akra kernel thread submit-prompt
  summary         한 문장. 에이전트가 읽는다
  input           JSON Schema
  effect          read | write | destructive
  confirm         true 이면 입력에 confirm: true 가 필요하다
  target          none | thread | approval | task
  recorded        판단 사건 기록에 남는가 (읽기 명령은 남지 않는다)
  rejections      이 명령이 낼 수 있는 이유 코드
  completes_on    이 명령의 끝을 알리는 기록 (§3.4)
}
Query  { name, summary, params (JSON Schema), result (JSON Schema) }
Event  { type, summary, payload (JSON Schema) }
Record { kind, summary, payload (JSON Schema) }
Reason { code, meaning, next[] }
Error  { code, name, meaning, next[] }
```

`next` 는 글이 아니라 데이터이다. 클라이언트가 그대로 실행할 수 있는 것만 둔다.

| `next` 의 종류 | 뜻 | 예 |
| --- | --- | --- |
| `wait_for { event }` | 그 이벤트가 조건을 만족할 때까지 기다린다 | `busy` → `thread_activity` 가 빌 때까지 |
| `command { name }` | 그 명령을 보낸다 | `unknown_thread` → `load_sessions` |
| `query { name }` | 그 조회로 지금 값을 읽는다 | `no_matching_turn` → `thread_activity` |
| `resend { new_key }` | 같은 명령을 새 멱등 키로 다시 보낸다 | `steer_in_flight` 가 끝난 뒤 |
| `ask_user` | 사람이 정해야 한다 | `bootstrap_review_required` |
| `none` | 다시 하지 않는다 | `text_mismatch` |

### 3.3 이유 코드는 하나의 목록이다

- `CommandRejectionReason`, `ThreadPromptRejection`, `PromptSubmissionReason` 을 domain 의 `ReasonCode` 하나로 합친다.
  전송되는 글자(`busy` 등)는 바꾸지 않는다. 그래서 기록과 v0 클라이언트가 그대로 읽는다.
- core 는 `ThreadPromptAdmission` 을 이유 코드로 한 번만 바꾼다. 설명 문장은 표에서 온다.
- 답에 `next` 를 싣는다: 명령 답의 `rejection.next`, `prompt_submission` 이벤트의 `next`. 값은 스펙의 그 이유와 같다.
- 넘겨받은 `task_finished` (완료로 보여 버린 프롬프트)를 여기서 더한다.

### 3.4 연산의 끝

Q3-6 의 영수증은 받아들임과 바로 거절까지만 알린다. 끝은 명령마다 다르므로 스펙이 정한다.

- 스펙의 `completes_on` 이 명령마다 끝을 알리는 기록을 적는다.

  | 명령 | 끝 |
  | --- | --- |
  | `set_auto_follow_max_turns`, `pause_auto_follow` | 받아들이면 끝이다 |
  | `submit_prompt` | 준비 전 거절, 준비 실패, 큐에 남김(`thread_prompt_queued`), 또는 그 턴의 `turn_ended` |
  | `steer_turn`, `decide_approval` | 그 steer 나 결정의 완료 (기록을 더해야 한다) |
  | `stop_all` | 도는 턴이 모두 `turn_ended` 가 된다 |
  | `reset_planning` | 초기화 결과 (기록을 더해야 한다) |

- 조회 `operation { op_id }` 를 더한다. 답은 `state`(`accepted`, `rejected`, `running`, `done`, `failed`)와 그 연산의
  기록이다. 따로 저장하지 않는다. `op_id` 뒤의 기록에서 계산한다 (plan 19 §5.1.2 의 원칙 4).
- 명령이 만든 기록은 `op_id` 를 싣는다. 지금은 턴이 한 번에 하나라서 thread 와 순서로 짝을 맞출 수 있다. 그러나 Q5 에서
  동시 턴이 되면 맞지 않으므로 처음부터 `op_id` 를 싣는다. core 는 이미 있는 correlation(프롬프트 준비, 턴 제출)에
  `op_id` 를 함께 싣는다.

### 3.5 호출자와 효과 등급

- 호출자는 접속이다. `attach` 의 `client_name` 과 종류(`desktop`, `cli`, `admin`, `telegram`, `agent`)를 커널이
  `command_received.caller` 에 적는다. 승인 결정도 명령이므로 누가 결정했는지 남는다.
- 커널은 스펙의 `effect` 와 `confirm` 을 검사한다. 입구는 검사하지 않는다. `destructive` 명령은 `confirm: true` 가
  없으면 거절한다 (지금의 `reset_planning` 규칙을 일반화한다, K-D1).
- 명령의 전제 조건(예: 보지 않는 thread 에 낸 턴 제출)은 커널이 검사하고 이유 코드로 거절한다.
- 호출자마다 다른 권한(예: 에이전트는 `destructive` 를 못 한다)은 이 설계에 넣지 않는다. 에이전트 자율성의 한계
  (plan 18 Q-K5)와 함께 정한다. 스펙에는 `effect` 가 있으므로 그 정책은 표 하나로 쓸 수 있다.

### 3.6 생성하는 것

| 대상 | 방법 | 비고 |
| --- | --- | --- |
| 자기 설명 | `akra/kernel/describe` 가 `KernelSpec` 을 답한다. 프로토콜 판을 1 로 올린다 | Desktop 은 지금 `describe` 를 부르지 않는다. 그래서 답의 모양을 바꿔도 깨지는 클라이언트가 없다 |
| Desktop 타입 | `desktop/scripts/` 의 생성기가 `schema/akra-kernel-spec.json` 에서 TS 타입을 만든다. `protocol.ts` 의 손 복사를 지운다 | 이유 코드와 상태 값이 `string` 이 아니라 합 타입이 된다 |
| 에이전트용 CLI | `akra kernel <묶음> <명령> [--필드 값] --json`, `akra kernel describe`, `akra kernel query <이름>`, `akra kernel history --after <n> [--wait]`, `akra kernel operation <op_id> [--wait]` | 코드를 생성하지 않는다. binary 안의 스펙으로 인자를 읽는다. Gateway 에 `akra mcp` 와 같은 방법으로 붙는다 |
| skill 문서 | 스펙에서 `assets/app-server/hub-skills/akra-kernel/SKILL.md` 를 만든다. 저장소에 두고 시험이 스펙과 견준다 | `akra-mcp-hub` skill 과 같은 방법으로 등록한다 (`skills/extraRoots/set`) |
| MCP 도구 | 위험한 조작 몇 개만 (KP-05) | 제안. 이 단계에서 하지 않는다 |
| SDK (TypeScript, Python) | 스펙 JSON 에서 | 제안. 이 단계에서 하지 않는다 |

에이전트용 CLI 의 계약:

- 출력은 한 줄 JSON 이다. 사람용 글과 섞지 않는다.
- 종료 코드: 0 = 받아들임, 2 = 커널이 거절함 (JSON 에 이유와 `next`), 1 = 닿지 못함 또는 잘못된 입력.
- `--idempotency-key` 를 주지 않으면 CLI 가 무작위 키를 만들어 답에 싣는다. 응답을 받지 못한 호출자는 그 키로 다시
  보낸다.
- 지금의 사람용 명령(`akra status`, `queue`, `reset`, `doctor`)은 남긴다. 변경 명령은 안에서 커널 명령을 쓴다 (§3.7).

### 3.7 입구를 커널 경로로 옮긴다

일의 묶음마다 커널 명령과 조회를 스펙에 더하고, 입구가 그 명령을 Gateway 로 보내게 바꾼다. 순서는 결함이 큰 것부터이다.

| 순서 | 일 | 더하는 커널 명령·조회 | 닫는 결함 |
| --- | --- | --- | --- |
| 1 | planning 초기화 | 있는 `reset_planning` 을 쓴다 | 네 번 쓴 대상·확인 규칙, K-D4 |
| 2 | 병렬 모드 켜기·끄기·배분·새로 고침, 수동 배분 | `set_parallel_mode`, `dispatch_parallel`, `refresh_parallel` | K-D3 (정지가 커널에 닿는다), K-D5 (입구가 스케줄러를 시작하지 않는다) |
| 3 | PR 검증 명령 | `pr_validation_command { key, action, expected_revision }` | K-D4 |
| 4 | 작업·방향 만들기·고치기·지우기, 초안과 승격, 파일 내보내기·적용 | `upsert_task`, `delete_task`, `upsert_direction`, … 모두 `expected_planning_revision` 필수 | `planning-tool` 의 revision 검사 없음 (K-D2 의 남은 입구), K-D4 |
| 5 | 상태·큐 읽기 | 있는 `planning`, `parallel` 조회와 새 `pr_validation` 조회 | 읽기 명령이 쓰기를 한다 (프롬프트 기록 정리). 정리는 커널이 한다 |

막는 것 두 가지와 그 답:

1. **Gateway 가 떠 있지 않을 때.** 지금 CLI 는 Gateway 없이 동작한다. 옮긴 뒤에도 그래야 한다. 답: 입구는 먼저
   Gateway 에 붙는다. 붙지 못하면 CLI 는 한 번 쓰는 커널을 자기 프로세스에서 조립한다 (`build_kernel_for_workspace`:
   잠금을 잡고, 명령 하나를 처리하고, 기록을 다 쓰고, 내린다). 그래서 명령은 어느 경우에도 커널의 같은 경로와 같은
   기록을 거친다. admin 과 Telegram 은 오래 도는 서버이므로 Gateway 를 요구한다 (미결 Q-K4-4).
2. **TUI 가 그 저장소의 커널을 쥐고 있을 때.** TUI 는 자기 프로세스에서 커널을 돌리고 소켓을 열지 않는다 (K3 §3.5).
   다른 입구는 그 커널에 닿지 못한다. TUI 가 Gateway 의 커널에 붙는 일은 K6 이다. 답 (기본값, 미결 Q-K4-3): K6
   전까지 이 경우에만 지금의 직접 경로를 남긴다. 그 경로는 "TUI 가 잠금을 쥐었다"를 확인한 뒤에만 탄다. 기록에는
   남지 않고, K-D3 과 K-D5 는 TUI 를 쓰는 동안 남는다. K6 에서 지운다.

### 3.8 하지 않는 것

- TUI 를 커널 프로토콜의 클라이언트로 바꾸지 않는다 (K6).
- 동시 턴과 thread 액터를 만들지 않는다 (Q5).
- 묻기·답하기, 승인을 사람·에이전트·정책으로 보내는 일, 관찰자(plan 20)를 만들지 않는다. 스펙은 그 명령과 기록이
  들어올 자리(`group`, `effect`, `records`)를 가진다.
- dry-run, 예산, 재귀 깊이 같은 안전장치(plan 18 §4.3 의 7)를 만들지 않는다.
- admin 의 HTML 화면을 다시 만들지 않는다. 화면은 그대로 두고 변경 요청만 커널 명령으로 보낸다.

### 3.9 경계 시험

| 시험 | 내용 |
| --- | --- |
| 스펙 파일 | `schema/akra-kernel-spec.json` 이 `spec()` 과 같다. skill 문서와 Desktop 생성 타입도 스펙과 같다 |
| 빠짐 없음 | 명령·조회·이벤트·기록·이유의 표가 빠짐없는 `match` 이다. 모든 이유에 `next` 가 있다 (`none` 도 적는다) |
| 거절의 완전성 | 커널 시뮬레이션에 불변식을 더한다: 커널이 명령에 낸 모든 이유 코드는 그 명령의 스펙 `rejections` 에 있다 |
| 문서 없는 클라이언트 (G2) | 시험 클라이언트가 `describe` 만 읽고, 바쁜 커널에 프롬프트를 내고, `next` 를 따라 기다렸다가 다시 내고, `operation` 으로 끝을 안다. 명령 이름과 이유 코드를 시험에 적지 않는다 |
| 입구의 경계 (G3) | `adapter/inbound/{cli,admin_api,telegram_bot}` 이 변경 서비스를 직접 부르지 않는다. 남은 호출은 허용 목록에 두고, 조각마다 줄인다 |
| CLI | 가짜 Gateway 로 `akra kernel` 의 JSON 과 종료 코드를 본다. 모델 턴은 쓰지 않는다 |

## 4. 단계 (PR 단위)

| 단계 | 내용 | 기존 동작 | 검증 |
| --- | --- | --- | --- |
| K4-1 | 스펙 뼈대. `domain::kernel_spec` 의 표, 프로토콜 타입의 스키마, `describe` 가 `KernelSpec` 을 답한다, `schema/akra-kernel-spec.json`, 프로토콜 판 1. 손으로 적은 이름 목록을 지운다. 병렬 상태 값을 타입으로 바꾼다 | 유지 (명령·이벤트의 전송 모양은 같다. `describe` 의 답만 바뀐다) | 스펙 파일 시험, 빠짐 없음 시험, 지금의 전송 모양 시험 |
| K4-2 | 이유 코드를 하나로 합치고 `next` 를 답에 싣는다. `task_finished` 를 더한다 | 유지 (글자는 같다. `next` 는 더한 필드이다) | domain 시험, 프로토콜 시험, 시뮬레이션의 거절 완전성 불변식 |
| K4-3 | Desktop 타입을 스펙에서 만든다. `protocol.ts` 의 손 복사를 지운다. Desktop 이 멱등 키를 보낸다 | 유지 | `npm run check`, vitest, e2e |
| K4-4 | `akra kernel` CLI. Gateway 에 붙고, 없으면 한 번 쓰는 커널을 쓴다 | 더함 | 가짜 Gateway 시험, 종료 코드 시험 |
| K4-5 | skill 문서를 스펙에서 만들고 등록한다. 문서 없는 클라이언트 시험 | 더함 | G2 시험, skill 파일 시험 |
| K4-6 | 연산의 끝. `completes_on`, `op_id` 를 싣는 기록, `operation` 조회 | 유지 (기록에 필드를 더한다) | core 시험, 시뮬레이션: 모든 연산이 끝 상태에 닿는다 |
| K4-7 | 호출자 기록, 효과 등급과 전제 조건을 커널이 검사한다 | 유지 (`reset_planning` 의 확인 규칙과 같다) | core 시험, 프로토콜 시험 |
| K4-8a | 입구 이전: planning 초기화 | 유지 (같은 결과. 기록에 남는다) | 입구 경계 시험, CLI·Telegram·admin 시험 |
| K4-8b | 입구 이전: 병렬 제어와 수동 배분. admin, Telegram, `parallel-tick` 이 스케줄러를 시작하지 않는다 | 전환 (Gateway 의 커널이 병렬 상태의 주인이다) | K-D3 회귀 시험: admin 의 정지가 커널에 닿는다 |
| K4-8c | 입구 이전: PR 검증 명령 | 유지 | admin 시험 |
| K4-8d | 입구 이전: 작업·방향·초안·파일. `planning-tool` 도 커널 명령을 쓴다 | 전환 (`planning-tool` 에 revision 이 필수가 된다) | planning worker skill 시험, admin 시험 |
| K4-8e | 입구 이전: 읽기. 직접 경로의 허용 목록을 비운다 (TUI 예외만 남는다) | 유지 | 입구 경계 시험 |

- 순서의 이유: K4-1 과 K4-2 가 나머지의 전제이다. K4-3~K4-5 는 서로 독립이다. K4-6 과 K4-7 은 core 를 바꾸므로
  시뮬레이션을 함께 넓힌다. K4-8 은 일의 묶음마다 PR 하나이다.
- 규모: K4-1~K4-5 는 각각 1~3일, K4-6 과 K4-7 은 각각 2~4일, K4-8 은 묶음마다 2~4일로 추정한다. 조사의 추정이다.

## 5. 위험

| 위험 | 대응 |
| --- | --- |
| 스펙이 타입을 다시 적은 또 하나의 복사본이 된다 | 표는 `match` 로 만들고 모양은 타입에서 만든다. 손으로 적는 것은 뜻(설명, 효과, 다음 행동)뿐이다 |
| `schemars` 가 만든 스키마가 serde 의 실제 전송 모양과 다르다 (tag, `skip_serializing_if`, `deny_unknown_fields`) | 지금의 전송 모양 시험을 남긴다. 시험이 표본 값을 스키마로 검사한다 |
| 내보낸 파일이 낡는다 | 시험이 견준다. 고치는 명령은 하나이다 |
| 입구를 옮기면서 Gateway 없이 쓰던 사용자가 막힌다 | CLI 는 한 번 쓰는 커널로 동작한다 (§3.7) |
| TUI 와 admin 을 함께 쓰는 사용자 | K6 전까지 직접 경로를 남긴다 (§3.7, Q-K4-3) |
| 한 번 쓰는 커널이 잠금을 오래 쥔다 | 명령 하나에 시간 상한을 둔다. 끝나면 기록을 다 쓰고 바로 내린다 |
| 이유 코드를 합치다가 전송 글자가 바뀐다 | 글자를 고정하는 시험을 먼저 쓴다. 기록의 이전 값도 읽는지 본다 |
| `op_id` 를 core 에 싣는 변경이 넓다 | 이미 있는 correlation 에만 싣는다. 시뮬레이션이 지킨다 |

## 6. 미결

| ID | 질문 | 기본값 (답이 없을 때) |
| --- | --- | --- |
| Q-K4-1 | 스키마를 `schemars` 로 만들 것인가 (새 의존성 하나) | 쓴다. plan 18 §4.4 가 정한 방법이다. 쓰지 않으면 타입마다 스키마 함수를 손으로 쓴다 |
| Q-K4-2 | 에이전트용 CLI 의 이름 | `akra kernel <묶음> <명령>`. 지금의 사람용 명령과 부딪히지 않는다 |
| Q-K4-3 | TUI 가 커널을 쥔 저장소에서 다른 입구의 변경 명령 | K6 전까지 직접 경로를 남긴다. 다른 선택: (a) 이유 코드로 거절한다, (b) "TUI 가 Gateway 의 커널에 붙는다"(K6)를 앞당긴다 |
| Q-K4-4 | Gateway 가 없을 때 admin 과 Telegram | Gateway 를 요구하고, 없으면 이유를 보이고 끝난다. 다른 선택: 자기 프로세스에서 커널을 오래 돌린다 (지금의 K-D5 와 같은 모양이 된다) |
| Q-K4-5 | `planning-tool` 을 `akra kernel` 로 합칠 것인가 | 이름은 남기고 안에서 커널 명령을 쓴다. planning worker 의 skill 을 함께 고친다 |
| Q-K4-6 | MCP 도구와 SDK 를 K4 에 넣을 것인가 | 넣지 않는다. 스펙 JSON 이 있으면 나중에 만들 수 있다 |
