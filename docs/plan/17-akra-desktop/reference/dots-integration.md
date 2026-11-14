# reference: dots 통합 설계

상태: Proposed · 기준일: 2026-11-13 · 근거 앱: 공식 Codex 데스크톱 `26.1106.51428`

이 문서는 [D-19](../03-decisions.md#d-19-dots-를-단일-공개-저장소에서-구현-d-03-반전)의 설계 근거이다.
"Akra 에서 dots 를 쓰려면 무엇을 만드나"에 답한다. 비공개 API 는 공개 문서가 없어 공식 앱 역분석으로 얻었다.
자격증명(토큰)·사용자 개인 정보는 이 문서에 넣지 않는다.

## 1. 구조 요약

- dots 는 OpenAI 클라우드에서 실행된다. 공식 앱은 클라이언트일 뿐이다. 로컬 codex 실행 파일에는 dots 전용
  메서드가 없다.
- dot 과의 대화는 REST 메시지 방이다. app-server 스레드는 dot 의 "작업 화면"이다.
- 인증은 ChatGPT 로그인 토큰 하나이다. 로컬 app-server `getAuthStatus {includeToken:true}` 또는 같은 값이 든
  `~/.codex/auth.json` 의 access_token 으로 얻는다. account-id 는 토큰 JWT claim 이다.
- 클라이언트에는 인증서 고정·요청 서명이 없다. 막힐 수 있는 지점은 서버 검사(기기 증명, 클라이언트 신원, 권한)이다.

### 통로 다섯

| 통로 | 주소 | 쓰임 |
| --- | --- | --- |
| app-server JSON-RPC / WebSocket | `wss://codex-cloud-backend.chatgpt.com/` (호스트 ID `durable`) | dot 의 작업 스레드: resume, 턴, 큐, 정지 |
| REST (같은 호스트) | `https://codex-cloud-backend.chatgpt.com/` | 스레드·턴·아이템 조회, 모델 목록. `POST /grpc` 로 JSON gRPC |
| REST (ChatGPT 백엔드) | `/tbo/*`, `/messaging/rooms/*`, `/aeon-messaging/*`, `/cloud-aeons/*` | dot 목록·생성·관리, dot 과의 대화(메시지 방), 채널 |
| 실시간 상태 | `GET /celsius/ws/user` → WS, `POST /messaging/rooms/{id}/live` (SSE) | "작업 중" 표시 |
| 클라우드 컴퓨터 화면 | `https://realtime.chatgpt.com/v1/cua` (WebRTC) | dot 의 클라우드 PC 화면 보기·조작 |

## 2. D0 확인 결과 (2026-11-13, 실행함)

사용자 허락 아래 읽기 전용 1회. 토큰은 출력하지 않았다.

- 연결: `wss://codex-cloud-backend.chatgpt.com/`, 하위 프로토콜 `codex-app-server`, `codex-client.desktop`,
  `openai-bearer.<access_token>`. 헤더 `X-OpenAI-Product-Sku: codex`, `ChatGPT-Account-Id`, `originator: Codex Desktop`,
  `User-Agent: Codex Desktop/<버전> (Mac OS; <arch>)`.
- 결과:
  - WebSocket 핸드셰이크 101 성공(서버는 하위 프로토콜을 되돌려주지 않지만 연결 유지).
  - `initialize` 가 **기기 증명 없이** 성공. 서버가 `attestation/generate` 를 보내지도 않았다.
  - 읽기 전용 `thread/list` 성공(실제 스레드 반환).
- 결론: 서버는 서명 안 된 "Codex Desktop" 신원의 클라이언트를 initialize 에서 받고, 인증 읽기까지 된다.
- 미결: 메시지 방 POST 의 `app_attest_challenge` 강제 여부(쓰기 경로), `/tbo` 의 정확한 기준 호스트,
  증명 생략이 계정 신호로 남는지.

## 3. 연결 계층 (main/Gateway)

| 항목 | 내용 |
| --- | --- |
| WebSocket | `ws`, JSON-RPC 텍스트. permessage-deflate 끔. 핸드셰이크 15초, 10초마다 ping, 60초 pong 없으면 끊음 |
| 토큰 전달 | 하위 프로토콜 `openai-bearer.<token>`. Authorization 헤더는 안 보냄 |
| 압축(선택) | 요청 헤더 `x-codex-app-server-compression: zstd-stream-v1`. 서버가 같은 값을 돌려주면 모든 프레임이 binary zstd (레벨 −7, windowLog 20), 16 MiB 분할, 메시지당 100 MiB |
| 재연결 | 1초→2배→20초. 클라우드는 min(기본,10초)+무작위. 로그인 필요 중엔 재연결 안 함 |
| 401 | 연결 중 401 이면 새 토큰으로 1회 재연결. 계정·사용자 ID 가 같아야 함 |
| initialize | `clientInfo{name/title:"Codex Desktop", version}`, `experimentalApi:true`, `requestAttestation:true`(macOS/Windows) |

### 요청 변환(main 이 클라우드로 보내기 전)

- 로컬에서 답함: `config/read`(메모리 설정), `config/batchWrite`, `permissionProfile/list`(빈 목록).
- 로컬 app-server 로 보냄: `account/read`, `getAuthStatus`, `configRequirements/read`.
- WebSocket 대신 REST: `thread/read`(turns 제외), `model/list`, `collaborationMode/list`, `account/rateLimits/read`,
  `thread/turns/list`, `thread/items/list`. turns·items 는 10초 무응답 시 WebSocket 재시도.
- 고치는 필드: 시작·재개·턴에서 `personality` 삭제, `serviceTier priority→fast`, `threadAccess`/`executorAccess`
  주입, 시작·재개에 `config["features.mcp_app_ui"]=true`. `thread/queue/add → turn/addUserMessage + mode:"queue"`.
  `thread/start` 에 `pluginsMcp:{productSku:"codex"}`. `thread/resume` 에서 `baseInstructions`,
  `developerInstructions`, `pluginsMcp` 삭제. 환경 ID 가 `managed`/`ccarenv_` 면 `deferredEnvironment`.

## 4. 화면 계층 (dot 고유)

- dot 판별: `threadStartKind=="aeon"` 또는 `threadSource=="aeon"`, 스레드 mode `durable`. 종류는 프로필 `aeon_kind`
  (`orbit`=dot, `aeon`=메시지 방 없는 에이전트).
- 작업 스레드: `thread/resume`(dot 이면 `approvalPolicy:"on-request"`, `approvalsReviewer:"auto_review"` 등 주입),
  `turn/addUserMessage`(필드 `clientUserMessageId` 필수, aeon 은 `messageThreadId` 필수, 응답 `admissionOutcome` =
  `started`/`steered`/`duplicate`), `thread/stop`(그 뒤 `needs_resume`), `thread/queue/*`.
- dot 의 반응: 알림이 아니라 도구 호출(`react_to_user_message`, 인자 `{message_id, emoji, label}`)로 온다.
- dot → 로컬 위임: 전용 필드 없이 사용자 메시지 글자 안에 `<codex_delegation>…</codex_delegation>` XML 로 온다.
  Akra 는 이 XML 이 든 평범한 사용자 턴을 받으면 되므로 로컬 쪽 변경이 작다.

### REST: dot 관리·대화

- 관리: `GET /tbo`(목록, cursor), `POST /tbo`(생성), `PATCH/DELETE /tbo/{id}`, `/tbo/{id}/runtime/{pause,resume}`,
  `GET /tbo/{id}/threads`.
- 대화(메시지 방): `GET/POST /messaging/rooms/{room_id}/messages`(POST 는 쓰기 — 허락 필요), 반응
  `.../messages/{id}/reactions`, `/read`, `/responding_heartbeat`, `/elicitation`.
- 실시간: `GET /celsius/ws/user` → WS, `POST /messaging/rooms/{id}/live`(SSE). 이벤트
  `calpico-room-live-state` 에 `threads[]{thread_id, turn_status, computer_status, activity_status}`.

## 5. Akra 매핑 (단일 공개 저장소, D-19)

| 층 | 만들 것 | 위치 |
| --- | --- | --- |
| 토큰 | Gateway `akra/*` 에 토큰 획득(로컬 app-server `getAuthStatus` 또는 auth.json) | Gateway |
| 클라우드 upstream | 두 번째 upstream WS(하위 프로토콜·헤더·ping·재연결·401·요청 변환). 접속마다 `hostId` 로 분기 | Gateway. 얇은 Gateway(D-11)를 넓힘 |
| REST 대행 | `/tbo/*`, `/messaging/*`, 실시간 상태(SSE/WS), 클라우드 호스트 REST·`/grpc` | Gateway `akra/dots/*` 또는 Electron main |
| 화면 | dot 목록, 메시지 방 대화창, 작업 화면, 상태 배지, 반응 표시 | renderer. 대화창 하나, 종류별 정책만 다르게 |
| 클라우드 컴퓨터 | WebRTC 보기·조작 | renderer (R9 와 별도) |
| 위임 받기 | `<codex_delegation>` 사용자 턴을 "dot 이 보냄"으로 표시 | renderer. 로컬 app-server 그대로 |

## 6. 단계와 경계

| 단계 | 내용 | 쓰기? |
| --- | --- | --- |
| D1 | dot 목록, 메시지 방 읽기, 실시간 상태, 반응 표시 | 읽기 (보내기는 허락 뒤) |
| D2 | 작업 화면: resume, `turn/addUserMessage`, 큐, 정지, 요청 변환 | 쓰기 — 허락 |
| D3 | 관리: 생성·이름·멈춤·재시작·지우기, 채널 | 쓰기 — 허락 |
| D4 | 클라우드 컴퓨터 WebRTC 보기·조작 | 쓰기 — 허락 |

- 경계(D-19): 실계정 쓰기는 매번 허락 뒤에. 토큰은 로그·커밋·화면에 안 남긴다. 검증은 턴 없이(빌드,
  initialize, `thread/list`) 한다([D-13](../03-decisions.md) Codex 사용량 최소화).
- 유지보수: 경로가 자주 바뀐다(이번에도 대화 경로·생성 경로가 바뀌었다). 버전 올라갈 때 재확인.

### 구현 상태 (2026-11-14)

- 됨: Gateway `cloud` 호스트(환경 변수 또는 플래그 파일로 켬), method 허용 목록(공개 프로토콜의 읽기와 대화 쓰기),
  로그인 거절 시 close 1008, keepalive(10초 ping, 60초 무응답 끊음). Desktop 은 설정으로 켜고, 원격 구역에서
  `thread/read` 로 읽고, 사용자가 세션에서 한 번 허락하면 `thread/resume`·`turn/start`·`turn/steer` 로 보낸다.
- 됨: dot 목록. Gateway `akra/dots/list` 가 `GET https://chatgpt.com/backend-api/tbo` 를 읽어 dot 의 이름·상태·
  현재 작업 thread 를 준다. Desktop 원격 구역이 dot 을 보이고, 누르면 그 작업 thread 를 연다.
- 됨: dot 과의 대화 읽기. Gateway `akra/dots/messages` 가 `GET /messaging/rooms/{id}` 와 `.../messages?limit=30`
  을 읽어 메시지(보낸 쪽 이름, dot 여부, 글)를 준다. Desktop 은 dot 을 누르면 이 대화를 본 창의 옆 칸에 보이고, 따로 뜬 창으로
  옮길 수 있다. 작업 thread 는 사이드바 Dots 구역의 "작업 대화"에서 연다. 서비스는 모든 메시지의 `role` 을 `user` 로 주므로 dot 은 `member_profile_snapshots` 의 id 로
  가린다 (계정 하나에서 본 규칙).
- 됨 (2026-11-14 실제 전송 1회로 확인, HTTP 200): 메시지 방에 보내기. Gateway `akra/dots/send { roomId, text }` 가
  `POST /messaging/rooms/{id}/messages` 에 `{"content": {"text": ...}, "request_id": "<uuid>"}` 를 보낸다. Desktop 은 dot 대화의 배너에서
  "이 세션에서 보내기 허용"을 누르고 경고를 확인한 뒤에만 입력창을 보인다 ("세션 한 번 허용").
  - 본문 모양은 2026-11-14 에 검증 오류로 확인했다 (타입이 틀린 본문만 보냈고 메시지는 만들어지지 않았다):
    `content.text` 필수 글자, `content.attachments` 선택 목록, `message_metadata`·`reply_to` 선택 객체
    (`message_metadata` 는 모르는 필드를 거절한다).
  - `request_id` 는 필수이다. 없으면 HTTP 422 `Messaging with the user's dot requires a stable send identifier` 이다
    (사용자의 첫 전송이 이 오류로 실패했다). `Idempotency-Key` 헤더는 대신하지 못한다. 답은 만든 메시지이고
    `request_id` 를 그대로 돌려준다. 기기 증명은 요구하지 않았다.
- 됨 (2026-11-14 실계정 확인): 실시간 상태. Gateway `akra/dots/watch` 가 방의 상태를 `akra/dots/live` 알림으로
  민다. 순서가 있다: `GET /celsius/ws/user` 의 `websocket_url`(서명 포함, `ws.chatgpt.com`)에 붙어 명령 묶음
  `[{id, command:{type:"connect", presence:{type:"presence", state}}}, {id, command:{type:"subscribe", topic_id:
  "calpico-room-live:<room>"}}]` 을 보내고, 구독 답 뒤에 `POST /messaging/rooms/{id}/live`(SSE)를 연다. SSE 는
  `calpico-room-live-ready` 와 ping 만 주고, 상태는 WebSocket 으로 온다: `[{type:"message", topic_id, payload:{type:
  "calpico-room-live", payload:{type:"calpico-room-live-state", room_id, threads:[{turn_status, is_active,
  computer_status, activity_status:{status_type, status_label}}], revision}}}]`. 구독만 하고 SSE 를 열지 않으면
  아무것도 오지 않는다. `connect` 없이 구독하면 `not_connected`, 배열이 아니면 `invalid_command_batch` 이다.
  메시지를 보내면 0.1초 뒤 `turn_status: in_progress`, 답이 끝나면 `completed` 가 온다(실측 5.7초). 새 메시지 자체의
  알림은 없다. 그래서 "일하는 중 -> 끝남"을 답으로 보고 대화를 다시 읽는다.
  - 다른 주제도 있다: `app_notifications`, `conversations`, `tbo-tasks`, `resource-updates`. 구독은 되지만 시험한
    40초 동안 아무것도 오지 않았다. 쓰지 않는다.
- 됨: 읽음 표시(dot 이 읽은 시각). `GET /messaging/rooms/{id}` 의 `read_receipts` 에서 dot 의 `last_read_at` 을 쓴다.
  내가 읽었다고 알리는 것(`.../read`)은 하지 않는다.
- 안 됨: 반응, dot 관리(D3),
  클라우드 컴퓨터(D4), §3 의 요청 변환·토큰 갱신·압축. dot 목록은 첫 페이지만, 대화는 최근 30개만 읽는다 (`limit` 은 30 까지 통과, 40 부터 HTTP 422).
- 확인됨 (2026-11-14, 실계정, 턴·쓰기 없음): 클라우드 WebSocket 이 공개 쓰기 method 를 안다. 없는 thread id 로
  보내면 `turn/start`·`turn/steer`·`turn/interrupt` 는 `-32602 unknown thread`, `thread/resume`·`thread/read` 는
  `-32600 thread not found` 로 답한다 ("모르는 method" 가 아니다). Desktop 이 실제로 보내는 `initialize`
  (clientInfo `akra-desktop`, experimentalApi false)와 `thread/list {limit:100, useStateDbOnly:true}` 도 통과한다.
  다리 전체(`open_session` -> 가드 -> 중계 -> 클라우드)도 통과한다. 재현: `cargo test --lib
  cloud_upstream::tests::live_desktop_requests -- --ignored`, `...::live_bridge_session -- --ignored`.
- 확인 안 됨: 실제 턴이 끝까지 성공하는지 (G2 의 나머지). method 가 있다는 것까지만 봤다. 실제 입력을 서버가
  받는지는 사용자가 허락한 전송 1회로 확인한다.
- 확인됨 (2026-11-14, 실계정 읽기 1회, 턴 없음): 클라우드 WebSocket 이 `initialize`, `thread/list`(스레드 2개),
  `thread/read`, `thread/turns/list` 를 모두 받는다. 토큰·내용은 찍지 않고 개수만 셌다. 재현: `cargo test --lib
  cloud_upstream::tests::live_cloud_reads -- --ignored`.
- 확인됨 (2026-11-14, 실계정 읽기): REST 의 기준 호스트는 `https://chatgpt.com/backend-api` 이다. `GET /tbo` 가
  `{items, cursor}` 를, `GET /tbo/{id}/threads` 가 `{items[{thread_id, parent_thread_id, is_user_visible,
  is_dot_managed, attached_at}], cursor}` 를, `GET /tbo/{id}/root-thread` 가 `{root_thread_id}` 를 준다.
  `codex-cloud-backend` 호스트의 `/tbo` 는 404 이다.

## 7. 위험

- 약관·계정 제재: 공식 앱 신원과 비공개 API 를 흉내 낸다. D0 통과로 줄지 않았고, 증명 생략이 신호가 될 수 있다.
- 안정성: 서버가 증명·신원 검사를 강화하면 바로 막힌다.
- 공식 허용 경로: 공개 codex CLI 로 ChatGPT 로그인 후 로컬 app-server. dots 와 같은 방향(상주 루프가 대화를
  관찰하고 먼저 말 걸기)은 로컬에서 [plan 20](../../20-akra-observe-and-offer.md)으로도 만든다.
