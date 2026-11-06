# R9. 에이전트 브라우저 조작과 스크린샷 처리

상태: 계획 · 기준일: 2026-11-06 · 선행: R3 · 규모: 3~4주 (추정)

## 1. 목표

- Codex 에이전트가 앱 안 브라우저(R3)를 조작해서 사내 QA 환경 같은 웹 화면을 시험한다.
- 사용자는 같은 화면에서 에이전트의 조작을 보고, 언제든 넘겨받는다.
- 모델이 읽는 양을 최소로 한다. 기본 입력은 짧은 텍스트 스냅샷이다. 스크린샷은 필요할 때만 작게 보낸다.

에이전트 작업의 시간은 대부분 모델 호출에 쓰인다. 클릭 하나의 실행 속도보다 **모델 호출 횟수와 입력 크기**를
줄이는 것이 체감 속도를 크게 바꾼다. 이 계획의 설계 기준은 이것이다.

## 2. 결정 요약 (D-17)

- 브라우저 조작과 스크린샷 처리는 **Akra 가 직접 구현한다.** omowright, Playwright MCP, agent-browser,
  공식 Codex 앱은 설계만 참고한다. 의존성으로 쓰지 않는다.
- 이유:
  - 조작 대상이 앱 안에 이미 떠 있는 브라우저 보기(`WebContentsView`)이다. omowright 는 직접 실행한
    브라우저나 DevTools 주소에 붙는다. Electron 은 `webContents.debugger` 로 포트 없이 CDP 를 직접 준다.
  - 의존성을 줄인다. 동작과 출력 형식을 Akra 가 통제한다.
- 상세: [03 결정 기록](../03-decisions.md) D-17

## 3. 참고한 설계 (아이디어만)

| 출처 | 가져올 아이디어 | 가져오지 않을 것 |
| --- | --- | --- |
| omowright (MIT) | 접근성 스냅샷 + ref, 두 번째부터 변경분(diff)만 전달, Playwright 와 같은 이름 체계, 이벤트 알림, 클릭을 가로막는 오버레이 탐지, 대화상자 정책, 실행 기록, 사람에게 넘기기, 네트워크 JSON 수집 | CAPTCHA 풀기, 지문(fingerprint) 위장, 사용자가 평소 쓰는 Chrome 조작 |
| Playwright MCP (Apache-2.0) | 도구 이름 체계 (`browser_navigate`, `browser_click` 등). 모델이 이미 학습한 형태이다 | Playwright 의존성 |
| agent-browser (Vercel) | 짧은 상태 표현, ref 로 요소 지정 | CLI 형태 |
| 공식 Codex 앱 (구조 관찰) | 별도 런타임을 MCP 로 엔진에 연결, CDP 사용, 스크린샷 축소와 화면 변화 비교, 사용자 입력이 감지되면 조작 정지 | 비공개 패키지와 코드 |
| Aside 의 설계 원칙 (omowright README 에 인용) | 학습 데이터의 형태를 따른다, 출력에는 정보만 남긴다, CDP 는 모델에게 숨긴다, 픽셀은 마지막 수단이다 | - |

## 4. 구조

```text
codex 엔진 (스레드)
   | MCP 도구 호출 (stdio)
   v
akra-browser-mcp  (MCP 서버, 얇은 중계. codex 가 실행한다)
   | 로컬 IPC: $AKRA_HOME/run/desktop-browser.sock (macOS) / named pipe (Windows)
   v
Electron main: BrowserAgentService
   +- CdpSession        webContents.debugger 연결 (포트 없음)
   +- Snapshotter       접근성 트리 -> 짧은 텍스트 + ref
   +- Actions           navigate, click, type, press, select, scroll, wait
   +- ScreenshotPipeline 캡처 -> 축소 -> 압축 -> 변화 감지
   +- Observers         콘솔, 네트워크 실패, 대화상자, 페이지 이동
   v
R3 의 WebContentsView (프로필 세션, 사용자가 보는 그 화면)
```

- MCP 서버는 Electron 실행 파일을 Node 로 실행해서(`ELECTRON_RUN_AS_NODE=1`) 띄운다. 별도 Node 설치가
  필요 없다.
