# 29-D3. 위임 실행: ChatGPT 채팅 백엔드

상태: **Proposed**. 현재 제품 동작을 설명하지 않는다. · 기준일: 2026-11-21 · 기준 커밋: `33a12b72` (`origin/prerelease`),
akra-tunnel `7cd0de6` · 상위: [plan 29 위임 실행](29-akra-delegated-execution.md) (§8) ·
선행: [D1](29-d1-contract-and-ledger.md), [D7](29-d7-akra-tunnel-core.md)

## 목표

위임 작업을 ChatGPT 채팅으로 보낸다. 보내기는 Akra Desktop 의 앱 안 브라우저가 한다. Akra Desktop 은 akra-tunnel-core 를
import 하고 자기 frontend (구동기) 를 만든다. 전달, 진행, 비정상 중단, 결과는 plan 29 §6 의 공통 값으로 나온다.

## 지금

- Akra 는 ChatGPT 대화를 읽기만 한다 (`ChatsPort`, `akra/chats/list`, `akra/chats/read`, `ChatDock.tsx`). 이 기능은
  dots 클라우드 플래그로 켜진다.
- 대화 조회에는 계정 단위의 요청 한도가 있다. 2026-11-20 의 시험에서 첫 요청부터 HTTP 429 가 왔다.
- 앱 안 브라우저는 창마다 보기 하나이고 프로필별 세션을 쓴다 (`BrowserPanelService`).
- akra-tunnel 은 자기 launcher 안에서만 ChatGPT 페이지를 관찰하고 보낸다. 라이브러리로 쓸 수 없다. D7 이 그것을 나눈다.

## 서브 이슈

| ID | 결과 하나 | 바뀌는 곳 | 선행 |
| --- | --- | --- | --- |
| D3-1 | 구동기: akra-tunnel-core 의 의존성, 전용 partition 의 보기와 숨긴 창, 페이지 handle, host 임대, 켜는 설정과 경고, 로그인 상태 | `desktop/src/main/`, `desktop/package.json` | D7-3, D7-4, D7-6 |
| D3-2 | 호출 길: Rust 의 `ChatDriverPort` 와 구동기를 잇는다. Desktop 이 없으면 `unavailable` 이다 | `src/adapter/inbound/gateway/`, `src/adapter/outbound/`, `desktop/src/main/` | D1-3, D3-1 |
| D3-3 | `chatgpt_chat` adapter: 작업 보내기, akra-tunnel-core 의 상태를 공통 값으로 옮기기, 결과 읽기와 모양의 확인 | `src/adapter/outbound/` | D3-2, D7-5 |
| D3-4 | `ChatsPort` 정리: 자기 설정, 요청 예산, 메시지 ID 와 끝남 여부, `cloud` 근거 | `chats_port.rs`, `cloud_chats/`, `gateway/chats.rs` | D1-3 |
| D3-5 | tunnel: 구동기 `status` 의 tunnel 상태, 앱 선택, `local_tools` 능력, `tunnel` 근거 | `src/adapter/outbound/`, 구동기 | D3-3, D4-1 |

## 계약

- `ChatDriverPort` 의 호출은 plan 29 §8.2 의 다섯 개이다.
- 구동기는 akra-tunnel-core 의 호출을 감싼다. DOM 을 직접 읽지 않는다. 복구 규칙을 가지지 않는다.
- adapter 는 akra-tunnel-core 의 값을 공통 값으로 옮긴다.

| akra-tunnel-core 의 값 | plan 29 의 값 |
| --- | --- |
| 관찰 `running` | 단계 `working` |
| 관찰 `complete`, `stopped`, `failed`, `disconnected`, `blocked`, `unknown` | 같은 이름의 단계 |
| 관찰 `load_failed`, 관찰하지 못함 | 단계 `unreachable` |
| 보내기 `sending` | 전달 `sending` |
| 보내기 `host_acknowledged`, `api_returned` | 전달 `accepted` |
| 보내기 `message_observed`, `progress_observed` | 전달 `observed` |
| 보내기 `send_uncertain` | 전달 `uncertain` |
| 보내기 `not_sent` | 전달 `not_sent` |

