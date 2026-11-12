# K4 상세 설계: 명령 스펙 하나와 입구 통일

## 문서 상태

- 상태: **구현 중**. K4-1, K4-2, K4-3 을 구현했다 (2026-11-12, §8). 나머지 조각은 제안이다. 현재 제품 동작의 기준은
  `docs/reference/` 이다. 2026-11-12 에 리뷰 세 개(독립 리뷰, 표준
  축, 스펙 축)를 받아 고쳤다 (§7). 같은 날 사용자가 §6 의 질문을 작업자의 판단에 맡겼고, 기본값으로 정했다.
- 상위 계획: [18 Akra 커널 아키텍처](18-akra-kernel-architecture.md) §4.3, §4.4, §5.5, §5.6, §6 K4. 결정 KP-04,
  KP-05 (승인 2026-11-07). [19 Akra 커널 성질 목표](19-akra-kernel-quality-goals.md) §5.4, Q4, 성질 P3.
- 이전 단계: [K3](18-k3-kernel-daemon-protocol.md) (프로토콜 v0, 완료 2026-11-10), plan 19 Q3 (사건 기록, 승인 기록,
  연산 영수증, 멱등 키, 완료 2026-11-11). plan 19 Q-Q1 에 따라 사건 기록(K5 의 기반)을 스펙보다 먼저 했다.
- 기준일: 2026-11-12 (Asia/Seoul)
- 기준 커밋: `aa85888d` (`origin/prerelease`).
- 범위: 커널의 명령, 조회, 이벤트, 기록, 이유 코드, 오류 코드를 한 번 정의한다. 그 정의에서 자기 설명, Desktop 타입,
  에이전트용 CLI, skill 문서를 만든다. CLI, admin, Telegram 의 변경 명령을 커널 경로로 옮긴다. 얇은 TUI(K6), thread
  액터(Q5), 관찰자와 기능 카탈로그(plan 20), 묻기·답하기와 승인 정책은 범위 밖이다.

## 1. 목표

| # | 목표 | 확인 방법 |
| --- | --- | --- |
| G1 | 사실 하나는 한 곳에만 적는다. 명령, 조회, 이벤트, 기록 종류, 이유 코드, 오류 코드의 이름과 모양과 뜻이다 | 손으로 적은 이름 목록과 손으로 복사한 타입이 없다 (§2.1 의 표가 빈다) |
| G2 | 커널이 스스로를 설명한다 (P3). 무엇을 할 수 있는가, 입력은 무엇인가, 왜 거절했는가, 다음에 무엇을 하는가 | 문서를 읽지 않은 프로그램이 `describe` 와 기록 조회만으로 프롬프트를 내고, 거절에 대응하고, 끝을 안다 (§3.9 의 시험) |
| G3 | 모든 입구가 같은 명령을 쓴다 (KP-04). CLI, admin, Telegram, 에이전트의 변경 명령이 커널을 거친다 | 입구 adapter 가 변경 서비스를 직접 부르지 않는다 (구조 시험). Gateway 의 커널을 쓰는 저장소에서 K-D3, K-D4, K-D5 가 닫힌다. TUI 가 커널을 쥔 저장소는 K6 에서 닫힌다 (§3.7) |
| G4 | 에이전트의 기본 입구는 `akra kernel ... --json` 과 생성된 skill 이다 (KP-05) | 에이전트가 셸만으로 커널에 명령하고 안정된 JSON 을 받는다 |

## 2. 현재 상태 (2026-11-12 코드 조사)

### 2.1 같은 사실이 여러 곳에 있다

| 사실 | 적힌 곳 | 문제 |
| --- | --- | --- |
| 명령 이름 | `KernelCommand` enum, `COMMANDS` 목록, 시험의 표본 목록 (`kernel_protocol/mod.rs`, `tests.rs`) | 시험은 개수만 본다. 새 variant 를 목록에 빠뜨려도 통과한다 |
| 조회와 이벤트 | 프로토콜 `KernelEvent` enum 하나가 이벤트와 조회의 답을 함께 담는다. `QUERIES`, `EVENTS` 목록이 따로 있다 | 조회의 답 타입을 타입에서 얻지 못한다. 시험이 목록의 길이를 보지 않는다 |
| 기록 종류 | `domain::kernel_event::KernelEvent`, `kind()`, `KERNEL_EVENT_KINDS` | 표본 시험이 지킨다. 컴파일러가 강제하지 않는다 |
| 프롬프트를 준비 전에 거절한 이유 5개 | `CommandRejectionReason`, `ThreadPromptRejection`, `PromptSubmissionReason` | enum 3개. `ThreadPromptAdmission` 에서 세 번 따로 바꾼다. 설명 문장도 두 곳에 있다 |
| "다음에 무엇을 하는가" | 이유 enum 의 주석 | 주석이다. 클라이언트에 가지 않는다 |
| 프로토콜 타입 | Rust 타입과 `desktop/src/renderer/kernel/protocol.ts` | 손으로 복사했다. `operation`, `history`, `recorded`, `describe` 가 빠졌고 이유 코드가 `string` 이다 |
| 명령의 성질 (읽기인가, 기록하는가, thread 를 가리키는가, 확인이 필요한가) | `descriptor()` 의 match, `into_core` 의 검사, 주석 | 한 표가 없다 |
| 병렬 상태 값 | `ParallelAgent.state`, `ParallelMergeItem.state`, `ParallelPrValidation.state`, `ParallelPrValidation.phase` | 자유 글자이다 (화면용 이름). K3 이 "K4 에서 고친다"고 남겼다 |

- Akra 커널의 스키마와 생성기는 없다 (`schemars` 없음, `build.rs` 없음). `schema/` 와
  `desktop/scripts/generate-protocol.mjs` 는 Codex app-server 의 프로토콜만 다룬다.
- Akra 는 MCP 서버가 아니다. MCP 는 hub 로만 쓴다.

### 2.2 입구가 커널을 거치지 않는다

`akra` CLI, admin HTTP, Telegram bot 은 커널(`KernelHost`, `AppCommand`, `akra/kernel/*`)을 쓰지 않는다. 각자
`composition/production.rs` 로 application 서비스를 조립해 직접 부른다. `akra mcp` 만 Gateway 에 붙는다.

| 일 | 구현된 입구 | 차이 |
| --- | --- | --- |
| planning 초기화 | CLI `reset`, admin `/api/planning/reset`, Telegram `/reset`, 커널 `reset_planning`, TUI | 대상 읽기와 확인 규칙을 네 번 썼다. 끝은 같은 서비스이다 |
| 작업 만들기·고치기 | admin (고치기에 revision 검사), `akra planning-tool run` (검사 없음. 커널이 띄운 숨은 planning worker 가 쓴다), TUI | revision 규칙이 입구마다 다르다 |
| 병렬 모드 켜기·끄기·배분·새로 고침 | admin `/api/admin/akra/control`, TUI | 커널 프로토콜에 없다. admin 은 자기 프로세스의 병렬 제어부를 움직인다 (K-D3) |
| 수동 배분 한 번 | CLI `parallel-tick` | CLI 에만 있다 |
| PR 검증 명령 5개 | admin `/api/admin/akra/validations/{k}/commands` (revision 검사, 명령 id 로 결과 조회) | admin 에만 있다 |
| 초안, 방향, 파일 내보내기·적용, 병렬 에이전트 프로필 저장 | admin (HTML 과 API), TUI | - |
| 상태·큐 읽기 | CLI `status`, `queue`, Telegram, admin | 읽기 명령도 서비스를 조립하면서 프롬프트 기록 정리(지우기)를 실행한다 |

- 이 입구들은 저장소 잠금(`kernel.lock`)을 잡지 않는다. 그런데 admin, Telegram, `parallel-tick` 은 병렬 제어부를
  조립하면서 PR 검증 스케줄러 스레드를 시작한다 (K-D5 의 남은 부분).
