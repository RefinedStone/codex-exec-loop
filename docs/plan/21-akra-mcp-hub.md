# 21. Akra MCP hub

상태: 3단계 완료 (CLI + skill, Gateway 상주 엔진, Desktop Notion 패널) · 기준일: 2026-11-11 · 관련: plan 17 R4(업무 패널), plan 20(observe-and-offer)

## 목표

- 사용자는 MCP 서버(예: Notion)를 Akra 에 한 번 등록하고 한 번 로그인한다. 모든 세션과 Akra 가 그 연결을 같이 쓴다.
- 세션은 MCP 도구 스키마를 미리 싣지 않는다. 필요할 때 찾아서 부른다. 서버가 늘어도 세션 컨텍스트 비용이
  거의 늘지 않는다.
- 에이전트가 외부 서비스를 양방향으로 쓴다. 읽기와 쓰기 모두 같은 경로이다.

## 배경

- Codex `config.toml` 에 MCP 서버를 직접 등록하면 모든 세션에 도구 스키마가 실린다. Notion MCP 하나가 도구
  80여 개이다.
- Codex 앱의 Notion 플러그인은 ChatGPT 커넥터(`codex_apps`, `https://chatgpt.com`)를 거친다. 회사 데이터가 개인
  ChatGPT 계정을 거친다 (plan 17 D-14).
- Notion 공식 MCP(`https://mcp.notion.com/mcp`)는 동적 클라이언트 등록(DCR)과 PKCE 를 지원한다. 사내 앱
  등록 없이 OAuth 로그인할 수 있다. Slack 공식 MCP 와 다르다.

## 결정

- **엔진은 Codex 이다.** hub 는 전용 `codex app-server` 를 `CODEX_HOME=$AKRA_HOME/mcp-hub` 로 띄운다. MCP 전송
  (stdio, streamable HTTP), OAuth, 토큰 보관과 갱신은 Codex 가 한다. Akra 는 HTTP 클라이언트나 OAuth 를 만들지
  않는다. AGENTS.md 의 "공식 Codex 인터페이스 우선"과 같은 방향이다.
- **필요할 때만 연결한다.** `akra mcp` 명령 하나가 app-server 하나를 띄우고 끝나면 정리한다. 호출은 대상 서버만
  켠 ephemeral 스레드(`thread/start` 의 `config` 에 `mcp_servers.<다른 이름>.enabled = false`)에서 한다.
  실측: 가짜 stdio 서버 기준 명령 하나가 약 0.24초이다 (app-server 시작 포함).
- **세션에는 skill 로 붙인다.** 서버가 하나라도 있으면 hub 가 `agent-skills/akra-mcp-hub/SKILL.md` 를 쓰고,
  Akra 의 app-server 연결이 initialize 직후 `skills/extraRoots/set` 으로 그 폴더를 등록한다. 세션에는 skill
  설명 한 줄만 실린다. Codex 가 `CODEX_HOME/skills/.system` 에 자체 skill 을 설치하므로 hub skill 은 따로 둔다.
- **plugin 으로 포장하지 않는다.** Akra 가 세션을 직접 열므로 skill root 등록으로 충분하다.
- **설정 해석 전에 분기한다.** `akra mcp` 는 저장소 설정을 읽지 않는다. 에이전트가 어느 폴더에서 실행해도 된다.

## 1단계 (완료, 2026-11-10)

- `akra mcp servers|add|remove|login|search|describe|call`. 출력은 JSON 이다. `call` 은 인자를 인라인 JSON
  이나 stdin(`-`)으로 받는다. 도구 결과의 `isError` 가 true 이면 종료 코드 1 이다.
- `login` 은 인증 주소를 stderr 에 보이고 `mcpServer/oauthLogin/completed` 를 최대 10분 기다린다.
- 등록부는 hub 폴더의 `config.toml` 이고 Codex 형식 그대로이다. 손으로 Codex 옵션을 더 적어도 보존된다.
- app-server 가 hub 에 묻는 요청(MCP elicitation 등)은 거절한다. 호출이 멈추지 않는다.
- 계층: CLI -> `McpHubPort` -> `McpHubService` -> `McpHubBackendPort` -> `CodexMcpHubBackend`.
- 확인: 격리된 `AKRA_HOME` 에서 가짜 stdio 서버의 add, servers, search, describe, call(인라인, stdin) 과 Notion
  등록 후 `notLoggedIn` 표시를 실제 app-server 로 확인했다. 실제 Notion 로그인과 호출은 하지 않았다.

## 2단계 (완료, 2026-11-11): Gateway 상주 엔진

- Gateway 가 hub 엔진(hub 전용 app-server)을 첫 요청에 띄우고 유지한다. 서버별 ephemeral 스레드를 다시 쓰므로
  MCP 연결과 원격 handshake 를 매번 하지 않는다. 10분 동안 쓰지 않으면 커널 정리 주기(30초)에 내린다. 오류가
  나면 엔진을 버리고 다음 요청이 새로 띄운다. 요청은 하나씩 처리한다.
