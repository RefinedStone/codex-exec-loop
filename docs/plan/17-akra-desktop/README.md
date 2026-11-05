# Akra Desktop 계획

## 문서 상태

- 상태: **Proposed plan**. 이 디렉터리는 현재 제품 동작을 설명하지 않는다. 현재 동작의 기준은
  [`docs/reference/`](../../reference/) 이다.
- 기준일: 2026-11-05 (Asia/Seoul)
- 기준 커밋: `0732aea87dd221b12c2cbeb2f84810a4a2f73111` (`origin/prerelease`)
- 독자: 이 계획을 이어받는 작업자 (사람 또는 코딩 에이전트)
- 목적: Akra 에 항상 실행되는 Gateway 데몬과 Electron 데스크톱 앱(이하 **Akra Desktop**)을
  추가한다. 사용자는 Codex 대화와 업무 도구를 앱 하나에서 처리한다.

## 요약

Akra 는 `codex app-server` 를 감싸는 Rust 클라이언트이다. 이 계획은 Akra 를 다음 구조로 확장한다.

```text
Akra Desktop (Electron)  --WebSocket-->  akra-gateway (데몬)  --WebSocket-->  codex app-server
  renderer(창) <-> main                    얇은 중계 + akra/* 확장            공식 엔진, 원본 대화 저장
```

- 엔진은 공식 Codex 를 그대로 쓴다. 응답 품질은 공식 Codex 앱과 같다.
- Gateway 는 app-server 프로토콜을 바꾸지 않고 전달한다. 나중에 필요한 메서드만 가로챈다.
- MVP 는 공식 Codex 앱의 "대화" 부분만 만든다. diff, 터미널, 브라우저, 업무 패널, 하네스 연동,
  Claude 연결은 MVP 이후 단계이다.

## 읽는 순서