- 기계용 출력은 `planning-tool` 의 JSON 줄뿐이다. 다른 CLI 명령에 `--json` 이 없고, Telegram 은 한국어 글, admin 은
  경로마다 다른 JSON 이다. 실패는 종료 코드 1 과 글이다.
- 이 입구의 명령은 판단 사건 기록에 남지 않는다 (K-D4).

### 2.3 커널을 쓰는 쪽의 제약

| 제약 | 근거 | 이 설계에 주는 영향 |
| --- | --- | --- |
| 커널을 조립하면 일을 이어 간다. 남긴 프롬프트를 되돌려 쉴 때 다시 내고(모델 턴), 열린 승인을 `lost` 로 닫고, PR 검증 스케줄러와 병렬 tick 을 시작한다 | `attach_kernel_event_log`, `build_kernel_for_workspace` | 명령 하나를 위해 커널 전체를 조립하면 안 된다 (§3.7) |
| Gateway 의 커널은 클라이언트가 떠난 뒤에도 10분 동안 잠금을 쥔다. 그동안 같은 저장소의 TUI 는 열리지 않는다 | K3 §3.5, §3.7 (커널 내리기 정책) | 입구가 붙을 때마다 TUI 가 10분 막힌다 (§3.7) |
| TUI 는 자기 프로세스에서 커널을 돌리고 소켓을 열지 않는다. Gateway 가 그 저장소에 붙으면 `-32012` 이다 | K3 §3.5 | TUI 가 쥔 저장소의 커널에는 다른 입구가 닿지 못한다 (§3.7) |
| 잠금 파일에는 pid, 시작 시각, 폴더만 있다 | `kernel_lock.rs` | 누가 쥐었는지(TUI 인가, Gateway 인가) 모른다 |
| 턴의 시작과 끝 기록은 thread 요약의 차이로 만든다. 명령과 이어 주는 값이 없다 | plan 19 Q3-2 | 연산의 끝을 계산하려면 기록에 연산 id 가 있어야 한다 (§3.4) |
| domain, core, `kernel_host` 는 inbound adapter 를 쓰지 못한다. outbound adapter 도 inbound 의 전송 타입을 쓰지 못한다 | `tests/architecture_boundaries.rs` | 표의 자리와 Gateway 클라이언트의 모양을 정한다 (§3.1, §3.6) |

### 2.4 이전 단계가 K4 로 넘긴 일

| 넘긴 일 | 출처 | 이 설계의 자리 |
| --- | --- | --- |
| 프로토콜을 스펙으로 만들면서 v1 로 올린다 | K3 §5 | K4-1 |
| 전송 타입의 이름 규칙 | K3 §3.7 | K4-1 (§3.2) |
| 병렬 상태 값을 타입으로 | K3 (`kernel_protocol` 주석) | K4-2 |
| 이유 코드와 다음 행동 | plan 19 §5.4 | K4-3 |
| 완료로 보여 버린 프롬프트를 알리지 않는다 | plan 19 Q2-3 | K4-3 (§3.3) |
| 연산이 끝났는지 | plan 19 Q3-6 | K4-4 |
| 누가 명령했는지, 누가 승인을 결정했는지 | plan 19 Q3-5 | K4-5 |
| 보지 않는 thread 에 낸 턴 제출을 커널이 막지 않는다 | plan 19 Q3-7 | K4-5 (명령의 전제 조건) |
| admin 의 병렬 정지가 커널에 닿지 않는다 (K-D3), 스케줄러가 여러 프로세스에서 돈다 (K-D5), 감사 기록 (K-D4) | plan 18 §5.6 | K4-9 |

## 3. 설계

### 3.1 스펙의 자리: Rust 가 정본이고, 내보낸 JSON 이 데이터이다

두 방법을 비교했다.

| 방법 | 장점 | 단점 |
| --- | --- | --- |
| A. 데이터 파일(YAML 등)을 정본으로 두고 Rust 타입을 생성한다 (plan 19 §5.4 의 처음 안) | 언어에 묶이지 않는다 | 빌드에 Rust 생성 단계가 생긴다. `match` 의 빠짐을 컴파일러가 잡지 못한다. 한 번에 모든 타입을 바꿔야 한다 |
| **B. Rust 타입과 Rust 표를 정본으로 두고, 스펙 JSON 을 내보낸다 (채택. plan 18 §4.4 의 방법)** | 지금 타입을 그대로 쓴다. 컴파일러가 빠짐을 잡는다. 조각마다 옮길 수 있다 | Rust 밖의 대상은 내보낸 JSON 에서 만든다. JSON 이 낡지 않게 지키는 시험이 필요하다 |

표는 그 enum 이 있는 층에 둔다. domain 은 adapter 를 쓰지 못하기 때문이다.

| 표 | 자리 | 이유 |
| --- | --- | --- |
| 명령 (`KernelCommandKind` 와 그 성질) | domain `kernel_spec` | 커널이 명령의 효과를 검사하고 기록한다. `KernelCommandDescriptor.command` 가 글자에서 이 종류로 바뀐다 |
| 이유 코드, 기록 종류 | domain (이미 domain 의 enum 이다) | core 가 낸다 |
| 조회, 이벤트, 오류 코드, 모든 전송 모양 | inbound adapter `kernel_protocol` | 전송 타입이 여기에 있다. 커널은 이 표를 쓰지 않는다 |

```text
domain::kernel_spec        명령의 종류와 성질, 이유 코드와 다음 행동, 기록 종류
kernel_protocol (inbound)  전송 모양 (serde 타입), 조회·이벤트·오류의 표, 타입에서 만든 JSON Schema
        │  spec() = domain 의 표 + adapter 의 표 + 스키마
        ▼
akra/kernel/describe  ──▶  schema/akra-kernel-spec.json (저장소에 둔다. 시험이 spec() 과 같은지 본다)
                                   │
        ┌──────────────┬───────────┼──────────────┬──────────────────────┐
        ▼              ▼           ▼              ▼                      ▼
   Desktop TS 타입   akra kernel   skill 문서    MCP 도구 (K4-10)   SDK (K4-10)
```

- 표의 항목은 빠짐없는 `match` 로 만든다. variant 를 더하면 표를 채울 때까지 컴파일되지 않는다. 손으로 적은
  `COMMANDS`, `QUERIES`, `EVENTS`, `KERNEL_EVENT_KINDS` 는 지운다.
- 명령의 이름은 두 타입에 걸친다: 전송 enum `KernelCommand` (필드가 있다)와 domain 의 `KernelCommandKind` (필드가
  없다). adapter 가 `KernelCommand → KernelCommandKind` 를 빠짐없는 `match` 로 바꾼다. 전송 글자(`submit_prompt`)는
  `KernelCommandKind` 에서만 나오고, 시험이 전송 enum 의 tag 와 같은지 본다. 이름 enum 을 하나 더 두는 까닭은 커널이
  명령의 종류를 알아야 하기 때문이다.
- `schema/akra-kernel-spec.json` 이 plan 19 §5.4 의 "데이터로 한 번만"이다. Rust 가 이 파일을 만들므로 Rust 와 어긋날
  수 없다. 시험이 파일과 `spec()` 을 견준다. 파일을 고치는 명령은 `scripts/refresh_akra_kernel_spec.sh` 하나이다.
- 스키마는 `schemars` 로 만든다 (plan 18 §4.4). K4-1 의 첫 일은 확인이다: 이 프로토콜의 모양(내부 tag enum 의 newtype
  variant, `deny_unknown_fields`, `skip_serializing_if`, 빈 variant `{}`)에서 스키마가 실제 전송 모양과 같은지 본다.
  다르면 그 타입의 스키마만 손으로 쓴다. 미결 Q-K4-1.

### 3.2 스펙의 내용

