# 22. 작업 컨텍스트 (work context)

상태: 1단계 완료 (스레드의 외부 문서 수집과 Desktop 목록) · 기준일: 2026-11-11 ·
관련: plan 18(kernel 프로토콜), plan 20(observe-and-offer), plan 21(MCP hub)

## 한 줄 요약

Akra 가 스레드, 작업, 프로젝트에 관련된 외부 문서(Notion, Jira, GitHub 등)를 **모델 없이 코드로** 수집해 장부에
적는다. AI 에게는 지금 하는 일과 맞는 문서만, 필요한 만큼만 준다. 수집, 변경 확인, 표시는 모델 토큰을 쓰지 않는다.

## 왜 필요한가

- Codex 앱은 세션마다 AI 가 도구로 관련 문서를 다시 찾고 전체를 다시 읽는다. 찾는 턴과 읽는 토큰이 매번 든다.
- Akra 에는 저장소별 kernel, planning 작업, 브랜치와 PR, 상주 Gateway 가 있다. 관계를 저장하고 이어 쓸 수 있다.
- 이 문서는 처음의 "스레드 연결(thread binding)" 안을 대신한다. 사람이 문서를 직접 묶는 대신, Akra 가 흔적에서
  관계를 찾는다. 사람의 고정과 제외는 자동 판단을 고치는 선택 기능이다.

## 원칙

1. **수집은 코드가 한다.** 주소와 키는 정확히 읽을 수 있는 흔적(메시지, 명령, 도구 인자, 브랜치, 커밋, PR)에서 뽑는다.
2. **push 는 좁게, pull 은 넓게.** 자동으로 주는 것은 지금 하는 일과 맞는 문서의 한 줄 목록뿐이다. 본문은 AI 나
   사람이 고를 때만 준다. 프로젝트 전체의 문서는 찾을 수 있는 색인으로만 둔다.
3. **관계는 "문서 ↔ 프로젝트"가 아니라 "문서 ↔ 범위"이다.** 범위는 스레드, 파일 경로, 작업이다. 서버 작업 세션에
   GUI 티켓을 주지 않으려면 문서가 어떤 코드 경로와 함께 다뤄졌는지를 알아야 한다.
4. **늦게 묶는다.** 세션을 열 때는 초점을 모른다. AI 가 파일을 만지기 시작한 뒤에 맞는 문서를 준다.
5. **읽은 것은 다시 주지 않는다.** 스레드가 문서의 어느 버전까지 읽었는지 기록하고, 그 뒤의 변경분만 준다.

## 구조

```text
신호 수집 ──> 인식기 ──> 장부 ──> 전달 조정 ──> AI / 사람
(스레드 기록, git)        (문서, 범위, 읽은 버전)
                              ↑
                         신선도 확인 (MCP hub, Jira REST, gh)
```

### 인식기 (외부 서비스를 늘리는 자리)

`src/domain/work_context.rs` 의 `ExternalResourceRecognizer` 이다. 서비스 하나가 구현 하나이다.

```text
kind() -> "notion" | "jira" | "github" | …
recognize(주소의 호스트와 경로) -> 문서(kind, key, url, title_hint) 또는 없음
```

- 새 서비스는 구현 하나를 `external_resource_recognizers()` 에 더한다. 장부, 수집기, Gateway, Desktop 은 종류
  이름만 다루므로 바꾸지 않는다. Desktop 은 모르는 종류를 종류 이름 그대로 보인다.
- `(kind, key)` 가 같으면 같은 문서이다. Notion 은 페이지 id, Jira 는 `사이트/키`, GitHub 는 `owner/repo#번호` 이다.
- 변경 확인(2단계)은 종류마다 제공자 포트(`version`, `changes`)를 따로 둔다. 인식과 조회는 다른 경계이다.

### 장부

`$AKRA_HOME/work-context/ledger.sqlite`, 사용자당 하나이다.

| 표 | 내용 |
|---|---|
| `resources` | 문서: `kind`, `key`, 주소, 제목 힌트 |
| `links` | 문서 ↔ 범위(`scope_kind`, `scope_id`), 처음 본 곳(`user`, `agent`, `tool`), 처음 본 시각 |
| `scan_markers` | 범위마다 마지막으로 읽은 위치 |

범위 종류는 글자이다. 지금은 `thread` 만 쓴다. `path`(파일 폴더)와 `task`(planning 작업, 브랜치)는 같은 표에 더한다.

### 수집기

- `WorkContextService` 가 스레드의 새 기록만 읽고(`thread/items/list` 를 새 것부터, 읽은 아이템에서 멈춤), 인식기로
  문서를 찾아 장부에 잇는다.
- 읽는 글: 사람과 AI 의 메시지, 실행한 명령, MCP 도구 인자. 명령 출력과 도구 결과는 읽지 않는다. 크고, 문서와
  무관한 주소가 많다.
