# R7. Claude 백엔드

상태: 계획 · 기준일: 2026-11-06 (최초 2026-11-05) · 선행: P3 · 규모: 4~5주 (추정)

## 1. 목표

1. 같은 앱, 같은 대화 화면에서 Claude Code 엔진으로 대화한다. 앱 코드는 바꾸지 않는다 (D-11).
2. **Akra 하네스(계획 큐, 병렬 슬롯, 검증, PR 전달)를 Claude 위에서도 그대로 실행한다.**

2번이 이 단계의 핵심이다. "Codex 와 Claude 위에서 같은 규칙 기반 외부 루프"는 공식 Codex 앱과 Orca 에 없는
조합이다. Orca 는 Codex 와 Claude 의 구조화 채팅을 이미 지원하므로, 대화 화면(1번)만으로는 차별점이 되지 않는다.

## 2. 연결 방식 두 가지 (P-11)

| 기준 | stream-json (기본) | 기록 파일 읽기 (보조) |
| --- | --- | --- |
| Claude 실행 | `claude -p --input-format stream-json --output-format stream-json --verbose` | 터미널의 일반 `claude` |
| 화면 데이터 | stdout 의 JSON 이벤트 | Claude Code 가 쓰는 세션 기록 파일(JSONL) |
| 응답 스트리밍 | 된다 | 안 된다. 기록은 메시지 단위로 쓰인다 |
| 입력, 중단, steer | JSON 메시지로 정확히 보낸다 | 터미널 키 입력으로 간접 전달 |
| 승인 처리 | 권한 도구(`--permission-prompt-tool`)나 hooks 로 앱이 처리 | 터미널에서 처리. 앱 화면에 승인 카드를 띄우기 어렵다 |
| 공식 기능 | 비대화형 모드라 일부 TUI 기능이 빠진다 | 전부 그대로 쓴다 |
| 형식 안정성 | 문서화된 입출력 형식 | **비공식 내부 형식.** 버전마다 바뀔 수 있다 |
| **하네스 사용** | **가능.** 세션 시작, 입력, 완료 판정, 중단을 프로그램으로 한다 | **불가.** 보기 전용이다 |

결론: Akra 가 새로 시작하는 Claude 세션은 모두 stream-json 으로 실행한다. 기록 파일 읽기는 다른 곳에서 한
대화를 보여 주고 이어 가는 용도로만 쓴다 (§6).

참고: Orca 도 두 방식을 함께 쓴다. 구조화 채팅은 stream-json(Agent SDK 방식)이고, 터미널 기반 채팅은 기록
파일을 읽는다. 터미널 화면 출력은 해석하지 않는다. 이 계획은 Orca 의 설계만 참고한다.

## 3. 구조

```text
                 Akra application (하네스: 계획, 슬롯, 검증, PR 전달. 엔진과 무관)
                   | outbound 포트 3개
        +----------+---------------------------+
        |                                      |
adapter/outbound/app_server (현재)      adapter/outbound/claude_cli (신규)
        |                                      |
codex app-server                         ClaudeCliSession (공통 드라이버)
                                               | stdio, 줄 단위 JSON
Akra Desktop -> Gateway -> 백엔드 선택 ---------+--> claude -p --input-format stream-json ...
                (스레드마다)   (app-server 형식 흉내)
```

- **ClaudeCliSession** 은 Claude CLI 실행, stream-json 입출력, 세션 id, 중단을 맡는 공통 드라이버이다. 하네스
  어댑터와 Gateway 가 함께 쓴다.
- 하네스 경로: outbound 어댑터가 포트를 구현하고, Claude 이벤트를 Akra 의 대화 이벤트로 바꾼다 (§4).
- 데스크톱 경로: Gateway 가 스레드마다 백엔드를 고른다. Claude 스레드이면 앱의 `thread/*`, `turn/*` 요청을
  드라이버 호출로 바꾸고, 결과를 app-server 형식의 알림과 서버 요청으로 바꿔 보낸다 (§5).

## 4. 하네스 연결 (코드 확인 2026-11-06)

### 4.1 하네스가 엔진을 부르는 지점

