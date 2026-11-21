# 29-D3. 위임 실행: ChatGPT 채팅 백엔드

상태: **Proposed**. 현재 제품 동작을 설명하지 않는다. · 기준일: 2026-11-21 · 기준 커밋: `33a12b72` (`origin/prerelease`),
akra-tunnel `7cd0de6` · 상위: [plan 29 위임 실행](29-akra-delegated-execution.md) (§8) ·
선행: [D1](29-d1-contract-and-ledger.md)

## 목표

위임 작업을 ChatGPT 채팅으로 보낸다. 보내기는 Akra Desktop 의 앱 안 브라우저가 한다. 전달, 진행, 비정상 중단, 결과는
plan 29 §6 의 공통 값으로 나온다.

## 지금

- Akra 는 ChatGPT 대화를 읽기만 한다 (`ChatsPort`, `akra/chats/list`, `akra/chats/read`, `ChatDock.tsx`). 이 기능은
  dots 클라우드 플래그로 켜진다.
- 대화 조회에는 계정 단위의 요청 한도가 있다. 2026-11-20 의 시험에서 첫 요청부터 HTTP 429 가 왔다.
- 앱 안 브라우저는 창마다 보기 하나이고 프로필별 세션을 쓴다 (`BrowserPanelService`). 에이전트는
  `BrowserAutomationPort` 로 그 보기를 조작한다.
- akra-tunnel 은 Electron 내장 브라우저로 ChatGPT 페이지를 관찰하고 보낸다 (`continuation-browser-host.mjs`,
  `continuation-dom.mjs`). 이 코드는 Akra 에 없다.

## 서브 이슈

| ID | 결과 하나 | 바뀌는 곳 | 선행 |
| --- | --- | --- | --- |
| D3-1 | 구동기의 보기와 로그인 상태: 전용 프로필의 보기, 켜는 설정과 경고, `status` | `desktop/src/main/` | - |
| D3-2 | 호출 길: Rust 의 `ChatDriverPort` 와 Desktop 구동기를 잇는다. Desktop 이 없으면 `unavailable` 이다 | `src/adapter/inbound/gateway/`, `src/adapter/outbound/`, `desktop/src/main/` | D1-3, D3-1 |
| D3-3 | 보내기와 전달 확인: 세션 열기, 지시 봉투와 표식, 전달 단계, `uncertain` | 구동기, `chatgpt_chat` adapter | D3-2 |
| D3-4 | 관찰과 중단 복구: DOM 을 단계로 옮기기, `reload`, `resume`, 한도와 챌린지의 분류 | 구동기, adapter | D3-3 |
| D3-5 | 결과 읽기: 표식 뒤의 마지막 답, 결과 모양의 확인 | 구동기, adapter | D3-3 |
| D3-6 | `ChatsPort` 정리: 자기 설정, 요청 예산, 메시지 ID 와 끝남 여부, `cloud` 근거 | `chats_port.rs`, `cloud_chats/`, `gateway/chats.rs` | D1-3 |
| D3-7 | tunnel: `TunnelStatusPort`, 앱 선택, `local_tools` 능력, `tunnel` 근거 | `src/adapter/outbound/`, 구동기 | D3-4, D4-1 |

## 계약

- 구동기의 호출은 plan 29 §8.2 의 여섯 개이다. 구동기는 재시도와 재개를 스스로 하지 않는다.
- 구동기는 Akra 가 연 대화와 사용자가 지정한 대화만 다룬다. 보내기 직전에 대상 주소를 다시 확인한다. 다르면 보내지
  않는다 (akra-tunnel 의 `CHAT_TARGET_CHANGED`).
- 표식은 `[AKRA_JOB:<id>:<nonce>]` 이다. 전달 `observed` 는 대화에서 이 표식을 본 것이다.
- 전달 단계는 구동기가 본 사실만 말한다: 입력했다, 보내기를 눌렀다, 메시지가 보인다, 진행이 보인다.
- 보내기 호출이 응답 없이 끝나면 전달은 `uncertain` 이다. 조정기는 `observe` 로 표식을 찾는다. 찾지 못해도 자동으로
  다시 보내지 않는다.
