# 22. 스레드 연결(thread binding) 계약

상태: proposed · 기준일: 2026-11-11 · 관련: plan 18(kernel 프로토콜), plan 20(observe-and-offer), plan 21(MCP hub)

## 한 줄 요약

"이 스레드는 이 문서를 같이 본다"는 약속을 Akra 에 등록한다. 그 약속을 지키는 일(변경 확인, 알림, 쓰기 승인)은
AI 에게 부탁하지 않고 Akra 가 코드로 한다. 확인과 알림에는 모델 토큰을 쓰지 않는다.

## 문제

- Notion 패널의 "스레드로 넘기기"는 페이지를 한 번 복사한다. 그 뒤 사람이 페이지를 고쳐도 스레드는 모른다.
- AI 가 외부 문서를 고칠 때 사람의 확인이 없다. skill 문서의 규칙("요청한 변경만 한다")에만 기댄다.
- 외부 문서가 바뀌어도 사람과 AI 모두 알림을 받지 못한다.
- Notion 전용으로 이 문제를 풀면 Jira, GitHub PR 에서 같은 코드를 다시 만들어야 한다.

## 용어

- **자원(resource)**: 스레드 밖에 있는 문서 하나이다. 식별자는 `notion://page/<id>`, `jira://<site>/<key>`,
  `github://<owner>/<repo>/pull/<n>` 처럼 쓴다.
- **연결(binding)**: `{ thread_id, resource, seen_version, notified_version }` 이다.
  - `seen_version`: AI 가 마지막으로 읽거나 쓴 자원의 버전이다.
  - `notified_version`: 마지막으로 "바뀜"을 알린 버전이다. 같은 변경을 두 번 알리지 않는다.
- **버전**: 자원의 수정 시각이나 수정 번호이다. Notion 은 `page_last_edited_at` 이다.

## 인터페이스

### 명령과 이벤트 (사람과 AI 가 같이 쓴다)

| 이름 | 보내는 쪽 | 내용 |
|---|---|---|
| `bind` | Desktop("스레드로 넘기기"), AI(`akra bind`) | 스레드와 자원을 묶는다. 이때의 버전을 `seen_version` 으로 기록한다 |
| `unbind` | Desktop, AI | 묶음을 푼다 |
| `list` | Desktop, AI | 스레드의 묶음과 각 자원의 상태(최신, 바뀜)를 돌려준다 |
| `resource_changed` (이벤트) | Akra | 묶인 자원이 `seen_version` 뒤에 바뀌었다 |

Gateway 의 `akra/binding/*` 메서드로 내보낸다. plan 18 의 kernel 프로토콜과 같은 모양(명령, 조회, 이벤트)을
따른다.

### 자원 제공자 포트

```text
ResourceProviderPort
  parse(url) -> resource          // 이 제공자의 주소인지 판단하고 식별자로 바꾼다
  version(resource) -> version    // 가장 가벼운 방법으로 현재 버전만 읽는다
  title(resource) -> string       // 알림에 쓸 이름이다
```

- Notion: MCP hub(plan 21)로 읽는다. 처음에는 `notion-fetch` 결과의 `page_last_edited_at` 을 쓴다. 공식 MCP 에
  더 가벼운 도구가 있으면 그것으로 바꾼다. 필요하면 같은 포트 뒤에 REST API(`GET /v1/pages/{id}`)를 넣는다.
- Jira, GitHub: 기존 Desktop 서비스(`jiraService.ts`, `githubService.ts`)가 읽는 값을 쓴다 (나중 단계).

## 행동 규약

규칙은 Akra 코드가 지킨다. skill 문서는 AI 에게 규칙을 설명만 한다.

