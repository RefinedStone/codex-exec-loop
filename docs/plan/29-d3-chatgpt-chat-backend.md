# 29-D3. 위임 실행: ChatGPT 채팅 백엔드

상태: **Proposed**. 현재 제품 동작을 설명하지 않는다. · 기준일: 2026-11-21 · 기준 커밋: `33a12b72` (`origin/prerelease`),
akra-tunnel `7cd0de6` · 상위: [plan 29 위임 실행](29-akra-delegated-execution.md) (§8) ·
선행: [D1](29-d1-contract-and-ledger.md), [D7](29-d7-akra-tunnel-core.md)
([akra-tunnel-core #1](https://github.com/RefinedStone/akra-tunnel-core/issues/1))

## 목표

위임 작업을 ChatGPT 채팅으로 보낸다. 보내기는 Akra Desktop 의 앱 안 브라우저가 한다. Akra Desktop 은 akra-tunnel-core 를
import 하고 자기 frontend (tunnel frontend) 를 만든다. 전달, 진행, 비정상 중단, 결과는 plan 29 §6 의 공통 값으로 나온다.

## 지금

- Akra 는 ChatGPT 대화를 읽기만 한다 (`ChatsPort`, `akra/chats/list`, `akra/chats/read`, `ChatDock.tsx`). 이 기능은
  dots 클라우드 플래그로 켜진다. Codex 의 토큰으로 읽는다.
- 대화 조회에는 계정 단위의 요청 한도가 있다. 2026-11-20 의 시험에서 첫 요청부터 HTTP 429 가 왔다.
- 앱 안 브라우저는 창마다 보기 하나이고 프로필별 세션을 쓴다 (`BrowserPanelService`).
- akra-tunnel 은 자기 launcher 안에서만 ChatGPT 페이지를 관찰하고 보낸다. 라이브러리로 쓸 수 없다. D7 이 그것을 나눈다.
- `desktop/` 에 tunnel 의 코드는 없다.

## 서브 이슈

| ID | 결과 하나 | 바뀌는 곳 | 선행 |
| --- | --- | --- | --- |
| D3-1 | tunnel frontend: akra-tunnel-core 의 의존성, 전용 partition 의 보기와 숨긴 창, `PagePort` 의 구현, host 임대, 작업의 줄, 켜는 설정과 경고, 로그인 상태 | `desktop/src/main/`, `desktop/package.json` | D7-3, D7-4, D7-6 (akra-tunnel-core #5, #6, #8) |
| D3-2 | 호출 길: Rust 의 `ChatSessionPort` 와 tunnel frontend 를 잇는다. Desktop 이 없으면 `unavailable` 이다 | `src/adapter/inbound/gateway/`, `src/adapter/outbound/`, `desktop/src/main/` | D1-3, D3-1 |
| D3-3 | `chatgpt_chat` adapter: 작업 보내기, akra-tunnel-core 의 상태를 공통 값으로 옮기기, 결과 읽기와 모양의 확인 | `src/adapter/outbound/` | D3-2, D7-5 (akra-tunnel-core #7) |
| D3-4 | ChatGPT 화면의 정리: 설정을 dots 플래그에서 떼기, 읽기 화면에서 tunnel frontend 의 보기로 열기, 계정이 다를 때의 표시 | `gateway/chats.rs`, `desktop/src/renderer/` | D3-1 |
| D3-5 | tunnel: `status` 의 tunnel 상태, 앱 선택, `local_tools` 능력, `tunnel` 근거 | `src/adapter/outbound/`, tunnel frontend | D3-3, D4-1 |

## 계약

- `ChatSessionPort` 의 호출은 plan 29 §8.2 의 다섯 개이다. 이 port 는 akra-tunnel-core 의 `ChatSessionService` 를 프로세스
  너머로 부른다.
- tunnel frontend 는 akra-tunnel-core 의 호출을 감싼다. DOM 을 직접 읽지 않는다. 복구 규칙을 가지지 않는다. 표식을 만들지
  않는다.
- tunnel frontend 가 가지는 것은 넷이다: 보기, `PagePort` 의 구현, 작업의 줄, 화면.
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
| 보내기 `not_sent`, 줄에서 기다림 | 전달 `not_sent` |

- 세션의 복구 (새로 고침, 재개) 는 akra-tunnel-core 가 한다. adapter 는 `recovery` 를 `none`, `in_progress`, `exhausted` 로
  알린다.
- 같은 작업 ID 의 `submit` 은 한 번만 보낸다. 응답을 잃으면 전달은 `uncertain` 이다. kernel 은 `observe` 로 확인한다.
  자동으로 다시 보내지 않는다.
- 동시 실행은 tunnel frontend 의 줄이 정한다. kernel 은 lease 를 두지 않는다. 줄에 있는 작업은 `stop` 으로 뺀다.
- `cloud` 근거는 akra-tunnel-core 가 준다. 페이지가 이미 받은 응답을 읽는다. 위임은 `ChatsPort` 를 부르지 않는다.
- 계정. Desktop 의 목록 (Codex 의 토큰) 과 tunnel frontend (웹 로그인) 는 계정이 다를 수 있다. 다르면 `continue` 세션을
  받지 않는다. 이유는 `chat_account_mismatch` 이다. 계정의 값은 Desktop main 을 떠나지 않는다.
- tunnel frontend 는 akra-tunnel 의 home 을 쓴다. `$AKRA_HOME` 밖이다. host 임대를 쥐지 못하면 건강은 `unavailable`
  (`chat_host_busy`) 이다.
- 보기는 숨긴 창 안에 둔다. 보기 자체는 보이는 상태이다. `setVisible(false)` 를 쓰지 않는다 (akra-tunnel 의 기록).
- 사용자가 그 보기에 입력하면 그 대화의 자동 행동을 멈춘다 (R9 §8 과 같다).
- 로그인, 챌린지, 승인 화면은 `blocked` 이다. 주의 항목이 사람에게 그 보기를 열어 준다.
- akra-tunnel-core 의 버전은 정확한 값으로 고정한다.

## 완료 기준

- [ ] 가짜 akra-tunnel-core 로 모든 단계와 전달 값이 공통 값으로 나온다.
- [ ] 보내기의 응답을 잃어도 메시지가 둘이 되지 않는다.
- [ ] 두 저장소가 동시에 작업을 내면 메시지는 하나씩 순서대로 간다.
- [ ] Desktop 을 끄면 백엔드 건강이 `unavailable` 이 된다. 진행 중인 시도는 `unreachable` 이고, Desktop 이 돌아오면
  관찰이 이어진다.
- [ ] 다른 frontend 가 host 를 쥐고 있으면 건강은 `unavailable` (`chat_host_busy`) 이고 보내기가 0회이다.
- [ ] 로그아웃 상태에서 보내기가 0회이고 주의 항목이 하나 열린다.
- [ ] 계정이 다르면 `continue` 세션의 보내기가 0회이다.
- [ ] akra-tunnel-core 의 복구가 `exhausted` 가 되기 전에 kernel 의 대체가 0회이다.
- [ ] 위임 작업이 도는 동안 `ChatsPort` 의 요청이 0회이다.
- [ ] Akra Desktop 의 설치본에 `sharp` 와 `playwright-core` 가 없다.

## 검증

- `command`: tunnel frontend 의 vitest (가짜 akra-tunnel-core), 가짜 Gateway 의 e2e (숨긴 창), Rust adapter 의 가짜
  frontend 시험.
- 실제 계정: 서브 이슈마다 사용자가 허락한 한 번. 먼저 chatgpt.com 의 대화 목록이 정상인지 사용자에게 묻는다.
- 실제 계정으로 반복 시험하지 않는다.

## 제외

- DOM 을 읽는 코드, 입력 순서, 표식, 복구 규칙. D7 의 일이다.
- 파일 올리기와 이미지. 모델 고르기. 대화 지우기 (기본 꺼짐, 뒤 조각).
- tunnel 의 시작과 중지 (plan 29 Q-DE16).
- 비공개 보내기 API. 지문 위장. 챌린지 풀기.

## 미결정

| ID | 질문 | 기본값 |
| --- | --- | --- |
| Q-D3-1 | 호출 길은 Gateway 의 되묻기 (`akra/resourceVersion/current` 의 방식) 인가, `desktop_browser_bridge` 인가 | Gateway 의 되묻기. D3-2 에서 두 길을 읽고 정한다 |
| Q-D3-2 | tunnel frontend 의 보기를 어디에 두는가 | 평소에는 숨긴 창 안에 둔다. 사람이 필요할 때 오른쪽 패널로 옮긴다 |
| Q-D3-3 | 그 partition 을 브라우저 패널의 프로필과 나누는가 | 나눈다. 전용 partition 하나이다 |
| Q-D3-4 | 창이 여럿일 때 어느 창의 것이 일하는가 | tunnel frontend 는 창이 아니라 main 에 하나이다 |
| Q-D3-5 | 임시 대화를 여는가 | D7-5 (akra-tunnel-core #7) 에서 동작을 확인한 뒤에 정한다 (plan 29 Q-DE5) |
| Q-D3-6 | akra-tunnel-core 의 home 을 akra-tunnel launcher 와 같이 쓰는가 | 같이 쓴다. host 임대가 하나만 허락한다. `tunnel` 근거는 같은 home 에서만 온다 |
| Q-D3-7 | 계정이 같은지를 무엇으로 비교하는가 | Desktop main 이 두 계정의 식별자를 비교한다. 결과만 밖으로 낸다 |
