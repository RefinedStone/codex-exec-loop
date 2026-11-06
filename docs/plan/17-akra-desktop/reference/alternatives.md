# 참고: 검토한 대안과 라이선스

상태: 조사 기록 · 조사일: 2026-11-05

각 제품의 현재 상세 비교는 [docs/competitive/](../../../competitive/README.md) 가 기준이다. 이 문서는
이 계획의 선택에 영향을 준 사실만 적는다.

## 1. 라이선스 요약 (코드 차용 기준)

| 대상 | 라이선스 | 이 계획에서의 사용 |
| --- | --- | --- |
| Codex 엔진과 app-server (`openai/codex`) | Apache-2.0 | 실행하고 참고한다. 코드를 차용하면 고지를 유지한다 |
| Orca (`stablyai/orca`) | MIT | 코드를 차용할 수 있다. 고지를 유지한다 |
| herdr, herdr-core | Apache-2.0 | 참고만 한다 |
| doriface | AGPL-3.0 | **코드를 차용하지 않는다.** 차용하면 Akra 전체에 AGPL 의무가 생긴다 |
| OmO 엔진 (oh-my-openagent) | SUL-1.0 | **코드를 차용하지 않는다.** 상업적 재배포가 제한된다 |
| 공식 Codex(ChatGPT) 데스크톱 앱 | 비공개 | 구조만 관찰한다. 코드를 복사하지 않는다 |
| Claude 데스크톱 앱, Claude Code 엔진 | 비공개 | 공식 CLI 실행만 한다 (R7) |

## 2. 공식 앱

### 공식 Codex 데스크톱 앱

- 데스크톱 UI 는 비공개이고 엔진은 공개이다. 구조는 [codex-desktop-app.md](codex-desktop-app.md) 에 있다.
- 대화 경험은 이 계획의 기준이다. 그러나 사용자의 업무 도구와 하네스를 넣을 수 없다.

### Claude 데스크톱 앱 (Code 탭)

- 2026-04 에 병렬 작업 중심으로 재설계되었다 (외부 자료 기준). 세션 여러 개, 세션마다 worktree 자동
  분리, 줄 단위 코멘트가 가능한 diff 리뷰, 예약 작업, 원격 세션을 제공한다.
- 앱과 엔진(Claude Code) 모두 비공개이다. `anthropics/claude-code` 저장소에는 엔진 소스가 없다.
- Agent SDK: Python SDK 는 MIT 이고 Claude Code CLI 를 실행하는 얇은 래퍼이다. TypeScript SDK 저장소에는
  소스가 없다.
- 인증 정책 (조사 기록): 개인 구독의 OAuth 자격 증명을 서드파티 제품과 Agent SDK 에서 쓰는 것을
  2026-02 약관에서 금지했다. Claude 연결(R7)은 공식 CLI 를 그대로 실행하는 방식만 쓴다.

## 3. Orca

| 항목 | 내용 (2026-11-05) |
| --- | --- |
| 저장소 | <https://github.com/stablyai/orca>, MIT |
| 설치본 | 1.4.248 (이 Mac). 경쟁 분석 문서는 1.4.176 기준이다 |
| 기술 | Electron 43, React 19, Vite 8 + electron-vite, TypeScript 7, zustand, xterm.js + node-pty, Monaco, Tailwind 4. 모바일은 React Native + Expo |
| 엔진 연결 | Codex, Claude Code, OpenCode, Pi 의 공식 CLI 를 터미널(pty)에서 그대로 실행한다. 대화를 구조화된 데이터로 받지 않는다 |
| 기능 | worktree 병렬 실행, 모바일 앱, GitHub·Linear·Jira 연동, SSH 원격, 앱 안 브라우저, 예약 실행(`orca automations`) |
| 브라우저 | 프로필마다 저장소(쿠키, localStorage, 캐시)를 분리한다. Chrome·Edge 쿠키를 가져온다. Google 쿠키는 가져오지 않는다 |
| 플러그인 | Host API 1.3: 패널(TSX), 전용 worker, `agent.launch`, `terminal.create/read/sendText`, `secrets.*`, `notifications.show`, `workspace.*` |

검토 결과:

- **Orca 플러그인 경로**는 가장 빠르게 효과를 낸다. 그러나 화면과 구조를 Orca 가 소유한다. D-01 에서
  선택하지 않았다.