- Gateway 메서드: `akra/mcp/statuses`, `akra/mcp/call`, `akra/mcp/reload`. 모든 요청에 `hubHome` 을 싣는다.
  Windows 의 Gateway pipe 이름은 `AKRA_HOME` 이 아니라 사용자에 묶이므로, hub 폴더가 다르면 Gateway 는
  `-32040` 으로 거절한다.
- CLI 는 `servers`, `search`, `describe`, `call` 을 Gateway 로 먼저 보낸다. 직접 실행으로 돌아가는 경우는 셋뿐이다:
  접속하지 못했을 때, 모르는 메서드(오래된 Gateway)나 다른 hub 폴더라는 답을 받았을 때, Gateway 가 upstream 을
  열지 못해 1013 으로 닫았을 때. 요청을 보낸 뒤의 다른 실패는 오류로 끝낸다. 그래서 쓰기 도구가 두 번 실행되지
  않는다.
- `add`, `remove`, `login` 은 CLI 가 직접 하고, 끝나면 `akra/mcp/reload` 로 상주 엔진을 내린다.
- Gateway 와 클라이언트의 endpoint 이름은 `src/gateway_endpoint.rs` 하나에서 정한다.
- 확인: 격리된 `AKRA_HOME` 의 실제 `akra-gateway` 에서 가짜 stdio 서버 호출이 첫 번째 293ms, 그 뒤 약 55ms
  였다 (직접 실행은 약 400ms). 서버 추가 뒤 상주 엔진이 내려갔다. 다른 `AKRA_HOME` 은 직접 실행했다. Gateway 를
  끈 뒤에도 직접 실행했고, 남은 app-server 는 없었다.

## 3단계 (완료, 2026-11-11): Desktop Notion 패널

- 결정: 패널도 MCP hub 를 쓴다 (사용자 선택). 로그인은 hub 의 OAuth 하나이고, 페이지마다 integration 을 연결하는
  일이 없다. REST API 의 정해진 JSON 대신 MCP 결과를 방어적으로 읽는다.
- renderer 가 창의 Gateway 접속으로 `akra/mcp/statuses` 와 `akra/mcp/call` 을 부른다. main 은 Notion 주소만
  기본 브라우저로 연다(`https` 의 `notion.so`, `notion.com`, `*.notion.site`). Desktop 은 `hubHome` 을 싣지 않고,
  Gateway 는 이런 요청을 자기 hub 로 처리한다.
- 도구 이름은 서버의 도구 목록에서 고른다. 공식 Notion MCP 는 `notion-search`/`notion-fetch`, ChatGPT 커넥터는
  `search`/`fetch` 이다. `notion-list-recent-pages` 가 있으면 열 때 최근 본 페이지를 보여 준다.
- 결과 형식(커넥터로 값 없이 구조만 확인): 최근 목록은 글 JSON `{results:[{type,url,title}], nextCursor}`, 검색은
  `{results:[{id,title,url}]}`(구조화 결과도 있음), fetch 는 `{title,url,text,path,page_last_edited_at,...}` 이다.
- 화면: 서버가 없으면 `akra mcp add`/`login` 명령을, 로그인이 필요하면 `login` 명령을 보여 준다. 준비되면 최근
  페이지, 검색, 제목 클릭(브라우저), "스레드로 넘기기"(페이지 내용으로 새 스레드의 입력창만 채움)이다.
- 확인: 가짜 Gateway 와 가짜 MCP hub 로 화면 시험(`test/e2e/notion-panel.mjs`)을 숨긴 창에서 돌렸다. 실제
  Notion 로그인과 호출은 하지 않았다.

## 남은 단계 (proposed)

- **도구 목록 캐시.** `search` 가 모든 서버에 연결하지 않도록 서버별 도구 목록을 저장한다.
- **쓰기 정책.** 서버별, 도구별로 읽기 허용과 쓰기 승인을 정한다. 지금은 명령 승인 정책과 skill 규칙에 맡긴다.

## 알려진 한계

- Gateway 가 없고 기본 정책(`danger-full-access`)도 아니면, 에이전트의 `akra mcp` 직접 실행이 네트워크와 hub
  폴더 쓰기 승인을 요구할 수 있다. Gateway 가 있으면 조회와 호출은 로컬 IPC 만 쓴다.
- 상주 엔진은 요청을 하나씩 처리한다. 긴 도구 호출 하나가 다른 세션의 호출을 기다리게 한다.
- skill root 는 app-server 연결을 열 때 정해진다. 서버를 처음 추가한 뒤에는 새 연결부터 skill 이 보인다.
- `search` 는 켜진 모든 서버에 연결한다. 로그인하지 않은 서버는 오류로 표시되고 도구가 0개이다.
