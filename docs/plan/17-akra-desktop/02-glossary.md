# 02. 용어

상태: Proposed · 기준일: 2026-11-05

이 계획의 모든 문서는 아래 뜻으로 용어를 쓴다.

## 1. 계층

| 용어 | 뜻 | 예 |
| --- | --- | --- |
| L0 모델 | 모델 자체 | GPT, Claude |
| L1 모델 API | 모델을 호출하는 HTTP API | Responses API, Messages API |
| L2 엔진 | 에이전트 하네스. 내부 루프, 시스템 프롬프트, 도구, compact, 샌드박스, 세션 저장 | Codex core, Claude Code |
| L3 인터페이스 | 엔진을 외부 프로그램에 노출하는 통로 | `codex app-server` JSON-RPC, `claude -p` stream-json |
| L4 클라이언트 | 엔진 위의 화면과 외부 루프(오케스트레이터) | 공식 Codex 앱, Akra |

| 용어 | 뜻 |
| --- | --- |
| 내부 루프 | 엔진 안에서 "모델 호출 → 도구 실행 → 다시 호출"을 반복하는 루프 |
| 외부 루프 | 엔진 밖에서 "작업 선택 → 실행 → 검증 → 다음 작업"을 반복하는 루프 |
| 루프 엔지니어링 | 외부 루프를 설계하는 일 |
| 하네스 엔지니어링 | 에이전트의 작업 환경(도구, 권한, 지시 파일, 테스트)을 설계하는 일 |
| 래핑 | 공식 L2 를 그대로 쓰고 L3 로 연결하는 방식. Akra 와 이 계획의 방식이다 |
| 재구현 | L2 를 직접 만드는 방식. 이 계획에서는 하지 않는다 |

## 2. Codex 프로토콜

| 용어 | 뜻 |
| --- | --- |
| app-server | Codex 엔진을 JSON-RPC 로 노출하는 프로세스 (`codex app-server`) |
| 접속 | app-server 와의 연결 하나. 접속마다 initialize 를 따로 한다 |
| Project | 작업공간. 이름, 경로 목록(roots), metadata(문자열 맵), 순서를 가진다 |
| Thread | 대화 하나. cwd, projectId, sectionId 를 가진다 |
| Turn | 사용자 입력 하나와 그 처리 과정 |
| Item | 턴 안의 단위. agentMessage, reasoning, commandExecution, fileChange, mcpToolCall, plan 등 |
| Section | 사용자가 만든 스레드 그룹 |
| 구독 | 접속이 어떤 스레드의 알림을 받는 상태. thread/start, thread/resume, thread/fork 로 시작하고 thread/unsubscribe 또는 접속 종료로 끝난다 |
| 알림 | 서버가 보내는 id 없는 메시지 (예: `item/agentMessage/delta`) |
| 서버 요청 | 서버가 클라이언트에 보내는 요청. 클라이언트가 응답해야 한다. 승인 요청이 여기에 속한다 |
| 승인 요청 | 명령 실행, 파일 변경, 권한 확대 전에 사람의 결정을 묻는 서버 요청 |
| humanCommand | 관리형 규칙이 요구하는 특별한 승인 요청. `requiredHumanApproval` 를 선언한 접속에만 간다 |
| experimentalApi | initialize 에서 실험 메서드와 필드를 받겠다고 선언하는 값 |
| ephemeral 스레드 | 기록을 남기지 않는 스레드 (`thread/start` 의 `ephemeral: true`) |
| compact | 문맥이 길어지면 이전 대화를 요약해서 줄이는 동작 |
| delta | 스트리밍 응답의 조각 |

## 3. 이 계획의 용어

| 용어 | 뜻 |
| --- | --- |
| Akra Desktop | 이 계획에서 만드는 Electron 앱 |
| Gateway | `akra-gateway` 데몬 안의 중계 계층. 클라이언트 접속을 받아 app-server 로 전달한다 |
| 데몬 | 로그인 동안 계속 실행되는 프로세스 |
| upstream | Gateway 가 실행하고 연결하는 `codex app-server` 프로세스 |
| 클라이언트 | Gateway 에 접속하는 쪽. Akra Desktop 의 창, 시험 스크립트, 나중의 다른 도구 |
| interceptor | Gateway 에서 특정 메서드를 가로채서 수정하는 지점. MVP 에서는 비어 있다 |
| `akra/*` 메서드 | Gateway 가 직접 처리하는 확장 메서드의 이름 공간 |
| A+ | 원본 대화는 Codex 가 갖고 Akra 는 파생 데이터만 갖는 원칙 (D-05) |
| 하네스 기능 | Akra 의 계획 큐, worktree 슬롯, 검증, PR 전달 |
| 녹화 파일 | 실제 접속의 JSON-RPC 메시지를 순서대로 저장한 파일. 화면 시험에 재생한다 (P1, P3) |

## 4. Electron

| 용어 | 뜻 |
| --- | --- |
| main | Electron 의 메인 프로세스. Node 권한을 가진다. 창, 파일, 네트워크, 자식 프로세스를 다룬다 |
| renderer | 창 안의 웹 화면 프로세스. 이 계획에서는 Node 권한이 없다 |
| preload | renderer 가 쓸 수 있는 함수만 골라 노출하는 연결 스크립트 |
| contextIsolation | preload 와 웹 화면의 JavaScript 실행 공간을 분리하는 설정 |
| sandbox | renderer 프로세스의 OS 권한을 제한하는 설정 |
| IPC | main 과 renderer 사이의 메시지 통신 |
| 메시지 버스 | 기능마다 IPC 채널을 만들지 않고 `{type, ...}` 메시지 하나의 통로로 주고받는 방식 |

## 5. 혼동하기 쉬운 말

| 말 | 구분 |
| --- | --- |
| 토큰 | 이 계획에서 "토큰 사용"은 **모델 사용량**을 뜻한다. 로그인 정보는 "자격 증명"이라고 쓴다 |
| 데몬 | `akra-gateway` 를 뜻한다. Codex 관리형 데몬(`codex app-server daemon`)은 "Codex 관리형 데몬"이라고 쓴다 |
| 세션 | Gateway 의 세션은 클라이언트 접속 하나이다. Codex 의 대화는 Thread 라고 쓴다 |
| 작업공간 | Codex Project 를 뜻한다. Akra 의 계획 작업 단위는 "작업(task)"이라고 쓴다 |