```text
KernelSpec { protocol_version, commands[], queries[], events[], records[], reasons[], errors[] }

Command {
  name         "submit_prompt"
  scope        이 명령이 바꾸는 상태의 구역: session, thread, approval, auto_follow, planning, parallel, pr_validation.
               권한 범위이다 (plan 18 §4.4). CLI 는 이 값을 묶음 이름으로 쓴다
  summary      한 문장. 에이전트가 읽는다
  input        JSON Schema. 빈 값과 길이의 규칙도 스키마에 적는다
  output       JSON Schema. 모든 명령의 답은 같은 영수증이다
  effect       read | write | destructive. 부작용이다
  state        store | runtime. 저장된 planning 데이터만 바꾸는가, 도는 커널의 상태를 바꾸는가 (§3.7)
  idempotency  natural (같은 입력은 같은 상태가 된다) | keyed (멱등 키로 재전송을 가린다)
  rejections   이 명령이 낼 수 있는 이유 코드
  completion   이 명령의 끝 (§3.4)
}
Query  { name, summary, input (JSON Schema), output (JSON Schema) }
Event  { name, summary, payload (JSON Schema) }
Record { name, summary, payload (JSON Schema) }
Reason { name, summary, next[] }
Error  { name, code, summary, next[] }
```

- 모든 항목의 식별자는 `name` 이고 설명은 `summary` 이다. `name` 의 값은 전송되는 글자와 같다 (이벤트의 `type`, 기록의
  `kind`, 이유 코드).
- 표에서 나오는 규칙은 필드로 두지 않는다. `destructive` 명령은 입력의 `confirm: true` 가 필요하다. `read` 명령은
  기록에 남지 않는다.
- 판은 `protocol_version` 하나이다. 전송 모양을 호환되지 않게 바꿀 때만 올린다. 필드를 더하는 것은 올리지 않는다.
  K4-1 에서 `describe` 의 답이 바뀌므로 1 이 된다.
- 기록의 payload 스키마는 클라이언트가 쓰는 필드만 적는다. `thread_prompt_queued` 의 `handoff` 처럼 커널이 되돌릴 때만
  쓰는 값은 스펙에서 감춘다.
- 전송 타입의 이름 규칙 (K3 이 넘긴 일): 명령은 `<동사>_<대상>`, 조회와 이벤트는 대상의 이름이다. 스키마의 타입
  이름은 `<대상>State`(지금 값), `<명령>Request`·`<명령>Response`(메서드의 입력과 답), `<대상>Record`(기록)이다.

`next` 는 글이 아니라 데이터이다. 순서가 있는 목록이고, 클라이언트가 그대로 실행할 수 있는 것만 둔다.

| `next` 의 단계 | 뜻 |
| --- | --- |
| `wait_for { event }` | 그 이름의 이벤트가 다음에 올 때까지 기다린다. 조건은 없다. 온 뒤에 다음 단계를 한다 |
| `wait { milliseconds }` | 그만큼 기다린다. 그 일의 끝을 알리는 이벤트가 없을 때만 쓴다 (K4-3 에서 더했다, §8) |
| `query { name }` | 그 조회로 지금 값을 읽는다 |
| `command { name }` | 그 명령을 보낸다 |
| `method { name }` | 그 메서드를 보낸다 (`not_attached` 뒤의 `akra/kernel/attach`. K4-3 에서 더했다, §8) |
| `resend` | 같은 요청을 다시 보낸다. 명령이면 새 멱등 키로 보낸다 |
| `ask_user` | 사람이 정해야 한다 |
| `stop` | 다시 하지 않는다 |

예: `busy` → `[wait_for { thread_activity }, resend]`. 다시 `busy` 이면 같은 목록을 다시 따른다. `unknown_thread` →
`[command { load_sessions }, resend]`. `no_matching_turn` → `[query { thread_activity }]`. `text_mismatch` → `[stop]`.

### 3.3 이유 코드는 하나의 목록이다

- `CommandRejectionReason`, `ThreadPromptRejection`, `PromptSubmissionReason` 을 domain 의 `ReasonCode` 하나로 합친다.
  전송되는 글자(`busy` 등)는 바꾸지 않는다. 그래서 기록과 v0 클라이언트가 그대로 읽는다.
- core 는 `ThreadPromptAdmission` 을 이유 코드로 한 번만 바꾼다. 설명 문장은 표에서 온다.
- 답에 `next` 를 싣는다: 명령 답의 `rejection.next`, `prompt_submission` 이벤트의 `next`. 값은 스펙의 그 이유와 같다.
- 입력이 틀려서 나는 JSON-RPC 오류(`-32602` 등)도 `errors[]` 의 `next` 를 가진다.
- 큐에 남긴 프롬프트를 작업이 완료되어 버릴 때, 지금은 기록(`thread_prompt_settled` 의 `task_finished`)만 남고
  클라이언트는 모른다. `prompt_submission` 이벤트로 알린다. 새 이유 코드를 만들지 않고 이미 있는 그 값을 쓴다.

### 3.4 연산의 끝

Q3-6 의 영수증은 받아들임과 바로 거절까지만 알린다. 끝은 명령마다 다르므로 스펙의 `completion` 이 정한다.

| 명령 | 끝 |
| --- | --- |
| `set_auto_follow_max_turns`, `pause_auto_follow` | 받아들이면 끝이다 |
| `rename_session`, `reset_planning`, `decide_approval`, `steer_turn`, `stop_all` | 그 요청의 완료. `decide_approval` 은 이미 있는 `approval_resolved` 이다. 나머지는 완료 기록을 더한다 |
| `submit_prompt` | 준비 전 거절, 준비 실패, 남긴 프롬프트가 턴 없이 끝남(`thread_prompt_settled` 의 `task_finished`, `parallel_slots`), 또는 그 턴의 `turn_ended`. 큐에 남긴 동안은 끝이 아니다 |

- 조회 `operation { op_id }` 를 더한다. 답은 `state` 와 그 연산의 기록이다. 따로 저장하지 않는다. `op_id` 뒤의 기록에서
  계산한다 (plan 19 §5.1.2 의 원칙 4).

  | `state` | 뜻 |
  | --- | --- |
  | `accepted` | 받아들였고 아직 기록이 없다 |
  | `running` | 진행 중이다 (준비 중, 큐에 남음, 턴이 돈다) |
  | `done` | 끝났다 |
  | `rejected` | 커널이 거절했다 |
  | `failed` | 준비나 턴이 실패했다 |
  | `lost` | 연산이 열린 채로 커널이 다시 조립되었다 |
  | `unknown` | 기록이 보존 한도(10,000개)를 넘어 지워졌다 |

- 명령이 만든 기록은 `op_id` 를 싣는다. 필요한 변경:
  - core 의 correlation(프롬프트 준비, 턴 제출, steer, 승인 결정, 중지)에 `op_id` 를 싣는다.
  - 턴의 시작과 끝 기록은 thread 요약의 차이로 만든다. 그래서 thread session 이 "지금 턴의 연산"을 가진다.
  - 남긴 프롬프트(`QueuedThreadPrompt`)에 `op_id` 를 더한다. 재시작 뒤 다시 낸 턴도 같은 연산이다.
  - 재시작 때 열린 턴을 `turn_ended` 의 `lost` 로 닫는다. 승인의 `lost` (Q3-5)와 같은 규칙이다. 지금은 열린 턴을 닫는
    기록이 없어서 그 연산이 끝나지 않는다.
- 자동 후속 턴은 명령이 아니다. `op_id` 가 없고 `origin` 이 `auto_follow` 이다.

### 3.5 호출자와 효과 등급