1. [01 배경과 목표](01-background-and-goals.md): 왜 만드는지, 무엇을 만들지 않는지
2. [03 결정 기록](03-decisions.md): 승인된 결정, 승인 전 제안, 제외 항목
3. [04 목표 구조](04-target-architecture.md): 프로세스, 데이터 소유, 연결 규칙, 장애 처리
4. 진행할 단계 문서 ([단계와 상태](#단계와-상태) 참고)
5. [06 인계 가이드](06-handoff-guide.md): 환경, 계정, 저장소 규칙, 명령, 금지 사항

용어가 낯설면 [02 용어](02-glossary.md) 를 먼저 읽는다. 근거 자료는 [`reference/`](reference/) 에 있다.

## 문서 목록

| 파일 | 내용 |
| --- | --- |
| [01-background-and-goals.md](01-background-and-goals.md) | 사용자 상황, 목표, 비목표, 제약, 성공 기준, 검토 경과 |
| [02-glossary.md](02-glossary.md) | 계층(L0~L4), Codex 프로토콜, Gateway, Electron 용어 |
| [03-decisions.md](03-decisions.md) | 결정 D-01~D-15, 제안 P-01~P-09 |
| [04-target-architecture.md](04-target-architecture.md) | 목표 구조, 연결 규칙, 흐름, 장애 처리, 보안, 배치 |
| [05-risks-and-open-questions.md](05-risks-and-open-questions.md) | 위험 K-01~K-10, 미결 질문 Q-01~Q-11 |
| [06-handoff-guide.md](06-handoff-guide.md) | 작업 환경, 계정, 저장소 절차, 명령, 사용량 정책, 금지 사항 |
| [reference/codex-app-server.md](reference/codex-app-server.md) | 공개 Codex app-server: 실행 형태, 프로토콜, 다중 접속 규칙 |
| [reference/codex-desktop-app.md](reference/codex-desktop-app.md) | 공식 Codex(ChatGPT) 데스크톱 앱의 구조 관찰 |
| [reference/akra-current.md](reference/akra-current.md) | 현재 Akra 구조, 대화 경로, 테스트 기준선 |
| [reference/alternatives.md](reference/alternatives.md) | 검토한 대안과 라이선스 (Orca, Claude, Tauri 등) |
| [phases/P0-preflight-verification.md](phases/P0-preflight-verification.md) | 사전 검증 기록 |
| [phases/P1-gateway-daemon.md](phases/P1-gateway-daemon.md) | Gateway 데몬 |
| [phases/P2-desktop-shell.md](phases/P2-desktop-shell.md) | Electron 골격과 Gateway 연결 |
| [phases/P3-mvp-conversation.md](phases/P3-mvp-conversation.md) | MVP 대화 화면 |
| [roadmap/R1-change-review.md](roadmap/R1-change-review.md) | 변경 검토 (diff) |
| [roadmap/R2-terminal.md](roadmap/R2-terminal.md) | 터미널 |
| [roadmap/R3-browser-panel.md](roadmap/R3-browser-panel.md) | 브라우저 패널 |
| [roadmap/R4-work-panels.md](roadmap/R4-work-panels.md) | 업무 패널 (GitHub, Jira, Slack) |
| [roadmap/R5-harness-integration.md](roadmap/R5-harness-integration.md) | Akra 하네스 연동 |
| [roadmap/R6-context-compaction.md](roadmap/R6-context-compaction.md) | 자체 compact |
| [roadmap/R7-claude-backend.md](roadmap/R7-claude-backend.md) | Claude 백엔드 |
| [roadmap/R8-packaging-and-operations.md](roadmap/R8-packaging-and-operations.md) | 배포와 운영 |
| [scripts/](scripts/) | P0 재현 스크립트 (Python: macOS·Linux, Node: Windows 포함) |

## 결정 요약

전체 내용과 상태 정의는 [03 결정 기록](03-decisions.md) 에 있다.

| ID | 결정 | 상태 |
| --- | --- | --- |
| D-01 | 출발점은 Akra + Electron. 코드 전체를 직접 소유한다 | 승인 |
| D-02 | 엔진은 Codex 먼저. Claude 는 앱 완성 후 확장한다 | 승인 |
| D-03 | 클라우드 에이전트(OpenAI dots) 비공개 백엔드 연결은 하지 않는다 | 제외 |
| D-04 | 저장소는 Akra 저장소 안의 `desktop/` (모노레포) | 승인 |
| D-05 | 기준 데이터 A+: 원본 대화는 Codex, 파생 데이터만 Akra | 승인 |
| D-06 | Gateway 는 항상 실행되는 데몬이다 | 승인 |
| D-07 | free 플랜 대응 기능은 지금 만들지 않는다 | 승인 |
| D-08 | TUI 는 유지한다 | 승인 |
| D-09 | 전송은 WebSocket + JSON-RPC. renderer 대신 main 이 접속한다 | 승인 |
| D-10 | 컴퓨터당 데몬 1개. 작업공간은 Codex Project 를 쓴다 | 승인 |
| D-11 | 얇은 Gateway: app-server 프로토콜 그대로 + `akra/*` + interceptor | 승인 |
| D-12 | MVP 는 대화 부분만 만든다 | 승인 |
| D-13 | 개발과 검증에서 Codex 사용량을 최소화한다 | 승인 |
| D-14 | Akra 저장소는 RefinedStone 신원으로만 작업한다. 회사 계정과 분리한다 | 승인 |
| D-15 | macOS 와 Windows 를 함께 지원한다 | 승인 |

## 단계와 상태

규모는 전업 1인이 코딩 에이전트를 함께 쓰는 기준의 추정이다.

| 단계 | 내용 | 상태 | 선행 조건 | 규모 |
| --- | --- | --- | --- | --- |
| [P0](phases/P0-preflight-verification.md) | 사전 검증 (빌드, 호환성, 다중 접속) | 완료 (2026-11-05) | - | - |
| [P1](phases/P1-gateway-daemon.md) | Gateway 데몬 | 구현 완료 (macOS 실기, Q-03·Q-09 실측 남음) | Q-10, P-01~P-04, P-09 승인 (2026-11-05) | 약 1.5주 (Windows 포함) |
| [P2](phases/P2-desktop-shell.md) | Electron 골격과 Gateway 연결 | 진행 중 (골격 완료, 피드백 대기) | P1 | 약 1주 |
| [P3](phases/P3-mvp-conversation.md) | MVP 대화 화면 | 대기 | P2 | 2~3주 |
| [R1](roadmap/R1-change-review.md) | 변경 검토 (diff) | 계획 | P3 | 1~2주 |
| [R2](roadmap/R2-terminal.md) | 터미널 | 계획 | P3 | 1주 |
| [R3](roadmap/R3-browser-panel.md) | 브라우저 패널 | 계획 | P3 | 1~2주 |
| [R4](roadmap/R4-work-panels.md) | 업무 패널 | 계획 | P3 | 패널당 약 1주 |
| [R5](roadmap/R5-harness-integration.md) | Akra 하네스 연동 | 계획 | P3, Q-01 | 2~4주 |
| [R6](roadmap/R6-context-compaction.md) | 자체 compact | 계획 | P3, Q-02 | 미정 |
| [R7](roadmap/R7-claude-backend.md) | Claude 백엔드 | 계획 | P3 | 3~5주 |
| [R8](roadmap/R8-packaging-and-operations.md) | 배포와 운영 | 계획 | P3 | 1~2주 |

R 단계의 순서는 고정하지 않는다. 사용자가 실제로 불편을 느낀 순서대로 진행한다.

## 변경 규칙

- 결정이 바뀌면 [03](03-decisions.md) 에 새 항목을 추가한다. 이전 항목은 지우지 않고
  "대체됨" 으로 표시한다.
- 단계가 끝나면 그 단계 문서의 상태와 위 표를 고친다. 끝난 동작의 설명은 `docs/reference/` 로
  옮기고, 이 계획에는 링크만 남긴다.
- 버전, 경로, 측정값은 측정한 날짜와 함께 적는다.
- 이 저장소는 공개 저장소이다. 자격 증명, 회사 정보, 비공개 서비스의 내부 구조를 적지 않는다.