하네스는 엔진을 outbound 포트 3개로만 부른다. 현재 구현은 `src/adapter/outbound/app_server/` 하나이다.

| 포트 | 메서드 | 용도 |
| --- | --- | --- |
| `PlanningWorkerPort` | `run_planning_session` | 계획 초안 작성 |
| `ParallelAgentWorkerPort` | `run_isolated_new_thread_stream` | 병렬 슬롯의 작업 실행 |
| `InteractiveTurnRuntimePort` | 턴 시작·스트리밍, steer, 승인, 중단, 스냅샷 | TUI 대화형 실행 |

- 검증, git, PR 전달, CI 처리는 엔진과 관계없다. 바꾸지 않는다.
- Akra 는 Codex 의 dynamic tools 를 쓰지 않는다. `item/tool/call` 을 "구현하지 않음"으로 거절한다. 그래서
  Codex 전용 도구 연결에 묶여 있지 않다.
- 결과: Claude 용 outbound 어댑터가 위 포트 3개를 구현하면 하네스 본체는 거의 바꾸지 않고 Claude 위에서 돈다.

### 4.2 맞춰야 할 개념

| Akra 가 Codex 에 쓰는 것 | Claude 대응 |
| --- | --- |
| 이벤트 → `ConversationStreamEvent` 16종 | stream-json 이벤트(시스템, 메시지, 도구 사용, 도구 결과, 결과)를 같은 이벤트로 바꾼다 |
| 턴 완료 판정 | stream-json 의 결과(result) 이벤트 |
| 샌드박스·승인 정책 | 권한 모드(`--permission-mode`), 허용 도구 목록(`--allowedTools`), 설정 파일 |
| 계획 작업자용 개발자 지시문 | `--append-system-prompt` |
| 작업 폴더 신뢰 설정 (Codex project trust) | Claude 의 작업 폴더 권한 설정 |
| 스레드 재개 | `--resume <세션 id>` |
| 모델 선택 | `--model` |

### 4.3 무인 실행의 승인 정책

병렬 슬롯은 사람이 지켜보지 않는다. 승인 요청에서 멈추면 안 된다.

- 슬롯용 권한 정책을 미리 정한다. 예: 슬롯 worktree 안의 파일 편집과 정해진 명령(빌드, 테스트, git)은 자동
  허용한다.
- 정책 밖의 요청은 거부하고, 작업을 "막힘(blocked)"으로 기록한다. Akra 의 기존 typed block 개념을 쓴다.
- 정책은 Codex 슬롯과 같은 수준이 되도록 맞춘다. 엔진에 따라 안전 수준이 달라지면 안 된다.

### 4.4 엔진 선택

- 하네스 작업: 병렬 작업자 프로필(`parallel_agent_profile_repository_port`)에 엔진 종류를 넣는 방법을 먼저
  검토한다. 작업마다 Codex 와 Claude 를 고른다.
- 대화: 새 스레드를 만들 때 고른다. 이후 그 스레드의 엔진은 바꾸지 않는다.

## 5. 데스크톱 연결 (app-server 형식 흉내)

| app-server | Claude CLI |
| --- | --- |
| `thread/start` | 새 세션 (첫 메시지 때 시작) |
| `thread/resume` | `--resume <세션 id>` |
| `thread/list` | Gateway 가 세션 목록을 관리한다 (CLI 에 목록 API 가 없다). 다른 곳에서 한 대화는 §6 의 기록 파일로 보충한다 |
| `turn/start` | 입력 스트림에 사용자 메시지 |
| `turn/steer` | 입력 스트림에 메시지 추가 |
| `turn/interrupt` | 중단 신호 |
| 승인 요청 | `--permission-prompt-tool` (MCP) 또는 hooks 로 받아서 서버 요청으로 바꾼다 |
| `item/*` 알림 | stream-json 이벤트를 아이템으로 바꾼다 |

## 6. 기록 파일 읽기 (보조)

- 용도:
  - 터미널이나 Claude 데스크톱 앱에서 한 대화의 목록과 내용을 보여 준다.
  - 그 대화를 Akra 에서 이어 간다. 기록 파일에서 세션 id 를 찾고, stream-json 에서 `--resume <세션 id>` 로
    잇는다. 원래 세션이 아직 실행 중이면 먼저 멈추도록 안내한다. 한 대화에 쓰는 프로세스는 하나로 유지한다.