- 호출자는 접속이다. `attach` 의 `client_name` 을 커널이 `command_received.caller` 에 적는다. 값은 클라이언트가 스스로
  밝힌 이름이다 (`akra-desktop`, `akra-cli`, `akra-admin`, `akra-telegram`). 로컬 소유자만 접속하므로 인증은 아니다.
  승인 결정도 명령이므로 누가 결정했는지 남는다. 커널 안의 결정(자동 후속)은 명령이 아니고, 기록의 `origin` 이 알린다.
- 커널은 스펙의 `effect` 를 검사한다. 입구는 검사하지 않는다. `destructive` 명령은 `confirm: true` 가 없으면 거절한다
  (지금의 `reset_planning` 규칙을 일반화한다, K-D1). 검사는 `KernelCommandDescriptor` 의 종류와 `confirmed` 로 한다.
- 명령의 전제 조건(예: 보지 않는 thread 에 낸 턴 제출)은 커널이 검사하고 이유 코드로 거절한다.
- 호출자마다 다른 권한(예: 에이전트는 `destructive` 를 못 한다)은 이 설계에 넣지 않는다. 에이전트 자율성의 한계
  (plan 18 Q-K5)와 함께 정한다. 스펙에 `scope` 와 `effect` 가 있으므로 그 정책은 표 하나로 쓸 수 있다.

### 3.6 생성하는 것

| 대상 | 방법 | 비고 |
| --- | --- | --- |
| 자기 설명 | `akra/kernel/describe` 가 `KernelSpec` 을 답한다. 프로토콜 판을 1 로 올린다 | Desktop 은 지금 `describe` 를 부르지 않는다. 그래서 답의 모양을 바꿔도 깨지는 클라이언트가 없다 |
| Desktop 타입 | 생성기 `desktop/scripts/generate-kernel-protocol.mjs` (`npm run generate:kernel-protocol`)가 `schema/akra-kernel-spec.json` 에서 `desktop/src/shared/kernel-protocol/generated/` 를 만든다. `protocol.ts` 의 손 복사를 지운다 | 생성 파일은 손으로 고치지 않는다 (`desktop/AGENTS.md` 의 규칙과 같다). vitest 시험이 생성 결과와 스펙을 견준다. `scripts/check_node_surfaces.sh` 가 그 시험을 돌린다. 생성기는 이 스펙이 쓰는 스키마만 읽는 작은 스크립트로 쓴다 (미결 Q-K4-7) |
| 에이전트용 CLI | `akra kernel <scope> <명령> [--필드 값] --json`, `akra kernel query <이름>`, `akra kernel history --after <n> [--wait]`, `akra kernel operation <op_id> [--wait]`, `akra describe --json` | 코드를 생성하지 않는다. binary 에 넣은 스펙 JSON 으로 인자를 읽고 입력을 검사한다 |
| skill 문서 | 스펙에서 `assets/app-server/hub-skills/akra-kernel/SKILL.md` 를 만든다. 저장소에 두고 시험이 스펙과 견준다 | `akra-mcp-hub` skill 과 같은 방법으로 등록한다 (`skills/extraRoots/set`) |
| MCP 도구, SDK (TypeScript, Python) | 스펙 JSON 에서 만든다 | K4-10. 쓰는 쪽이 생길 때 한다. MCP 는 `destructive` 명령과 셸이 없는 클라이언트용이다 (KP-05) |

입구가 커널에 닿는 길은 하나이다 (입구마다 분기를 쓰지 않는다).

| 부품 | 층 | 하는 일 |
| --- | --- | --- |
| `KernelCommandPort` | application inbound port | 입구(CLI, admin, Telegram)가 부른다. 명령의 종류, JSON 입력, 멱등 키를 받고 영수증을 준다 |
| `KernelCommandService` | application service | 어디로 보낼지 정한다 (§3.7 의 세 갈래). 이 분기는 여기에만 있다 |
| `KernelGatewayPort` 와 `GatewayKernelClientAdapter` | outbound port 와 adapter | Gateway 접속으로 `akra/kernel/*` 를 보낸다. 전송 타입을 쓰지 않고 JSON 값을 보낸다 (`akra mcp` 의 Gateway 클라이언트와 같은 방법) |

에이전트용 CLI 의 계약:

- 출력은 한 줄 JSON 이다. 사람용 글과 섞지 않는다.
- 종료 코드: 0 = 받아들임, 2 = 커널이 거절함 (JSON 에 이유와 `next`), 1 = 닿지 못함 또는 잘못된 입력.
- 변경 명령은 `--idempotency-key` 를 받는다. 호출자가 주지 않으면 CLI 가 무작위 키를 만든다. 그 키는 이 실행 안의
  다시 보내기(접속이 끊겨 다시 붙을 때)에만 쓴다. 실행을 넘어 다시 보내려는 호출자는 키를 직접 준다. skill 문서가
  이것을 적는다.
- 이름은 `akra kernel` 이다. plan 18 §4.4 의 `akra <그룹> <동작>` 은 지금의 사람용 명령(`status`, `queue`, `reset`,
  `config`, `mcp`)과 부딪힌다. `akra describe --json` 은 plan 18 대로 둔다.
- 지금의 사람용 명령(`akra status`, `queue`, `reset`, `doctor`)은 남긴다. 변경 명령은 안에서 `KernelCommandPort` 를 쓴다.

### 3.7 입구의 변경 명령을 커널 경로로 옮긴다

일의 묶음마다 커널 명령을 스펙에 더하고, 입구가 `KernelCommandPort` 로 보내게 바꾼다. 순서는 결함이 큰 것부터이다.

| 순서 | 일 | 더하는 커널 명령 | `state` | 닫는 결함 |
| --- | --- | --- | --- | --- |
| 1 | planning 초기화 | 있는 `reset_planning` 을 쓴다 | store | 네 번 쓴 대상·확인 규칙, K-D4 |
| 2 | 병렬 모드와 수동 배분, 병렬 에이전트 프로필 | `set_parallel_mode`, `dispatch_parallel`, `refresh_parallel`, `save_parallel_agent_profiles` | runtime (프로필은 store) | K-D3 (정지가 커널에 닿는다), K-D5 (입구가 스케줄러를 시작하지 않는다) |
| 3 | PR 검증 명령 | `retry_pr_validation`, `pause_pr_validation`, `resume_pr_validation`, `queue_pr_remediation`, `acknowledge_pr_validation`. 입력은 `key`, `expected_revision` | runtime | K-D4. admin 의 명령 id 조회는 `operation` 조회로 바꾼다 |
| 4 | 작업·방향 만들기·고치기·지우기, 초안 만들기·저장·승격, 파일 내보내기·적용 | `upsert_task`, `delete_task`, `upsert_direction`, `delete_direction`, `create_draft`, `save_draft`, `promote_draft`, `export_planning_files`, `apply_planning_files` | store | `planning-tool` 의 revision 검사 없음 (K-D2 의 남은 입구), K-D4 |

- revision: 고치기와 지우기는 `expected_planning_revision` 이 필수이다. 지금은 admin 의 작업 고치기만 검사한다. 만들기는
  받지 않는다.
- 읽기는 옮기지 않는다. G3 은 변경 명령의 목표이다. admin 의 화면과 조회(리뷰, 프롬프트 기록, 지표, pool, 초안, SSE),
  CLI 의 `status`·`queue`·`doctor`, Telegram 의 조회는 지금 경로에 둔다. 서비스를 조립할 때 실행하는 프롬프트 기록
  정리만 읽기 경로에서 뺀다. 정리는 커널이 한다.
- 옮기지 않는 변경과 그 이유:
  - `akra config set|unset`: 사용자와 프로젝트의 설정 파일이다. 저장소 커널의 상태가 아니다.
  - admin 의 debug harness (`--debug-harness` 일 때만): 시험용이고 저장하지 않는다.
  - admin 의 로그인과 로그아웃: admin 서버의 세션이다.

`KernelCommandService` 가 명령을 보내는 곳은 세 갈래이다.