- 주인은 Gateway 이다. 저장소 kernel 은 10분 쉬면 내려가므로 사용자 단위 장부의 주인으로 맞지 않는다.

## 1단계 (완료, 2026-11-11)

- Gateway 메서드 `akra/context/thread { threadId }` → `{ resources: [{ kind, key, url, title, source, firstSeenAt }] }`.
  부를 때 새 기록을 수집한 뒤 그 스레드의 문서를 돌려준다.
- 인식기: Notion 페이지, Jira 티켓(`/browse/KEY-123`, Cloud 와 Server), GitHub PR 과 issue.
- Desktop: 스레드를 열 때와 턴이 끝날 때 읽는다. 패널이 닫혀 있어도 한다. 상단 "문서 N" 버튼과 "관련 문서" 패널이
  종류별로 보여 준다. 링크는 기본 브라우저로 연다.
- 계층: Gateway `context.rs` → `WorkContextPort` → `WorkContextService` → `ThreadTranscriptPort`(app-server),
  `WorkContextStorePort`(SQLite).
- 확인 (실제 `akra-gateway`, 격리된 `AKRA_HOME`, 실제 저장된 스레드 8개, 내용은 출력하지 않고 개수만):
  문서 0~21개를 찾았다. 첫 수집 0.1~1.1초, 두 번째부터 약 80ms 였고 결과가 같았다. Desktop 화면 시험은 가짜
  Gateway 로 숨긴 창에서 돌렸다.
- 함께 알게 된 것:
  - `thread/items/list` 는 ephemeral 스레드에서 `not supported yet` 로 실패한다. 턴이 없는 새 스레드는 문서가 없다.
  - `thread/inject_items` 로 넣은 글은 모델 기록에만 들어가고 스레드 아이템 목록과 화면에는 나오지 않는다. 그래서
    4단계의 자동 한 줄은 사람에게 따로 보여야 한다.

## 남은 단계 (proposed)

2. **변경 확인**: 종류별 제공자 포트. Notion 은 MCP hub 의 `notion-get-version-changes`(`start_time` = 마지막 확인
   시각, B0 에서 확인)를 쓰고, 편집 권한이 없는 페이지는 `notion-fetch` 의 `page_last_edited_at` 을 쓴다. Jira 는
   이슈의 `updated`, GitHub 는 `gh` 이다. 스레드가 활성일 때만, 유효 시간이 지난 문서만 확인한다. 장부에
   `읽은 버전`을 더하고 Desktop 목록에 "바뀜"을 보인다.
3. **경로 범위와 `akra context`**: 스레드가 고친 파일의 폴더, 문서가 연결된 커밋과 PR 의 경로를 `path` 범위로 잇는다.
   AI 용 `akra context list|show|changes` 와 skill 을 더한다. 새 스레드가 같은 경로를 만지면 관련 문서를 찾는다.
   키워드 검색은 SQLite FTS5 로 한다. 벡터 검색은 놓친 사례를 측정한 뒤에 검색 포트 뒤에 더한다.
4. **자동 한 줄 전달**: 초점이 생긴 뒤 턴 경계에서 맞는 문서 목록(상한 3개)을 스레드에 넣는다. 사람의 고정과 제외,
   열어 본 문서의 가점, 무시된 문서의 감점을 더한다.
5. **쓰기 승인**: MCP hub 의 쓰기 도구 호출을 Desktop 승인으로 보낸다. `mcp` 출처만 있는 변경은 AI 자신의 변경으로
   보고 알림에서 뺀다.

## 비용

- 수집, 변경 확인, 표시: 모델 토큰 0. app-server 조회, 정규 규칙, SQLite, 외부 서비스 조회뿐이다.
- AI 에게 주는 목록: 문서당 한 줄(약 15토큰), 상한이 있다.
- AI 가 변경분이나 전체를 읽을 때: 읽은 분량만큼이다. 변경분을 먼저 주어 전체 읽기를 줄인다.
- 절감 효과는 측정해야 안다. 같은 작업을 이 기능이 있을 때와 없을 때로 돌려 토큰을 비교하는 기준을 3단계 전에 정한다.

## 한계와 열린 질문

- 링크 없이 키만 쓴 Jira 언급(`ABC-123`)과 제목만 말한 Notion 문서는 찾지 못한다.
- 흔적이 없는 새 문서는 범위를 모른다. 검색으로만 나온다.
- 폴더 구분이 약한 저장소에서는 경로 기준 연결의 정확도가 떨어진다. 그래서 자동 전달은 한 줄과 상한으로 묶는다.
- 처음 여는 아주 긴 스레드는 최근 5,000 아이템만 본다.
- `akra` 가 PATH 에 없는 환경에서는 에이전트가 `akra mcp`, `akra context` 를 실행하지 못한다.
- `akra mcp login` 이 app-server 에 `timeoutSecs` 를 넘기지 않아 app-server 의 기본 제한 시간에 먼저 끝난다.
