# R7. Claude 백엔드

상태: 계획 · 기준일: 2026-11-05 · 선행: P3 · 규모: 3~5주

## 목표

같은 앱, 같은 대화 화면에서 Claude Code 엔진으로 대화한다. 앱 코드는 바꾸지 않는다 (D-11).

## 방식

```text
Akra Desktop  ->  Gateway  -> 백엔드 선택 (스레드마다)
                                 +-> codex app-server     (현재)
                                 +-> Claude 어댑터 (신규) -> claude CLI (공식 바이너리)
```

- Claude 어댑터는 **app-server 형식을 흉내 낸다.** 앱이 보내는 `thread/*`, `turn/*` 요청을 받아서 Claude
  CLI 로 실행하고, 결과를 app-server 형식의 알림과 서버 요청으로 바꿔 보낸다.
- Claude CLI 실행: `claude -p --input-format stream-json --output-format stream-json --verbose`.
  stdio 로 줄 단위 JSON 을 주고받는다. Akra 가 app-server 를 다루는 방식과 같다.

## 대응표 (초안)

| app-server | Claude CLI |
| --- | --- |
| `thread/start` | 새 세션 (첫 메시지 때 시작) |
| `thread/resume` | `--resume <session-id>` |
| `thread/list` | Gateway 가 세션 목록을 관리한다 (CLI 에 목록 API 가 없다) |
| `turn/start` | 입력 스트림에 사용자 메시지 |
| `turn/steer` | 입력 스트림에 메시지 추가 |
| `turn/interrupt` | 중단 신호 |
| 승인 요청 | `--permission-prompt-tool` (MCP) 또는 hooks 로 받아서 서버 요청으로 바꾼다 |
| `item/*` 알림 | stream-json 이벤트(메시지, 도구 사용, 결과)를 아이템으로 바꾼다 |

## 제약

- **공식 CLI 만 실행한다.** CLI 의 로그인 정보를 꺼내 다른 곳에서 쓰지 않는다. 개인 구독의 OAuth 자격
  증명을 서드파티 제품에서 쓰는 것은 약관으로 금지되어 있다 ([alternatives.md](../reference/alternatives.md) §2).
- 회사가 지원하는 Claude 계정을 쓰는 경우 회사의 사용 정책을 따른다 (D-14).
- 엔진 소스가 비공개라서 동작 변경을 미리 알 수 없다. stream-json 형식 변경에 대비해 어댑터에 시험을 둔다.

## 미결

- Gateway 의 백엔드 선택 기준: Project metadata, 스레드 생성 시 선택, 또는 둘 다
- Claude 세션의 Project 연결 정보를 어디에 저장할지 (A+ 원칙상 Akra 파생 데이터)