| 상황 | 보내는 곳 | 비고 |
| --- | --- | --- |
| Gateway 가 떠 있고 그 저장소의 커널을 쓸 수 있다 | Gateway 의 커널. 잠깐 붙는 접속(`transient`)으로 붙는다 | 마지막 접속이 잠깐 붙는 접속이었고 남은 일이 없으면 커널을 바로 내린다. 10분 대기는 Desktop 처럼 오래 붙는 클라이언트에만 쓴다. 그렇지 않으면 `akra` 명령 한 번이 TUI 를 10분 막는다 |
| Gateway 가 없다 | `state: store` 명령은 명령 전용 조립으로 실행한다. `state: runtime` 명령은 이유 코드 `kernel_not_running` 으로 거절한다 (`next`: Gateway 를 시작한다) | 명령 전용 조립은 저장소 잠금을 잡고, 기록을 붙이고(되돌리기와 `lost` 닫기는 하지 않는다), 명령 하나를 처리하고, 기록을 다 쓰고, 잠금을 푼다. app-server, 스케줄러, 병렬 tick, 남긴 프롬프트의 되돌림은 시작하지 않는다. 그래서 모델 턴이 없다 |
| TUI 가 그 저장소의 커널을 쥐고 있다 | K6 전까지 지금의 직접 경로 (기본값, 미결 Q-K4-3) | 잠금 파일에 쥔 쪽의 종류(`tui`, `gateway`, `command`)를 적어서 가린다. 이 경로의 명령은 기록에 남지 않는다. K-D3 과 K-D5 는 TUI 를 쓰는 동안 남는다. K6 에서 지운다 |

- 커널이 띄운 planning worker 의 `planning-tool` 도 이 길을 탄다. TUI 의 worker 는 셋째 갈래를 타므로 K6 까지 revision
  검사가 없다.
- K4-9d 전에 확인할 것: sandbox 안에서 도는 worker 의 셸이 Gateway 소켓에 닿는가. 닿지 않으면 worker 는 명령 전용
  조립을 쓰지 못한다 (커널이 잠금을 쥐고 있다). 그 경우 `planning-tool` 은 직접 경로에 남기고 revision 검사만 더한다.

### 3.8 하지 않는 것

- TUI 를 커널 프로토콜의 클라이언트로 바꾸지 않는다 (K6).
- 동시 턴과 thread 액터를 만들지 않는다 (Q5).
- 기능 카탈로그(plan 20 §4: 기능과 에이전트가 "언제 나를 부를지"를 선언한다)를 만들지 않는다. plan 20 §8 의 "명령
  노출"이 이 설계이다: 카탈로그의 항목은 스펙의 명령 이름을 가리킨다. 카탈로그의 선언과 고르는 규칙은 plan 20 에서
  정한다.
- 묻기·답하기, 승인을 사람·에이전트·정책으로 보내는 일, 관찰자(plan 20)를 만들지 않는다.
- dry-run, 예산, 재귀 깊이 같은 안전장치(plan 18 §4.3 의 7)를 만들지 않는다.
- admin 의 HTML 화면을 다시 만들지 않는다. 화면은 그대로 두고 변경 요청만 커널 명령으로 보낸다.

### 3.9 경계 시험

모든 시험은 모델 턴을 쓰지 않는다 (가짜 app-server, 가짜 Gateway).

| 시험 | 내용 |
| --- | --- |
| 스펙 파일 | `schema/akra-kernel-spec.json` 이 `spec()` 과 같다. skill 문서와 Desktop 생성 타입도 스펙과 같다 |
| 빠짐 없음 | 명령·조회·이벤트·기록·이유의 표가 빠짐없는 `match` 이다. 모든 이유에 `next` 가 있다 (`stop` 도 적는다) |
| 스키마와 전송 모양 | 지금의 전송 모양 시험을 남긴다. 표본 값을 스키마로 검사한다 |
| 거절의 완전성 | 커널 시뮬레이션에 불변식을 더한다: 커널이 명령에 낸 모든 이유 코드는 그 명령의 스펙 `rejections` 에 있다 |
| 연산의 끝 | 커널 시뮬레이션에 불변식을 더한다: 시나리오가 끝나면 모든 연산의 `state` 가 `done`, `rejected`, `failed`, `lost` 중 하나이다 |
| 문서 없는 클라이언트 (G2) | 시험 클라이언트가 `describe` 만 읽고, 바쁜 커널에 프롬프트를 내고, `next` 를 따라 기다렸다가 다시 내고, `operation` 으로 끝을 안다. 명령 이름과 이유 코드를 시험에 적지 않는다 |
| 입구의 경계 (G3) | `adapter/inbound/{cli,admin_api,telegram_bot}` 이 변경 서비스를 직접 부르지 않는다. 남은 호출은 허용 목록에 두고, 조각마다 줄인다. 끝에는 §3.7 의 예외만 남는다 |
| CLI | 가짜 Gateway 로 `akra kernel` 의 JSON 과 종료 코드를 본다. 명령 전용 조립이 app-server 와 스케줄러를 시작하지 않는 것을 본다 |

## 4. 단계 (PR 단위)

| 단계 | 내용 | 기존 동작 | 검증 |
| --- | --- | --- | --- |
| K4-1 (완료 2026-11-12, §8) | 스펙 뼈대. `schemars` 확인, `KernelCommandKind` 와 domain 의 표, adapter 의 표와 스키마, 조회의 답 타입을 이벤트에서 나눈다, `describe` 가 `KernelSpec` 을 답한다, `schema/akra-kernel-spec.json`, 프로토콜 판 1, 이름 규칙. 손으로 적은 이름 목록을 지운다 | 유지 (명령·조회·이벤트의 전송 모양은 같다. `describe` 의 답만 바뀐다) | 스펙 파일 시험, 빠짐 없음 시험, 스키마와 전송 모양 시험 |
| K4-2 (완료 2026-11-12, §8) | 병렬 상태 값을 타입으로 바꾼다 | 유지 (전송되는 글자는 같다. 스펙이 값의 목록을 알린다) | 프로토콜 시험, domain 시험 |
| K4-3 (완료 2026-11-12, §8) | 이유 코드를 하나로 합치고 `next` 를 답에 싣는다. 버린 프롬프트를 `prompt_submission` 으로 알린다 | 유지 (글자는 같다. `next` 는 더한 필드이다) | domain 시험, 프로토콜 시험, 시뮬레이션의 거절 완전성 불변식 |
| K4-4 | 연산의 끝. 기록에 `op_id`, 완료 기록, 재시작 때 열린 턴의 `lost`, `operation` 조회 | 유지 (기록에 필드와 종류를 더한다) | core 시험, 시뮬레이션의 연산 끝 불변식 (재시작 seed 포함) |
| K4-5 | 호출자 기록. 효과 등급과 전제 조건을 커널이 검사한다 | 바뀜 (보지 않는 thread 의 턴 제출을 거절한다. `destructive` 의 확인 규칙은 `reset_planning` 과 같다) | core 시험, 프로토콜 시험, 시뮬레이션 |
| K4-6 | Desktop 타입을 스펙에서 만든다. `protocol.ts` 의 손 복사를 지운다 | 유지 | `npm run check`, vitest (생성 결과 시험), e2e |
| K4-7 | Desktop 이 변경 명령에 멱등 키를 보낸다 | 유지 | vitest, e2e |
| K4-8 | `akra kernel` CLI, `KernelCommandPort`, Gateway 클라이언트, 명령 전용 조립, 잠깐 붙는 접속, 잠금 파일의 쥔 쪽. skill 문서를 스펙에서 만들고 등록한다 | 확장 | CLI 시험, G2 시험, skill 파일 시험 |
| K4-9a | 입구 이전: planning 초기화 | 유지 (같은 결과. 기록에 남는다) | 입구 경계 시험, CLI·Telegram·admin 시험 |
| K4-9b | 입구 이전: 병렬 제어, 수동 배분, 프로필. admin, Telegram, `parallel-tick` 이 스케줄러를 시작하지 않는다 | 전환 (Gateway 의 커널이 병렬 상태의 주인이다) | K-D3 회귀 시험 (admin 의 정지가 커널에 닿는다), 시뮬레이션 |
| K4-9c | 입구 이전: PR 검증 명령 | 유지 | admin 시험 |
| K4-9d | 입구 이전: 작업·방향·초안·파일. `planning-tool` 도 `KernelCommandPort` 를 쓴다. 읽기 경로에서 프롬프트 기록 정리를 뺀다 | 전환 (고치기와 지우기에 revision 이 필수가 된다) | planning worker skill 시험 (모델 턴 없음), admin 시험, 입구 경계 시험 |
| K4-10 | MCP 도구와 SDK 를 스펙에서 만든다 | 확장 | 쓰는 쪽이 생길 때 정한다 |