- 페이지의 "다시 시도" 버튼은 누르지 않는다. 연결 끊김은 새로 고침만 한다.
- 재개는 그 대화에 이어서 보내는 새 메시지이다. 조정기가 고른다. 한도는 `max_resumes` 이다 (기본 2).
- 사용자가 구동기 보기에 입력하면 그 대화의 자동 행동을 멈춘다 (R9 §8 과 같다).
- `cloud` 근거는 끝을 확인할 때만 읽는다. 429 를 받으면 `retry_at` 까지 읽지 않는다.
- DOM 을 읽는 코드는 한 모듈이다. 모르는 화면은 `unknown` 이다. 추측하지 않는다.
- 로그인, 챌린지, 승인 화면은 `blocked` 이다. 주의 항목이 사람에게 그 보기를 열어 준다.

## 완료 기준

- [ ] fixture 페이지에서 `working`, `complete`, `stopped`, `failed`, `disconnected`, `blocked`, `unknown` 이 나온다.
- [ ] 보내기의 응답을 잃어도 메시지가 둘이 되지 않는다.
- [ ] 대상 대화가 바뀌면 보내기가 0회이다.
- [ ] Desktop 을 끄면 백엔드 건강이 `unavailable` 이 된다. 진행 중인 시도는 `unreachable` 이고, Desktop 이 돌아오면
  관찰이 이어진다.
- [ ] 로그아웃 상태에서 보내기가 0회이고 주의 항목이 하나 열린다.
- [ ] 결과는 표식 뒤의 답만 읽는다. 앞의 턴의 답을 결과로 읽지 않는다.
- [ ] 조회 한도(429) 중에 `cloud` 읽기가 0회이다. Desktop 의 목록 화면은 마지막 성공 값을 계속 보인다.

## 검증

- `command`: 구동기의 vitest (fixture 페이지), 가짜 Gateway 의 e2e (숨긴 창), Rust adapter 의 가짜 구동기 시험.
- 실제 계정: 서브 이슈마다 사용자가 허락한 한 번. 먼저 chatgpt.com 의 대화 목록이 정상인지 사용자에게 묻는다.
- 실제 계정으로 반복 시험하지 않는다.

## 제외

- 파일 올리기와 이미지. 모델 고르기. 대화 지우기 (기본 꺼짐, 뒤 조각).
- akra-tunnel 의 코드 변경. tunnel 의 시작과 중지.
- 비공개 보내기 API. 지문 위장. 챌린지 풀기.

## 미결정

| ID | 질문 | 기본값 |
| --- | --- | --- |
| Q-D3-1 | 호출 길은 Gateway 의 되묻기 (`akra/resourceVersion/current` 의 방식) 인가, `desktop_browser_bridge` 인가 | Gateway 의 되묻기. D3-2 에서 두 길을 읽고 정한다 |
| Q-D3-2 | 구동기 보기를 어디에 두는가 | 평소에는 숨긴 창 안에 둔다. 보기 자체는 보이는 상태이다 (akra-tunnel 의 방식. `setVisible(false)` 는 쓰지 않는다). 사람이 필요할 때 오른쪽 패널로 옮긴다 |
| Q-D3-3 | 구동기의 프로필을 브라우저 패널의 프로필과 나누는가 | 나눈다. 전용 프로필 하나이다 |
| Q-D3-4 | 창이 여럿일 때 어느 창의 구동기가 일하는가 | 주 창 하나. 머신 전체 lease 가 하나만 허락한다 |
| Q-D3-5 | 임시 대화를 여는가 | D3-3 에서 동작을 확인한 뒤에 정한다 (plan 29 Q-DE5) |
| Q-D3-6 | akra-tunnel 의 자동 이어가기에서 Akra 의 대화를 빼는 방법 | 정하지 않았다. 표식으로 가리는 안을 본다 (plan 29 Q-DE12) |
| Q-D3-7 | akra-tunnel 의 DOM 코드를 복사하는가, 공유 패키지로 두는가 | 복사한다. 두 저장소의 출시 주기가 다르다 |