- 위치: Claude Code 의 세션 기록 JSONL (`~/.claude/projects/` 아래).
- 형식이 비공식이다. 모르는 필드와 줄은 건너뛴다. 버전별 시험 기록(내용을 바꾼 사본)으로 회귀를 막는다.
- 기록에는 대화 원문이 있다. 회사 코드가 들어 있을 수 있으므로 읽기만 한다. 다른 곳에 복사하거나 저장하지
  않는다.

## 7. 제약

- **공식 CLI 를 수정하지 않고 실행한다.** 로그인은 `claude` 자체 절차로 한다. Akra 는 Claude 의 자격 증명과
  세션 토큰을 읽거나 저장하거나 중계하지 않는다.
- **약관** (2026-11-06, Claude Code 공식 문서 "Legal and compliance" 확인):
  - 구독의 사용량 한도는 Claude Code 와 Agent SDK 의 일반적인 개인 사용을 전제로 한다.
  - 제품이나 서비스를 만드는 개발자는 API 키를 써야 한다. 다른 사용자의 요청을 구독 자격 증명으로 중계하면 안 된다.
  - 수정하지 않은 Claude Code 바이너리에 사용자가 자기 구독으로 로그인하는 것은 허용된다.
  - Akra 는 사용자 본인만 쓴다. 하네스가 자동 루프를 많이 돌리면 "일반 사용"을 넘는다고 볼 여지가 있다. 자동
    실행이 많아지면 API 키로 바꾼다 (미결 4).
- 회사가 지원하는 Claude 계정을 쓰는 경우 회사의 사용 정책을 따른다 (D-14).
- 엔진 소스가 비공개라서 동작 변경을 미리 알 수 없다. stream-json 과 기록 형식 변경에 대비해 시험을 둔다.
- Claude 사용량에도 D-13 의 원칙을 적용한다. 시험은 녹화 재생으로 하고, 실제 호출은 사용자 허락 후에 한다.

## 8. 단계

| 단계 | 내용 | 완료 기준 | 규모 |
| --- | --- | --- | --- |
| C1 드라이버 | ClaudeCliSession: 실행, stream-json 입출력, 세션 id, 재개, 중단 | 녹화 재생 시험 통과. 허락받은 실측 1~2회로 녹화 확보 | 약 1주 |
| C2 하네스 어댑터 | 포트 3개 구현, 이벤트 변환, 무인 승인 정책, 엔진 선택 설정 | TUI 와 병렬 슬롯에서 Claude 작업이 끝까지 실행된다 (계획 → 슬롯 → 검증 → PR) | 1.5~2주 |
| C3 데스크톱 | Gateway 백엔드 선택, app-server 형식 흉내 | Akra Desktop 대화 화면에서 Claude 스레드를 쓴다 | 1~1.5주 |
| C4 기록 파일 | 세션 목록, 보기, 이어 가기 | 터미널에서 한 대화를 앱에서 보고 이어 간다 | 3~5일 |

C2 와 C3 의 순서는 사용자의 우선순위에 따라 바꿀 수 있다. 차별점은 C2 에서 나온다.

## 9. 미결

| 번호 | 질문 | 기본안 |
| --- | --- | --- |
| 1 | Gateway 의 백엔드 선택 기준 | 스레드 생성 시 선택. Project metadata 는 `project/*` 가 안정 API 가 된 뒤 |
| 2 | Claude 세션과 작업 폴더의 연결 정보를 어디에 저장하는가 | A+ 원칙에 따라 Akra 파생 데이터로 저장한다 |
| 3 | 슬롯 무인 실행의 권한 정책 기본값 | Codex 슬롯과 같은 수준으로 맞춘다 (C2 에서 확정) |
| 4 | 구독과 API 키를 나누는 기준 | 대화와 소량의 하네스 실행은 구독, 자동 실행이 많아지면 API 키 |
| 5 | 회사 Claude 계정을 개인 도구에서 써도 되는가 | 회사에 확인한다. 확인 전에는 개인 계정만 쓴다 |