- 순서의 이유: K4-1 이 나머지의 전제이다. K4-3, K4-4, K4-5 는 core 를 바꾸므로 시뮬레이션을 함께 넓힌다. K4-8 의 CLI 와
  G2 시험은 `next` (K4-3)와 `operation` (K4-4)을 쓴다. K4-6 과 K4-7 은 K4-1 뒤에 언제든 할 수 있다. K4-9 는 일의
  묶음마다 PR 하나이다.
- 규모: K4-1 은 3~4일, K4-2, K4-3, K4-5, K4-6, K4-7 은 각각 1~3일, K4-4 는 4~6일, K4-8 은 3~5일, K4-9 는 묶음마다
  2~4일로 추정한다. 조사의 추정이다.

## 5. 위험

| 위험 | 대응 |
| --- | --- |
| 스펙이 타입을 다시 적은 또 하나의 복사본이 된다 | 표는 `match` 로 만들고 모양은 타입에서 만든다. 손으로 적는 것은 뜻(설명, 효과, 다음 행동)뿐이다 |
| `schemars` 의 스키마가 serde 의 실제 전송 모양과 다르다 (내부 tag 의 newtype variant, `Option` 과 `skip_serializing_if`, 입력과 출력의 차이) | K4-1 에서 먼저 확인한다. 다른 타입은 스키마를 손으로 쓴다. 표본 값을 스키마로 검사하는 시험을 둔다 |
| 내보낸 파일이 낡는다 | 시험이 견준다. 고치는 명령은 하나이다 |
| Gateway 의 커널이 잠금을 쥐어 TUI 가 열리지 않는다 | 잠깐 붙는 접속은 남은 일이 없으면 바로 내린다 (§3.7) |
| 명령 하나를 위해 조립한 커널이 일을 이어 간다 (모델 턴, `lost` 닫기) | 명령 전용 조립은 되돌리기와 실행 장치를 시작하지 않는다. `state: store` 명령만 받는다 (§3.7) |
| TUI 와 admin 을 함께 쓰는 사용자 | K6 전까지 직접 경로를 남긴다 (§3.7, Q-K4-3) |
| sandbox 안의 planning worker 가 Gateway 에 닿지 못한다 | K4-9d 전에 확인한다. 닿지 않으면 `planning-tool` 은 직접 경로에 남기고 revision 검사만 더한다 |
| 이유 코드를 합치다가 전송 글자가 바뀐다 | 글자를 고정하는 시험을 먼저 쓴다. 기록의 이전 값도 읽는지 본다 |
| `op_id` 를 core 에 싣는 변경이 넓다 (correlation, thread session, 남긴 프롬프트, 재시작) | K4-4 를 한 PR 로 하지 않고 기록 종류마다 나눌 수 있다. 시뮬레이션이 지킨다 |
| 기록이 지워져 연산의 상태를 모른다 | `operation` 이 `unknown` 을 답한다 |

## 6. 미결이던 질문과 결정

사용자가 이 질문들을 작업자의 판단에 맡겼다 (2026-11-12). 아래의 값으로 정한다. 구현하다가 값이 맞지 않으면 그
조각의 PR 에서 이 표와 이유를 함께 고친다.

| ID | 질문 | 결정 (2026-11-12) |
| --- | --- | --- |
| Q-K4-1 | 스키마를 `schemars` 로 만들 것인가 (Rust 의 새 의존성 하나) | 쓴다. plan 18 §4.4 가 정한 방법이다. K4-1 의 확인에서 맞지 않는 타입만 손으로 쓴다 |
| Q-K4-2 | 에이전트용 CLI 의 이름 | `akra kernel <scope> <명령>` 과 `akra describe --json` |
| Q-K4-3 | TUI 가 커널을 쥔 저장소에서 다른 입구의 변경 명령 | K6 전까지 직접 경로를 남긴다. 다른 선택: (a) 이유 코드로 거절한다, (b) "TUI 가 Gateway 의 커널에 붙는다"(K6)를 앞당긴다 |
| Q-K4-4 | Gateway 가 없을 때 | `state: store` 명령은 명령 전용 조립으로 실행하고, `state: runtime` 명령은 거절한다. admin 과 Telegram 은 병렬 제어처럼 `runtime` 명령을 쓰려면 Gateway 가 필요하다. 다른 선택: 입구가 Gateway 를 시작한다 (Desktop 과 같은 규칙. app-server 가 함께 뜬다) |
| Q-K4-5 | `planning-tool` 을 `akra kernel` 로 합칠 것인가 | 합친다. `planning-tool` 을 지우고 planning worker 의 skill 과 계약 글을 같은 PR 에서 바꾼다. sandbox 확인(§3.7)이 실패하면 남긴다 |
| Q-K4-6 | MCP 도구와 SDK 의 때 | K4-10 으로 미룬다. 쓰는 쪽(셸이 없는 클라이언트, 데이터 작업)이 생기면 한다 |
| Q-K4-7 | Desktop 타입 생성기 | 이 스펙이 쓰는 스키마만 읽는 작은 스크립트를 쓴다. 다른 선택: npm 의 JSON Schema 생성기를 의존성으로 더한다 |

## 7. 리뷰 반영 (2026-11-12)

독립 리뷰 하나와 두 축 리뷰(표준, 스펙)를 받았다. 고친 것:

- 한 번 쓰는 커널은 명령 하나만 실행하지 않는다 (남긴 프롬프트의 되돌림, `lost` 닫기, 스케줄러). 명령 전용 조립과
  `state` 로 바꿨다 (§2.3, §3.7).
- Gateway 의 커널이 10분 동안 잠금을 쥐어 TUI 를 막는다. 잠깐 붙는 접속을 더했다 (§3.7).
- domain 의 표는 adapter 의 enum 을 쓰지 못한다. 표를 enum 이 있는 층으로 나누고 `KernelCommandKind` 를 더했다 (§3.1).
- 연산의 끝: 큐에 남김은 끝이 아니다. 턴 기록에 연산을 잇는 값이 없다. 재시작 때 열린 턴을 닫는 기록이 없다.
  `rename_session` 이 빠졌다. 상태 `lost`, `unknown` 을 더했다 (§3.4).
- 입구가 커널에 닿는 Port 와 adapter 의 이름과 층이 없었다 (§3.6). Gateway 클라이언트는 전송 타입을 쓰지 못한다.
- 단계의 순서: `operation` 조회가 그것을 쓰는 CLI 와 G2 시험보다 뒤에 있었다 (§4).
- 스펙의 필드: 출력 스키마, 권한 범위(`scope`), 멱등 여부를 더했다. `confirm`, `recorded` 는 `effect` 에서 나오므로 뺐다.
  `spec_version` 을 뺐다. 식별자와 설명의 이름을 `name`, `summary` 로 맞췄다 (§3.2).
