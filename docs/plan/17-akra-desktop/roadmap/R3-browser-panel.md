# R3. 브라우저 패널

상태: 완료 (쿠키 가져오기는 미룸) · 기준일: 2026-11-06 · 선행: P3 · 규모: 1~2주

## 목표

앱 안 브라우저로 사내 QA 환경 같은 웹 서비스를 시험한다. 계정별로 로그인 상태를 나눠 유지한다.

## 범위

- 브라우저 보기: Electron `WebContentsView` 를 앱 창 안에 배치한다.
- 프로필: 프로필마다 별도 세션 저장소를 쓴다 (`session.fromPartition('persist:<프로필>')`). 쿠키,
  localStorage, 캐시가 서로 섞이지 않는다.
- 프리셋: 자주 쓰는 사이트와 프로필의 조합을 저장한다 (예: QA 서버 + 관리자 프로필, QA 서버 + 학습자
  프로필).
- 쿠키 가져오기: Chrome 의 쿠키를 선택한 도메인만 프로필로 가져온다.

## 설계 메모

- 브라우저 보기는 앱 화면 renderer 와 다른 세션을 쓴다. 앱 화면의 보안 설정을 약하게 만들지 않는다.
- 쿠키 가져오기:
  - macOS Chrome 쿠키는 키체인의 "Chrome Safe Storage" 키로 암호화되어 있다. 처음 읽을 때 키체인 접근
    승인이 필요하다.
  - Google 쿠키는 가져오지 않는다. Google 은 내장 브라우저 로그인을 차단한다.
  - 가져온 도메인의 쿠키만 교체한다. 다른 사이트의 로그인은 유지한다.
  - Orca(MIT)의 구현(`src/main/browser/cookies/`)을 참고한다. 원자성, 동시성, 롤백까지 처리한다.
    차용하면 고지를 유지한다.
- 설치된 Chrome 자체를 앱 안에 넣을 수는 없다 ([alternatives.md](../reference/alternatives.md) §6).
- 나중에 에이전트가 브라우저를 조작하게 하려면 `webContents.debugger` 로 CDP 를 쓴다. 이 기능은
  [R9](R9-agent-browser-and-screenshots.md) 에서 다룬다.

## 미결

- 사내 사이트의 주소와 계정 정보는 저장소에 넣지 않는다. 사용자 설정(`userData`)과 OS 키체인에만 둔다.
- 쿠키 가져오기를 R3 에 포함할지, 프로필별 직접 로그인만 먼저 할지 정한다.

## 진행 기록

### 2026-11-06: 브라우저 패널 (PR 1)

- 결정: 쿠키 가져오기는 이번에 넣지 않는다. 프로필별 직접 로그인과 프리셋만 먼저 한다. Windows Chrome 127 부터
  쿠키가 app-bound 방식으로 암호화되어 다른 앱이 풀기 어렵고, macOS 는 키체인 승인이 필요하다. 실사용에서 필요해지면
  다시 정한다 (미결 2 정리).
- main: `desktop/src/main/browserPanel.ts` 가 창마다 `WebContentsView` 를 하나 둔다. 세션은
  `persist:akra-browser-<프로필 id>` 이다. 앱 화면과 세션, preload 를 공유하지 않는다. sandbox, contextIsolation,
  권한 요청 거절, http(s) 와 about:blank 밖으로의 이동 차단, 새 창은 같은 보기에서 연다.
- 설정: 프로필과 프리셋은 `userData/browser-settings.json` 에만 둔다. 사내 주소를 저장소에 넣지 않는다 (미결 1).
  읽을 때마다 검증하고, 임시 파일에 쓴 뒤 바꿔 넣는다.
- 화면: 상단 "브라우저" 버튼으로 오른쪽 패널을 연다. 프로필 선택과 추가, 프리셋 선택과 저장, 뒤로/앞으로/새로
  고침, 주소창. 페이지는 main 이 패널의 빈 자리 위에 그린다. renderer 가 자리의 위치와 크기를 바뀔 때마다 알린다.
  그래서 앱의 드롭다운은 페이지 위로 겹쳐 보이지 않는다.
- 시험: 주소 정리, 설정 검증, 저장 단위 시험. 화면 시험(`test/e2e/browser.mjs`, 창 숨김, 임시 userData)은 로컬
  시험 서버로 프로필별 쿠키 분리, 프리셋, 막힌 주소, 설정 저장을 확인한다.
- `AKRA_DESKTOP_USER_DATA` 로 시험의 userData 를 임시 폴더로 옮긴다.