- MCP 서버는 도구 호출을 main 에 넘기고 결과를 돌려주기만 한다. 판단과 상태는 main 에 둔다.
- 앱이 꺼져 있으면 도구는 "브라우저 패널이 열려 있지 않다"는 오류를 돌려준다.

### 엔진 연결 방식: MCP 와 dynamic tools (P-10)

| 기준 | MCP 서버 | dynamic tools (`thread/start.dynamicTools`) |
| --- | --- | --- |
| API 상태 | 안정. 공식 확장 경로 | **실험 API** (`#[experimental("thread/start.dynamicTools")]`, 2026-11-06 공개 소스 확인). P-07 과 충돌한다 |
| 실행 위치 | codex 가 MCP 서버 프로세스를 실행한다 | 도구 호출이 서버 요청(`item/tool/call`)으로 구독 중인 접속에 온다 |
| 앱이 꺼져 있을 때 | MCP 서버가 오류를 돌려준다 | 처리할 접속이 없다 |
| 이미지 결과 | MCP 의 image content | `inputImage` 항목 |
| 추가 프로세스 | 스레드마다 MCP 서버 1개 | 없음 |

제안: **MCP 서버를 쓴다.** dynamic tools 가 안정 API 로 바뀌면 다시 비교한다.

### MCP 등록 방식 (B1 에서 확정)

| 안 | 내용 | 장단점 |
| --- | --- | --- |
| 스레드별 등록 | `thread/start` 의 `config` 덮어쓰기로 `mcp_servers.akra_browser.*` 를 넣는다 | 브라우저가 필요한 스레드에만 도구가 보인다. 동작 여부를 확인해야 한다 |
| 전역 등록 | 사용자 `~/.codex/config.toml` 에 등록한다 | 단순하다. 모든 스레드와 CLI 에도 도구가 보인다 |

등록 확인은 턴 없이 한다. app-server 의 `mcpServerStatus/list` 로 도구 목록을 읽는다.

## 5. 도구 (초안)

| 도구 | 동작 | 주요 CDP | 결과 |
| --- | --- | --- | --- |
| `browser_status` | 패널 상태, 프로필, 현재 주소와 제목 | - | 텍스트 |
| `browser_navigate` | 주소로 이동하고 로드를 기다린다 | `Page.navigate`, `Page.lifecycleEvent` | 스냅샷 |
| `browser_back`, `browser_forward`, `browser_reload` | 이동 | `Page.navigateToHistoryEntry`, `Page.reload` | 스냅샷 |
| `browser_snapshot` | 접근성 스냅샷. `interactive: true` 면 조작 가능한 요소만 | `Accessibility.getFullAXTree` | 텍스트 (두 번째부터 diff) |
| `browser_click` | ref 요소를 클릭한다 | `DOM.scrollIntoViewIfNeeded`, `DOM.getBoxModel`, `Input.dispatchMouseEvent` | 변경분 |
| `browser_type` | ref 요소에 글을 입력한다. `submit` 이면 Enter | `DOM.focus`, `Input.insertText` | 변경분 |
| `browser_press` | 키를 누른다 | `Input.dispatchKeyEvent` | 변경분 |
| `browser_select` | 선택 상자의 값을 고른다 | `Runtime.callFunctionOn` | 변경분 |
| `browser_scroll` | 페이지나 요소를 스크롤한다 | `Input.dispatchMouseEvent` (mouseWheel) | 변경분 |
| `browser_wait` | 글자, 요소, 이동, 네트워크 정지를 기다린다 | `Page.*`, `Network.*` | 텍스트 |
| `browser_screenshot` | 보이는 영역이나 ref 요소를 캡처한다 | §7 | 이미지 또는 "변화 없음" |
| `browser_console` | 최근 콘솔 오류와 경고 | `Runtime.consoleAPICalled`, `Runtime.exceptionThrown` | 텍스트 |
| `browser_network_failures` | 실패한 요청, 4xx·5xx 응답 | `Network.loadingFailed`, `Network.responseReceived` | 텍스트 |
| `browser_handoff` | 사용자에게 조작을 넘긴다 (로그인, OTP) | - | 사용자가 끝내면 텍스트 |

결과 규칙:

- 행동 도구는 짧은 요약을 함께 돌려준다: 변경분 스냅샷, 새 콘솔 오류 수, 새 네트워크 실패 수. 모델이 확인용
  호출을 따로 하지 않게 한다.