- `wait_for` 의 조건이 글이었다. "그 이벤트가 다음에 올 때까지"로 뜻을 정했다 (§3.2).
- 목표 G3 과 §3.7 이 어긋났다 (TUI 가 쥔 저장소). G3 을 고쳤다.
- 빠진 변경(병렬 에이전트 프로필)을 더했다. 옮기지 않는 변경과 읽기를 적었다 (§3.7). 허용 목록을 비운다는 말을 뺐다.
- revision 필수의 범위를 고치기와 지우기로 정했다. `pr_validation_command` 를 동사가 있는 명령 5개로 나눴다.
- CLI 의 멱등 키 설명이 틀렸다 (응답을 받지 못한 호출자는 키도 받지 못한다) (§3.6).
- MCP 와 SDK, 기능 카탈로그, 전송 타입의 이름 규칙, 버린 프롬프트의 알림 경로가 자리 없이 빠져 있었다.
- plan 19 §5.4 의 "데이터에서 Rust 타입을 생성한다"와 plan 18 의 K4·K5 행을 이 설계에 맞게 고쳤다.

반려한 것:

- "domain 에 이름 enum 을 더하면 중복이다": 커널이 명령의 종류로 효과를 검사하고 기록해야 한다. 전송 글자는 한 곳에서만
  나오고, 빠짐은 컴파일러가 잡는다.
- "기록의 payload 스키마는 필요 없다": Desktop 이 `recorded` 이벤트의 타입을 쓰려면 필요하다. 내부 값은 감춘다.
- "CLI 이름을 `akra <그룹> <동작>` 으로": 지금의 사람용 명령과 부딪힌다. `akra describe --json` 만 그대로 둔다.

## 8. 조각의 결과

### K4-1 (2026-11-12)

만든 것:

- `schemars` 확인: 이 프로토콜의 모양(내부 tag enum 의 필드 variant, newtype variant, 빈 variant `{}`,
  `deny_unknown_fields`, `skip_serializing_if`, 기본값 필드)에서 스키마가 실제 전송 값을 받는다. 손으로 쓴 스키마는
  없다. 한 가지가 넓다: 없을 수 있는 필드(`Option`)는 스키마가 `null` 도 받는다. 커널은 그 필드를 `null` 로 보내지
  않고 빼지만, 입력에서는 `null` 을 받는다.
- 시험이 실제 값을 스키마로 검사한다: 모든 명령과 요청, 조회와 그 답, 값이 채워진 모든 이벤트, 모든 종류의 기록,
  attach 의 요청과 답, 명령의 답. 검사기는 이 프로토콜이 쓰는 JSON Schema 의 부분만 아는 시험용 함수이다. 스펙에 그
  함수가 모르는 키워드가 생기면 다른 시험이 먼저 실패한다. 검증기 의존성은 더하지 않았다.
- 입력 스키마가 코드의 검사 규칙을 가진다: 빈 글자를 받지 않는 필드(`minLength`, `pattern`), 멱등 키의 길이.
- 이름 목록은 매크로 `spec_names!` 한 곳에 적는다. 줄 하나가 값, 전송되는 이름, 한 문장 설명이다.
  - domain `kernel_spec`: `KernelCommandKind` 와 명령의 성질(`scope`, `effect`, `idempotency`, `rejections`),
    `KernelRecordKind`. 거절 이유 `CommandRejectionReason` 도 같은 매크로로 옮겼다.
  - adapter `kernel_protocol`: 메서드, 조회, 이벤트, 오류의 표. 메서드 이름 상수와 오류 코드 상수는 그 표에서
    나온다 (Gateway 의 `-32601` 포함).
  - 거절의 설명 글(`rejection.message`)은 이유의 표에서 온다. 따로 적은 문장을 지웠다.
  - 손으로 적은 `METHODS`, `COMMANDS`, `QUERIES`, `EVENTS`, `KERNEL_EVENT_KINDS` 를 지웠다.
- `describe` 가 스펙 전체(`KernelSpec`)를 답한다: 스키마의 방언(`$schema`, 2020-12), 메서드(요청인가 알림인가),
  명령, 조회, 이벤트, 기록, 이유, 오류, `scope`·`effect`·`idempotency` 값의 뜻(`terms`), 타입 정의(`$defs`).
  프로토콜 판은 1 이다. 명령·조회·이벤트·기록의 전송 모양은 바뀌지 않았다. 바뀐 값은 거절의 `message` 글이다.
- 조회의 답을 이벤트 enum 에서 나눴다 (`KernelQueryAnswer`). 전송 모양은 같다.
- `KernelCommandDescriptor.command` 가 글자에서 `KernelCommandKind` 로 바뀌었다. 기록할지는 `effect` 에서 나온다
  (`read` 는 기록하지 않는다). 따로 적은 목록이 없다.
- `schema/akra-kernel-spec.json` 을 저장소에 두었다. 시험이 `describe` 의 답과 같은지 본다. 고치는 명령은
  `bash scripts/refresh_akra_kernel_spec.sh` 이다.

설계와 다른 점:

- 스펙의 `state` 와 `completion` 은 아직 없다. 읽는 코드가 생기는 조각(K4-8 의 보내는 곳, K4-4 의 연산의 끝)에서
  더한다. `reasons` 와 `errors` 에는 `next` 가 없다 (K4-3).
- 명령의 성질 중 커널이 읽는 것은 "기록하는가"뿐이다. 나머지는 시험이 코드와 묶는다: `destructive` 명령은 입력에
  `confirm` 이 있고 없이 보내면 `-32010` 이다. 커널이 낸 거절 이유는 그 명령의 `rejections` 에 있어야 한다 (debug
  빌드의 검사. 시험과 시뮬레이션이 지난다). `scope` 와 `idempotency` 는 아직 선언이다 (K4-5).
- 설명(`summary`)은 영어로 쓴다. 에이전트와 생성되는 skill 문서가 읽기 때문이다.
- 스키마에는 필드의 설명이 없다. `schemars` 는 Rust 의 문서 주석을 설명으로 넣는데, 이 저장소의 주석은 개발자가
  읽는 글이라서 스키마에서 뺐다. 필드의 설명이 필요하면 skill 문서를 만들 때(K4-8) 표로 더한다.
- 이름 규칙(§3.2)은 새 타입에 썼다 (`KernelQueryAnswer`, 스키마의 `KernelRecord`). 이미 있는 타입의 이름은 바꾸지
  않았다. Desktop 타입을 생성하는 조각(K4-6)에서 생성되는 이름을 보고 정한다.
- 빠짐없는 `match` 는 명령과 기록에만 있다 (`KernelCommand::kind`, `KernelEvent::record_kind`). 조회와 이벤트는
  시험이 지킨다: 표의 이름들과 전송 타입의 스키마가 가진 이름들이 같아야 한다.

확인: domain 시험(이름이 겹치지 않음, 읽기 명령의 규칙, 모든 이유가 어느 명령에 속함), 프로토콜 시험(표와 전송
타입의 이름 일치, 이유와 오류의 값, 명령 항목의 뜻과 입력, 입력 규칙, `destructive` 의 확인 규칙, 실제 값의 스키마
검사, 검사기가 아는 키워드, 스펙 파일 일치), Gateway 시험(실제 커널의 `describe` 답), 구조 시험, 커널 시뮬레이션.

남긴 것:

- Desktop 의 시험 값은 아직 `protocol_version: 0` 으로 답한다. Desktop 은 이 값을 검사하지 않는다. Desktop 타입을
  생성하는 조각(K4-6)에서 고친다.
- 남긴 프롬프트(`thread_prompt_queued`)의 `handoff` 는 스펙에서만 감춘다. `recorded` 이벤트와 `history` 조회로는
  그대로 전송된다 (이 변경 이전부터 그렇다). 프롬프트 글이 클라이언트에 가므로, 전송에서도 뺄지는 K4-4 에서 정한다.

