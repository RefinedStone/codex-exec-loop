# 참고: 공식 Codex 데스크톱 앱의 구조

상태: 관찰 기록 · 관찰일: 2026-11-05

## 범위와 원칙

- 대상: macOS 의 ChatGPT 데스크톱 앱 `26.1029.61003`. 2026-07 에 Codex 앱이 이 앱에 통합되었다.
  Chat, Work, Codex 모드가 한 앱에 있다.
- 방법: 로컬 설치본의 번들 메타데이터와 JavaScript 번들을 정적으로 읽었다. 앱 코드는 실행하지 않았다.
- 이 앱은 **비공개 소프트웨어**이다. 이 문서는 구조 수준의 관찰만 기록한다. 코드를 복사하지 않는다.
- 클라우드 에이전트(dots) 관련 내부 구조는 [reference/dots-integration.md](dots-integration.md) 에 따로 기록한다
  (D-19 반전 전에는 D-03 에 따라 기록하지 않았다).

## 1. 구성

| 항목 | 관찰 |
| --- | --- |
| 셸 | Electron (프레임워크 이름을 바꿔서 포함) |
| 크기 | 앱 전체 약 1.8GB. UI 패키지(`app.asar`) 약 721MB, 그중 대부분이 웹 화면이다 |
| 엔진 | 공개 Codex CLI 를 번들로 포함한다 (`codex-cli 0.166.0-alpha.6`) |
| 엔진 실행 | 로컬: `codex -c features.code_mode_host=true app-server ...` (stdio). 원격: SSH 너머에서 `codex app-server --listen unix://` |
| Windows 관찰 | `26.1029.7803.0` (2026-11-05). 로컬 엔진은 stdio 자식 프로세스로 실행한다. 관리형 데몬 소켓은 쓰지 않는다 |
| 그 밖의 번들 | computer-use 런타임, macOS 네이티브 모듈, LaTeX 컴파일러, ripgrep |
| 코드 상태 | 압축(minify)만 했다. 난독화는 없다. 소스맵은 포함하지 않았다 |
| 규모 | 정확한 줄 수는 알 수 없다. 웹 화면에 ChatGPT 웹 기능 전체가 들어 있어서 Codex 부분만 분리할 수 없다 |

## 2. 프로세스 분리

```text
renderer (웹 화면)   React UI, 스레드 상태, app-server JSON-RPC 요청을 직접 만든다
    | preload 가 노출한 브리지 객체 하나 (함수 약 20개)
    | IPC: 메시지 버스 채널 한 쌍 + 소수의 보조 채널. 큰 메시지는 조각으로 나눠 보낸다
main (Node)          메시지 type 으로 분기한다
    - app-server 요청: 대상 호스트(로컬/원격)를 골라 그대로 전달한다
    - git worker, 터미널(pty), 창 관리, 외부 HTTP 대행
    v
codex app-server (번들 CLI, 공개 엔진)
```

- renderer 는 `sandbox: true`, `contextIsolation: true`, `nodeIntegration: false` 이다.
- app-server 클라이언트 로직(무엇을 요청할지)은 **renderer** 에 있다. main 은 대부분 중계한다.
- main 의 중계 단계에는 요청마다 "그대로 전달 / 수정해서 전달 / 로컬에서 응답" 을 고르는 필터가 있다.
  이 계획의 interceptor 와 같은 발상이다 (D-11).

## 3. app-server 사용

- 공개 프로토콜의 요청·알림 이름 약 271개 중 약 255개를 쓴다. 앱 기능의 대부분이 공개 엔진 위에 있다.
- 2026-11-04 공개 프로토콜에 없는 메서드 이름이 20개 안팎 있다. 대부분 비공개 서버 기능과 연결된다.
  일부는 이전 공개 버전에 있던 메서드이다 (예: codex 0.149.1 은 `thread/rollback` 을 받는다). 숫자는
  문자열 비교로 센 값이다. 이 계획에서는 공개 프로토콜에 있는 메서드만 쓴다.
- 설정 기능 플래그(`features.*`)를 약 40개 쓴다. 예: `multi_agent`, `browser_use`, `computer_use`,
  `memories`, `hooks`.

## 4. 여러 창 처리

- 앱은 app-server 접속을 창들이 공유한다. 그래서 한 창이 스레드를 소유하고, 다른 창은 소유 창을
  따라가는 구조를 따로 구현했다. 이를 위한 메시지 종류가 17개 있다.
- 이 계획은 창마다 접속을 따로 연다 (P-03). app-server 가 구독 단위로 알림을 나누므로 이 구조가
  필요 없다.

## 5. 이 계획에 반영한 것

| 관찰 | 반영 |
| --- | --- |
| renderer 권한 제거 + preload 브리지 | 그대로 따른다 ([04](../04-target-architecture.md) §8) |
| 메시지 버스 하나 | main 과 renderer 사이에 같은 방식을 쓴다 |
| main 의 요청 필터 | Gateway 의 interceptor 로 옮긴다. 앱이 아니라 데몬에 둔다 |
| 로컬과 원격을 같은 프로토콜로 처리 | Gateway 를 거치므로 앱은 대상을 알 필요가 없다 |
| 창들이 접속을 공유 | 따르지 않는다. 창마다 접속을 연다 |
| 클라이언트 로직을 renderer 에 둠 | 따른다. Gateway 는 해석하지 않는다 |