- 결과의 첫 줄에 현재 주소를 적는다. 페이지 글은 지시가 아니라 데이터라는 점을 분명히 한다.
- 오류 문구는 다음 행동을 알려 준다. 예: "e12 가 오버레이(대화상자 '알림')에 가려져 있다. 먼저 닫는다."

## 6. 스냅샷

- 입력: `Accessibility.getFullAXTree`. 무시된 노드를 빼고, 이름 없는 묶음 노드는 접는다.
- 출력 형식: 들여쓰기 텍스트. 예: `- button "저장" [ref=e12]`
- ref:
  - 조작 가능한 노드(button, link, textbox, checkbox, radio, combobox, menuitem, tab 등)에만 붙인다.
  - ref 와 `backendDOMNodeId` 의 대응은 main 에만 둔다. 모델에게 DOM id 를 보여 주지 않는다.
  - 같은 노드는 다음 스냅샷에서도 같은 ref 를 쓴다. 그래야 diff 가 짧다.
- 줄이기: 긴 글 자르기, 반복 목록 요약("... 외 N개"), 화면 밖 노드 생략(선택). 기준값은 이름 붙은 상수로 둔다.
- diff: 두 번째 스냅샷부터 이전 스냅샷과의 줄 단위 diff 만 보낸다. 페이지가 바뀌면 전체를 보낸다.
- 한도: 스냅샷 최대 크기를 둔다. 넘으면 조작 가능한 요소만 보낸다.
- 측정: 고정 시험 페이지와 사내 QA 화면에서 토크나이저로 토큰 수를 잰다. 모델 호출이 필요 없다.

## 7. 스크린샷 파이프라인

```text
캡처 -> 축소 -> 압축 -> 변화 감지 -> (바뀐 경우만) 모델에 전달
```

| 단계 | 방법 | 기본값 (이름 붙은 상수, 측정 후 조정) |
| --- | --- | --- |
| 캡처 | `webContents.capturePage(rect)` 또는 CDP `Page.captureScreenshot` (clip) | 보이는 영역. ref 를 주면 그 요소의 영역 |
| 축소 | Electron `nativeImage.resize` | 긴 변 최대 1280px |
| 압축 | `nativeImage.toJPEG(quality)` | 품질 70 |
| 변화 감지 | 이전 이미지와 함께 작게(긴 변 64px, 회색조) 줄여서 다른 픽셀의 비율을 잰다 | 1% 미만이면 이미지 대신 "변화 없음" |
| 안정 대기 | 일정 간격으로 캡처해서 변화가 멈출 때까지 기다린다 | 간격 200ms, 최대 3초 |

- **네이티브 의존성을 쓰지 않는다.** 브라우저 스크린샷은 Electron 내장 `nativeImage` 로 축소와 압축을
  한다. sharp(libvips 네이티브)는 OS 전체 화면 캡처(§12)에서 성능이 부족할 때만 추가한다.
- 변화 감지는 축소 이미지를 비교하는 작은 함수로 직접 만든다. 정밀한 비교가 필요하면 pixelmatch(ISC)를
  추가한다.
- 전달: MCP image content (base64 JPEG).
- 언제 보내는가: 기본은 스냅샷(텍스트)이다. 스크린샷은 모델이 요청할 때와, 캔버스·차트처럼 접근성 트리에
  내용이 없을 때만 쓴다.
- 보존: 단계별 스크린샷은 실행 기록이 켜져 있을 때만 `$AKRA_HOME` 에 저장한다. 회사 화면이 들어 있을 수
  있으므로 저장소에 넣지 않는다.

## 8. 사용자와의 공존

- 조작 표시: 에이전트가 조작하는 동안 패널에 표시줄과 마지막 행동을 보여 준다.
- 넘겨받기: 사용자가 패널에서 마우스나 키보드를 쓰면 에이전트 조작을 멈춘다. "재개" 를 누르면 다시 맡긴다.
  멈춘 동안의 도구 호출은 "사용자가 조작 중" 을 돌려준다.
- 사람에게 넘기기: 로그인, OTP, 결제, 개인 확인 화면은 `browser_handoff` 로 사용자에게 넘긴다. 자격 증명을
  모델에게 주지 않는다.