- **코드 참고 후보**: 쿠키 가져오기(`src/main/browser/cookies/`), 터미널, 브라우저 프로필. MIT 이므로
  차용할 수 있다 (R2, R3).

## 4. OmO, Senpi, Sisyphus Labs

| 대상 | 내용 |
| --- | --- |
| OmO (oh-my-openagent) | 엔진 코드는 공개(SUL-1.0)이다. 데스크톱 앱은 소스가 없고, 로그인한 사용자에게 바이너리만 배포한다 |
| Senpi | pi(badlogic/pi-mono)의 포크. OmO Native 의 엔진이다. 상세는 [docs/competitive/omo-native/](../../../competitive/omo-native/README.md) |
| doriface | herdr-core 터미널을 웹, PWA, Electron 에서 원격으로 보는 클라이언트 (AGPL-3.0). 호스트 게이트웨이가 메시지를 해석하지 않고 바이트 그대로 전달한다. 이 계획의 Gateway 와 같은 발상이다 |
| omowright | 에이전트용 브라우저 자동화 라이브러리 (MIT, 2026-08 시작). Playwright 모양 API, 접근성 스냅샷 + ref, 변경분 전달, 포트 없는 CDP(`--remote-debugging-pipe`). 실측 README 기준 스냅샷이 원본 HTML 의 약 5~36% 토큰이다. R9 에서 설계만 참고한다 (D-17) |
| dori-superagent | herdr + omo 위의 외부 루프 하네스. TypeScript 약 1,600줄. 리드 세션 하나, 작업마다 별도 세션(lane), 조용한 lane 을 재촉하는 duty loop, 증거가 있어야 통과하는 머지 게이트. 라이선스는 확인하지 않았다. R5 설계 때 구조만 참고한다 |

## 5. Electron, Tauri, 순수 Rust GUI

| 기준 | Electron | Tauri | 순수 Rust GUI (gpui, iced 등) |
| --- | --- | --- | --- |
| 프레임워크 언어 | C++ (Chromium, Node) | Rust | Rust |
| 앱 로직 언어 | TypeScript | Rust | Rust |
| 화면 | HTML, TypeScript | HTML, TypeScript | Rust |
| 웹 엔진 | Chromium 을 앱마다 포함 | OS 웹뷰 (macOS 는 WebKit) | 없음 |
| 앱 안 브라우저 | 가능, Chrome 과 같은 엔진 | 가능, Safari 엔진 | 불가능 |
| Chrome 쿠키 가져오기 | 가능 (Orca 구현 있음) | 형식 변환을 직접 해야 함 | 해당 없음 |
| CDP 조작 | 내장 (`webContents.debugger`) | WebKit 에는 CDP 가 없음 | 해당 없음 |
| 크기, 메모리 | 큼 | 작음 | 작음 |
| 기본 보안 | sandbox, preload 를 직접 설정 | 권한(capability) 단위 기본 제한 | - |
| 참고 자료 | 많음 (공식 Codex 앱, Orca, VS Code) | 보통 | 적음 |

결론: 앱 안 브라우저(사내 QA 시험)와 CDP 요구 때문에 Electron 을 골랐다 (D-01).

## 6. 설치된 Chrome 을 앱 안에 넣을 수 있는가

넣을 수 없다. 이유는 다음과 같다.

- Chrome 은 다른 앱에 들어가는 임베딩 API 를 제공하지 않는다. 앱에 넣을 수 있는 것은 Chromium
  엔진(Electron, CEF)이다.
- 앱 안의 Chromium 은 자기 프로필을 쓴다. Chrome 프로필의 로그인 정보를 공유하지 않는다.
- macOS Chrome 의 쿠키는 키체인의 "Chrome Safe Storage" 키로 암호화되어 있다.
- Chrome 이 실행 중이면 프로필 폴더가 잠겨 있다. 버전이 다른 엔진이 같은 프로필을 쓰면 손상될 수 있다.
- Chrome 136 이후 기본 프로필에서는 원격 디버깅(CDP)이 꺼진다.
- Google 은 앱 내장 브라우저에서의 로그인을 차단한다.

대안: 앱 안 브라우저에 사이트별 프로필을 만들고 한 번 로그인하거나, Chrome 쿠키를 가져온다 (R3).