- 세션의 복구(새로 고침, 재개)는 akra-tunnel-core 가 한다. adapter 는 `recovery` 를 `none`, `in_progress`, `exhausted` 로 알린다.
- 같은 작업 ID 의 `submit` 은 한 번만 보낸다. 응답을 잃으면 전달은 `uncertain` 이다. kernel 은 `observe` 로 확인한다.
  자동으로 다시 보내지 않는다.
- 구동기는 Akra 가 연 대화와 사용자가 지정한 대화만 다룬다. akra-tunnel-core 가 보내기 직전에 대상 주소를 확인한다.
- 보기는 숨긴 창 안에 둔다. 보기 자체는 보이는 상태이다. `setVisible(false)` 를 쓰지 않는다 (akra-tunnel 의 기록).
- 사용자가 구동기 보기에 입력하면 그 대화의 자동 행동을 멈춘다 (R9 §8 과 같다).
- `cloud` 근거는 끝을 확인할 때만 읽는다. 429 를 받으면 `retry_at` 까지 읽지 않는다.
- 로그인, 챌린지, 승인 화면은 `blocked` 이다. 주의 항목이 사람에게 그 보기를 열어 준다.
- akra-tunnel-core 의 버전은 정확한 값으로 고정한다.

## 완료 기준

- [ ] 가짜 akra-tunnel-core 로 모든 단계와 전달 값이 공통 값으로 나온다.
- [ ] 보내기의 응답을 잃어도 메시지가 둘이 되지 않는다.
- [ ] Desktop 을 끄면 백엔드 건강이 `unavailable` 이 된다. 진행 중인 시도는 `unreachable` 이고, Desktop 이 돌아오면
  관찰이 이어진다.
- [ ] 다른 frontend 가 host 를 쥐고 있으면 건강은 `unavailable` (`chat_host_busy`) 이고 보내기가 0회이다.
- [ ] 로그아웃 상태에서 보내기가 0회이고 주의 항목이 하나 열린다.
- [ ] akra-tunnel-core 의 복구가 `exhausted` 가 되기 전에 kernel 의 대체가 0회이다.
- [ ] 조회 한도(429) 중에 `cloud` 읽기가 0회이다. Desktop 의 목록 화면은 마지막 성공 값을 계속 보인다.
- [ ] Akra Desktop 의 설치본에 `sharp` 와 `playwright-core` 가 없다.

## 검증

- `command`: 구동기의 vitest (가짜 akra-tunnel-core), 가짜 Gateway 의 e2e (숨긴 창), Rust adapter 의 가짜 구동기 시험.
- 실제 계정: 서브 이슈마다 사용자가 허락한 한 번. 먼저 chatgpt.com 의 대화 목록이 정상인지 사용자에게 묻는다.
- 실제 계정으로 반복 시험하지 않는다.

## 제외

- DOM 을 읽는 코드, 입력 순서, 복구 규칙. D7 의 일이다.
- 파일 올리기와 이미지. 모델 고르기. 대화 지우기 (기본 꺼짐, 뒤 조각).
- tunnel 의 시작과 중지 (plan 29 Q-DE16).
- 비공개 보내기 API. 지문 위장. 챌린지 풀기.

## 미결정

| ID | 질문 | 기본값 |
| --- | --- | --- |
| Q-D3-1 | 호출 길은 Gateway 의 되묻기 (`akra/resourceVersion/current` 의 방식) 인가, `desktop_browser_bridge` 인가 | Gateway 의 되묻기. D3-2 에서 두 길을 읽고 정한다 |
| Q-D3-2 | 구동기 보기를 어디에 두는가 | 평소에는 숨긴 창 안에 둔다. 사람이 필요할 때 오른쪽 패널로 옮긴다 |
| Q-D3-3 | 구동기의 partition 을 브라우저 패널의 프로필과 나누는가 | 나눈다. 전용 partition 하나이다 |
| Q-D3-4 | 창이 여럿일 때 어느 창의 구동기가 일하는가 | 구동기는 창이 아니라 main 에 하나이다 |
| Q-D3-5 | 임시 대화를 여는가 | D7-5 에서 동작을 확인한 뒤에 정한다 (plan 29 Q-DE5) |
| Q-D3-6 | akra-tunnel-core 의 home 을 akra-tunnel launcher 와 같이 쓰는가 | 같이 쓴다. host 임대가 하나만 허락한다. `tunnel` 근거는 같은 home 에서만 온다 |