- 조작 대상: 에이전트는 R3 의 브라우저 보기만 조작한다. 앱 화면, 다른 창, OS 는 조작하지 않는다.
- 이동 제한: R3 의 규칙(http(s), about:blank)을 유지한다. 프로필별 허용 도메인 목록을 둘 수 있다 (선택).

## 9. 보안

- 로컬 IPC 는 P-04 와 같은 규칙을 쓴다: 소켓 0600, 디렉터리 0700, Windows 는 named pipe 의 접근 제어.
- MCP 서버는 main 에 접속할 때 토큰을 제시한다. 토큰은 Akra 가 MCP 서버 실행 설정의 환경 변수로 넘긴다.
- 페이지 내용은 회사 데이터일 수 있다. 개인 Codex 계정으로 보내도 되는지 회사 정책을 확인한다 (D-14, R4).
- 페이지의 글은 데이터이다. 페이지가 다른 도메인으로 이동시키거나 새 창을 열면 결과에 분명히 표시한다.
- 쓰지 않는 기능: CAPTCHA 풀기, 지문 위장, 사용자 Chrome 조작, 자격 증명 자동 입력.

## 10. 단계

| 단계 | 내용 | 완료 기준 | 규모 |
| --- | --- | --- | --- |
| B1 연결 | BrowserAgentService 골격, `webContents.debugger` 연결, 로컬 IPC, `akra-browser-mcp`, codex 등록 | `mcpServerStatus/list` 에 도구가 보인다 (턴 없음). `browser_status` 가 동작한다 | 3~4일 |
| B2 스냅샷과 행동 | 스냅샷, ref, diff, navigate·click·type·press·select·scroll·wait | 시험 페이지에서 도구를 직접 호출하는 화면 시험 통과 | 약 1주 |
| B3 스크린샷 | 파이프라인, 변화 감지, 안정 대기 | 크기·압축·변화 감지 단위 시험, 시험 페이지 확인 | 3~4일 |
| B4 QA 보조와 공존 | 콘솔, 네트워크 실패, 대화상자 정책, 넘기기, 조작 표시, 넘겨받기 | 화면 시험 통과 | 약 1주 |
| B5 측정 | 오프라인 토큰 측정, 허락받은 실측 | 결과를 이 문서에 기록 | 2~3일 |

## 11. 시험 방법 (D-13)

- B1~B4 는 MCP 클라이언트로 도구를 직접 호출해서 시험한다. 모델을 호출하지 않는다.
- 로컬 시험 서버의 고정 페이지를 쓴다 (R3 화면 시험과 같은 방식): 폼, 긴 목록, 대화상자, 지연 로딩, 오버레이,
  캔버스, 실패하는 요청.
- B5 의 실측은 사용자 허락을 받고 한다. 비교 대상은 공식 Playwright MCP 서버이다. 항목은 스냅샷 토큰, 단계 수,
  성공률, 걸린 시간이다.

## 12. 범위 밖

- **OS 전체 화면 조작 (computer use):** 별도 계획으로 다룬다. 스크린샷 파이프라인은 재사용한다. macOS 의 화면
  기록·손쉬운 사용 권한, Windows 의 입력 주입, sharp 가 필요할 수 있다.
- 헤드리스 대량 수집 (Lightpanda 등). 화면이 없어 QA 시험에 맞지 않는다.
- Chrome 쿠키 가져오기 (R3 의 미결).

## 13. 미결

| 질문 | 기본안 |
| --- | --- |
| MCP 를 스레드별로 등록하는가, 전역으로 등록하는가 | 스레드별. B1 에서 동작을 확인하고 정한다 |
| `webContents.debugger` 와 개발자 도구를 동시에 쓸 수 있는가 | B1 에서 확인한다. 충돌하면 에이전트 조작 중에는 개발자 도구를 막는다 |
| 위험한 행동(폼 제출, 삭제)을 사용자에게 확인받는가 | 처음에는 확인하지 않는다. 처음 쓰는 도메인에서만 확인하는 안을 B4 에서 검토한다 |
| iframe 과 다른 출처 프레임(OOPIF)을 어디까지 지원하는가 | B2 는 같은 출처 프레임까지. 나머지는 필요할 때 |
| 한 스레드가 여러 탭을 쓰는가 | 처음에는 보기 하나. 새 창은 R3 처럼 같은 보기에서 연다 |