### K4-2 (2026-11-12)

만든 것:

- 병렬 상태의 네 값이 스펙에서 정해진 값의 목록이 되었다: 에이전트의 `state`, 병합 큐 항목의 `state`, PR 검증의
  `state` 와 `phase`. 이전에는 스펙에서 자유 글자(`string`)였다.
- 병합 큐와 PR 검증의 값은 domain 에 이미 enum 이 있었다. 프로토콜 enum 이 그 enum 을 빠짐없는 `match` 로 받는다.
  표시 이름을 글자로 바꾸던 `state_name` 을 지웠다.
- 에이전트의 상태는 domain 에서 글자였다. roster 가 보이는 상태를 enum 으로 정했다
  (`ParallelModeAgentRosterState`, 13개: lease 상태 셋과 전달 단계 열). roster 의 `state_label` 은 그 enum 의 이름에서
  나온다 (`roster_state`). 전달 단계를 고르는 `runtime_state_override` 도 그 enum 을 돌려준다. 그래서 이름이 한 곳에
  있다. 스펙의 `terms.agent_state` 가 13개 값의 뜻을 알린다.

설계와 다른 점:

- 전송되는 글자는 바뀌지 않았다. 설계는 "바뀜"으로 적었다. 이전 값이 이미 표시 이름을 snake_case 로 쓴 것이었고,
  새 enum 의 이름이 그것과 같다. 병합 큐와 PR 검증은 프로토콜 시험이 domain 의 모든 값에서 이전 글자(표시 이름)와
  같은지 본다. 에이전트는 domain 시험이 이전 글자 13개와 같은지 보고, 프로토콜 시험이 그 이름을 그대로 보내는지
  본다. 그래서 Desktop 은 고치지 않았다.
- 에이전트 상태에 `unknown` 이 있다. roster 가 만든 행에는 나오지 않는다. 손으로 만든 행(시험의 표본)의 이름이
  roster 의 이름이 아닐 때 쓴다. 이 값을 없애려면 roster 행이 글자 대신 enum 을 가져야 하고, TUI 와 admin 의 시험
  표본 약 20곳을 고쳐야 한다. 병렬 모드의 정리(plan 19 Q5)에서 한다.
- session detail 의 상태(`state_label`, 약 16개의 글자가 병렬 서비스와 TUI, admin, 저장된 투영에 걸쳐 있다)는 타입으로
  바꾸지 않았다. 커널 프로토콜은 그 값을 보내지 않는다. 병렬 모드의 정리(plan 19 Q5)에서 한다.

확인: 프로토콜 시험(domain 의 모든 값에서 전송 글자가 이전과 같다, 스펙의 값 목록과 필드의 타입), domain 시험(lease
상태와 전달 단계 열 개가 같은 이름의 상태가 된다), roster 를 읽는 화면의 시험(TUI, admin), 구조 시험, 스펙 파일 시험.

### K4-3 (2026-11-12)

만든 것:

- 이유 코드가 domain 의 `ReasonCode` 하나이다 (19개). `CommandRejectionReason`, `ThreadPromptRejection`,
  `PromptSubmissionReason` 을 지웠다. 전송되는 글자는 이전과 같다. domain 시험이 19개의 글자를 고정한다. 명령의
  `rejections` 는 앞의 9개이고, 나머지는 `prompt_submission` 의 이유이다.
- core 는 `ThreadPromptAdmission::rejection` 한 곳에서 이유 코드로 바꾼다. 영수증, 기록(`thread_prompt_rejected`),
  `prompt_submission` 이벤트가 그 값을 쓴다.
- `next` 가 데이터이다: 스펙의 `reasons[].next`, `errors[].next`, 명령 답의 `rejection.next`, `prompt_submission` 의
  `next`. 값은 adapter `kernel_protocol::next` 의 빠짐없는 `match` 하나에서 온다. `prompt_submission` 의 `message` 도
  이유의 `summary` 이다. `bootstrap_review_required` 와 `planning_refused` 만 초안 이름과 원인을 싣는다.
- 큐에 남긴 프롬프트를 내지 않고 끝낼 때 클라이언트에 알린다. 상태 `dropped` 를 더했고, 이유는 기록의 값을 그대로
  쓴다: `task_finished` (작업이 이미 끝났다), `rejected_again` (한 번 더 낸 것이 또 거절되었다).
- 시뮬레이션에 불변식 I10 (거절의 완전성)을 더했다: `command_rejected` 의 이유는 같은 연산의 명령 스펙 `rejections` 에
  있다. 기본 seed 들에서 거절이 한 번도 나지 않으면 시험이 실패한다. 시뮬레이션이 기록하는 명령은 `submit_prompt`
  뿐이므로 다른 명령의 거절(steer, 승인 결정)은 이 불변식이 보지 않는다.

설계와 다른 점:

- `next` 의 표는 domain 이 아니라 adapter 에 있다. 단계가 가리키는 조회와 이벤트의 이름은 adapter 의 표이고,
  domain 은 adapter 를 쓰지 못한다. 커널은 `next` 를 읽지 않는다.
- 단계 `wait { milliseconds }` 를 더했다. `steer_in_flight`, `decision_in_flight`, `parallel_mode_unknown` 은 그 일의
  끝을 알리는 이벤트가 없다. 없는 이벤트를 기다리라고 할 수 없어서 1초를 기다리게 했다. `preparation_active` 도 같다:
  거절 자체가 `prompt_submission` 을 내므로 그 이벤트를 기다리면 자기 거절에서 바로 풀린다 (리뷰에서 고쳤다).
- 단계 `method { name }` 를 더했다. `not_attached` 의 다음 행동은 `akra/kernel/attach` 를 보내고 다시 보내는 것이다.
- `wait_for` 에는 조건이 없다 (§3.2). 큐에 남은 프롬프트의 `wait_for prompt_submission` 은 다른 thread 의 이벤트에도
  풀린다. 클라이언트가 이벤트의 `task_id` 로 가린다. 이유의 설명이 그것을 적는다. 연산의 끝은 K4-4 의 `operation`
  조회가 정확히 알린다.
- `errors[].next` 는 스펙에만 있다. JSON-RPC 오류 응답은 `code` 와 `message` 이고, 클라이언트는 `code` 로 스펙의
  항목을 찾는다.
- 상태 `dropped` 와 이유 `rejected_again` 은 설계에 없었다. 한 번 더 낸 프롬프트가 또 거절되면 이전에는 `queued` 로
  알렸다. 커널은 그것을 다시 내지 않으므로 `queued` 의 `next` (`wait_for prompt_submission`)가 끝나지 않는다.
- `ThreadPromptRejected` 기록의 `reason` 스키마가 5개에서 19개의 목록으로 넓어졌다. 이전 기록은 그대로 읽힌다.
- 명령의 답과 `prompt_submission` 의 `message` 글이 바뀌었다 (판단에는 `reason` 을 쓴다). 프로토콜 판은 그대로 1 이다.
  필드(`next`)와 값(`dropped`, 이유 10개)을 더한 것이다.

확인: domain 시험(이유의 글자, 이전 기록의 거절 이유를 읽는다), 프로토콜 시험(모든 이유와 오류의 `next` 가 스펙의 이름을 가리킨다,
모든 이유가 클라이언트에 닿는다, 단계의 전송 모양, 이벤트와 답의 모양), core 시험(버린 프롬프트의 이벤트), 커널
시뮬레이션 (300 seed), 스펙 파일 시험.

남긴 것:

- `rejected_again` 의 core 경로(다시 낸 프롬프트가 또 거절됨)는 core 시험이 없다. 프로토콜 시험이 이벤트 모양을 본다.
- Desktop 은 `prompt_submission` 을 쓰지 않는다. `protocol.ts` 에 `dropped` 만 더했다. `next` 는 K4-6 의 생성 타입에서
  온다.