| 규칙 | 언제 | Akra 가 하는 일 | 모델 토큰 |
|---|---|---|---|
| R1 바뀜 알림 | 묶인 자원이 `seen_version` 뒤에 바뀌고 스레드가 쉬고 있을 때 | `thread/inject_items` 로 스레드 기록에 한 줄을 넣는다: "Notion 페이지 X 가 T 에 바뀌었다. 이어서 작업하기 전에 다시 읽어라" | 감지할 때 0. 다음 턴에 한 줄 |
| R2 쓰기 승인 | AI 가 hub 로 묶인 자원을 바꾸는 도구를 부를 때 | Desktop 승인 화면에 띄우고 답을 기다린다 | 0 |
| R3 유휴 중 제안 | R1 과 같은 때 | `resource_changed` 이벤트를 낸다. Desktop 은 "AI 에게 반영할까요?"를 보인다 | 사람이 누르기 전까지 0 |
| R4 읽기와 쓰기 기록 | AI 가 hub 로 묶인 자원을 읽거나 쓸 때 | `seen_version` 을 새 버전으로 바꾼다 | 0 |

- R1 은 턴 시작을 가로채지 않는다. Desktop 의 턴은 kernel 을 거치지 않고 app-server 로 바로 가기 때문이다.
  대신 변경을 알면 쉬는 스레드의 기록에 미리 넣어 두고, 다음 턴이 그것을 읽는다. 턴이 도는 중이면 끝난 뒤에 넣는다.
- R4 는 hub 가 AI 의 모든 Notion 호출을 중계하므로 가능하다. AI 가 hub 를 거치지 않고 읽으면 기록되지 않는다.

## 소유자와 저장

- 연결은 사용자 단위이고 Gateway 가 켜져 있는 동안 지켜야 한다. 저장소 단위 kernel 은 10분 쉬면 내려가므로
  감시자로 맞지 않는다. 그래서 연결 저장소와 감시자는 Gateway 의 application service 로 둔다.
- 저장: `$AKRA_HOME/bindings/` 의 사용자 전용 파일(SQLite 또는 JSON)이다. 스레드 id 를 키로 쓴다.
- 확인 주기: 묶인 자원만, 기본 1분이다. Notion 사용량 제한을 넘지 않도록 자원 수에 따라 늘린다.

## 비용

- 감시, 비교, 알림, 승인은 코드이다. MCP 호출은 Notion 서버로 가고 모델로 가지 않는다.
- 모델 토큰은 다음 턴의 알림 한 줄(수십 토큰)과, AI 가 다시 읽기로 정했을 때 문서 내용만큼 든다.
- Notion 호출량: 묶인 자원 수 × 확인 횟수이다. `notion-fetch` 는 본문 전체를 받으므로 가벼운 버전 조회가 중요하다.

## 단계

1. **B0 확인 (사용자 필요)**: 실제 Notion 로그인 후 공식 MCP 의 도구 목록에서 가벼운 버전 조회 도구를 찾는다.
2. **B1 연결과 R1**: Gateway 의 연결 저장소, `akra/binding/bind|unbind|list`, Notion 제공자, 감시자, R1 알림.
   Desktop 의 "스레드로 넘기기"가 `bind` 도 보낸다.
3. **B2 R3, R4**: `resource_changed` 이벤트와 Desktop 제안, hub 의 읽기와 쓰기 기록.
4. **B3 R2**: hub 의 쓰기 도구 호출을 Desktop 승인으로 보낸다. 도구의 `readOnlyHint` 로 쓰기 도구를 가린다.
5. **B4 다른 자원**: Jira, GitHub PR 제공자.

## 열린 질문

- 공식 Notion MCP 에 수정 시각만 읽는 도구가 있는가 (B0).
- `thread/inject_items` 로 넣은 메시지의 역할(`developer` 또는 `user`)과, 쉬는 스레드에 넣을 때 기록과 화면에
  어떻게 보이는가. 실제 app-server 로 확인해야 한다.
- `akra` 가 PATH 에 없는 환경이 있다. 이때 skill 의 `akra mcp` 와 `akra bind` 를 에이전트가 실행하지 못한다.
  Gateway 가 skill 문서에 실행 파일의 절대 경로를 적거나, 설치 단계에서 PATH 를 맞춰야 한다.
