# 25. 클라우드 에이전트 호스트 (Akra 안에서 클라우드 에이전트와 대화)

상태: **Proposed**. 현재 제품 동작을 설명하지 않는다. 시작 조건(§1)이 아직 충족되지 않았다. · 기준일: 2026-11-13 (Asia/Seoul) ·
기준 커밋: `1736d22a` (`origin/prerelease`) ·
관련: [plan 17 Akra Desktop](17-akra-desktop/README.md) ([결정 D-03, D-05, D-11, D-13](17-akra-desktop/03-decisions.md),
[R7 Claude 백엔드](17-akra-desktop/roadmap/R7-claude-backend.md)), [plan 18 kernel](18-akra-kernel-architecture.md),
[plan 20 관찰-제안](20-akra-observe-and-offer.md), [plan 21 MCP hub](21-akra-mcp-hub.md),
[plan 22 작업 컨텍스트](22-akra-work-context.md), [plan 23 주의 항목](23-akra-attention-items.md)

## 한 줄 요약

Gateway 가 app-server 호스트를 여러 개 중계한다. 두 번째 호스트가 클라우드 에이전트의 app-server 이면, Desktop 이 그
에이전트의 대화를 Akra 안에서 읽고 메시지를 보낸다. kernel 은 그 대화를 소유하지 않는다.

## 범위

- 이 문서는 Akra 쪽 설계만 다룬다. 클라우드 쪽의 인증과 호출 방법은 적지 않는다. 그 부분은 공식 경로가 생길 때
  채운다 (§1).
- 클라우드 에이전트가 이 컴퓨터의 Akra 를 쓰는 방향은 plan 21 4단계가 맡는다. 그 에이전트가 남긴 일이 Akra 로
  들어오는 방향은 plan 20 §4.1 이 맡는다. 이 문서는 세 번째 방향이다: 사용자가 Akra 화면에서 그 에이전트와
  대화한다.
- 공식 앱의 내부 구조는 이 저장소에 적지 않는다 (D-03 의 뒷부분).

## 1. 시작 조건 (G0)

아래 중 하나가 있을 때만 S1b 이후를 시작한다.

- 공급자의 공식 문서가 외부 클라이언트의 접속 방법을 적는다.
- 공식 CLI 가 그 접속을 지원한다.
- 공식 API 가 그 에이전트를 다룬다.

2026-11-13 기준으로 셋 다 없다. 그래서 지금 할 수 있는 조각은 S1a 뿐이다. S1a 는 R7(Claude 백엔드)도 쓴다.

D-03("클라우드 에이전트 비공개 백엔드 연결", 제외)은 그대로 둔다. G0 이 충족되면 D-03 을 "대체됨"으로 바꾸고 아래
결정을 둔다.

- 결정 (제안): 클라우드 에이전트는 공개 프로토콜 타입과 공식 인증 경로로만 연결한다. 공식 인증 경로가 없으면
  연결하지 않는다.

## 2. 사용 장면

| # | 장면 | 필요한 것 |
| --- | --- | --- |
| U1 | 사이드바에서 내 에이전트 목록을 본다 | 목록 읽기 |
| U2 | 에이전트의 대화와 진행 중인 작업을 읽는다 | thread 읽기, 실시간 알림 |
| U3 | 에이전트에게 메시지를 보낸다 | 쓰기 한 종류 |
| U4 | 에이전트의 결과를 로컬 thread 로 넘긴다 | 기존 "스레드로 넘기기" (plan 21 3단계) |

하지 않는 것: 에이전트 만들기, 설정, 예약 작업, 외부 메신저 연결, 파일 올리기. 공식 앱에서 한다.

## 3. 구성 요소

```text
Desktop ── Gateway ─┬─ 호스트 "local"   : 지금의 app-server (그대로)
                    └─ 호스트 "<원격>"  : 클라우드 app-server (새로)
kernel: 원격 thread 를 소유하지 않는다. 명령도 내지 않는다. 기록도 하지 않는다.
```

### 3.1 Gateway

- 지금 구조: 클라이언트 접속 하나가 upstream 연결 하나를 연다. 메시지를 바꾸지 않고 중계한다 (D-11).
- 변경: 접속할 때 호스트를 고른다. WebSocket 요청 경로 `/host/<id>` 를 쓴다. 경로가 없으면 `local` 이다.
- 요청마다 라우팅하지 않는다. 그래서 중계 코드는 바뀌지 않는다.
- `GatewayUpstreamPort` 를 호스트 id 로 찾는 목록 하나를 둔다. 모르는 id 의 접속은 close 1008 로 닫는다.
- `akra/gateway/info` 가 호스트 목록과 상태를 답한다.
- `local` 이 아닌 접속의 `akra/kernel/*` 는 거절한다. `akra/mcp/*` 와 `akra/context/*` 는 그대로 답한다.
- `GatewayRecorder` 는 `local` 이 아닌 접속을 기록하지 않는다. 원격 대화가 디스크에 남지 않는다.

### 3.2 원격 upstream adapter (G0 뒤)

- `GatewayUpstreamPort` 의 두 번째 구현이다. 자식 프로세스 대신 TLS WebSocket 을 연다.
- TLS 의존성이 지금 없다. `tokio-tungstenite` 는 `handshake` 기능만 켜져 있다. `native-tls`(운영체제의 TLS)와
  `rustls` 중 하나를 고른다 (Q-C1).
- 재접속: 연결이 끊기면 세대 번호를 올린다. 세션은 close 1012 로 닫히고 Desktop 이 다시 붙는다. `local` 과 같은
  규칙이다.
- 자격 증명: G0 이 정한 공식 방법으로만 받는다. 이 문서는 정하지 않는다.
- 설정: 사용자가 켠 호스트만 연다. 기본값은 꺼짐이다.

### 3.3 Desktop main

- 호스트마다 Gateway 접속 하나를 연다. 접속 상태를 호스트별로 둔다.
- 접속 옵션에 호스트 id 를 더한다. endpoint 를 찾는 방법은 바뀌지 않는다.
- 원격 접속은 사용자가 설정에서 켠 뒤에만 연다.

### 3.4 Desktop renderer

- 사이드바에 원격 에이전트 구역을 둔다. 꺼져 있으면 보이지 않는다.
- 대화 화면은 기존 thread 화면을 다시 쓴다. 차이는 세 가지이다: 호스트 표시, kernel 패널 숨김, 승인 UI 숨김.
- 입력창은 G2 전까지 읽기 전용이다.

### 3.5 kernel

- 원격 thread 를 소유하지 않는다. auto-follow, planning, 병렬 모드는 `local` 에만 적용한다.
- "에이전트가 답을 기다린다"를 주의 항목(plan 23)으로 다루는 일은 이 문서의 범위 밖이다.

## 4. 데이터

- thread 의 식별자는 `(호스트, thread id)` 이다. renderer 의 저장 키를 이것으로 바꾼다.
- 원격 기록은 복사하지 않는다 (D-05). 캐시는 메모리에만 둔다.
- 작업 컨텍스트(plan 22)의 장부에 원격 thread 를 넣지 않는다.

## 5. 단계와 관문

| 단계 | 내용 | 바뀌는 곳 | 검증 | 통과하지 못하면 |
| --- | --- | --- | --- | --- |
| S1a | 호스트 목록, 경로로 선택, info, 기록 제외 | `src/adapter/inbound/gateway/`, `src/composition/production.rs` | 가짜 두 번째 upstream 으로 Gateway 시험. `local` 회귀 없음 | - |
| G0 | 공식 접속 경로 확인 (§1) | - | 공식 문서 또는 공식 CLI | 여기서 멈춘다 |
| S1b | 원격 adapter, TLS, 설정 | `src/adapter/outbound/app_server/`, `Cargo.toml` | 로컬 TLS 가짜 서버 | - |
| G1 | 읽기 전용 확인 | - | `initialize` 와 목록 읽기가 성공한다. 모델 턴 없음 (2026-11-14 통과) | S2 이후를 하지 않는다 |
| S2 | Desktop 다중 접속, 원격 구역, 읽기 화면 | `desktop/src/main`, `desktop/src/renderer` | vitest, 가짜 Gateway e2e (숨긴 창), 실제 계정 확인 한 번 | - |
| G2 | 쓰기 경로 확인 | - | 공개 프로토콜의 method 가 원격 thread 에서 동작한다 (2026-11-14: 서버가 method 를 아는 것까지 확인. 실제 턴은 사용자가 보낼 때 확인) | 읽기 전용으로 끝낸다 |
| S3 | 메시지 보내기 | renderer 입력창, main | 사용자가 허락한 턴 한 번 (D-13) | - |

- S3 (보내기)는 "세션 한 번 허용"으로 구현했다 (사용자 결정 2026-11-14): 원격 대화의 배너에서 "이 세션에서
  보내기 허용"을 누르면 그 호스트 세션이 쓰기 가능해진다. 그 뒤 thread/resume 으로 구독하고 composer 로
  turn/start·turn/steer 를 보낸다. Gateway cloud 다리는 공개 쓰기 method 만 허용한다(비공개 turn/addUserMessage
  는 넣지 않는다). 실계정 턴은 사용자가 보낼 때만 일어난다. G2 는 그 전송으로 확인된다.

- 규모 (코드 조사의 추정): S1a 1~2일, S1b 2~3일, S2 3~5일, S3 1~2일.
- S1a 만 G0 과 무관하다. S1b 부터는 G0 이 충족된 뒤에 한다.
- S1a 는 구현되었다 (Q-C2, Q-C3 의 기본값을 썼다). 동작은
  [current-product](../reference/current-product.md) 의 Gateway 절에 있다. 운영 구성은 `local` 만 등록하므로
  `composition/production.rs` 는 바뀌지 않았다. 원격 접속의 `akra/context/*` 는 빈 `resources` 로 답한다.
- S2 의 Desktop 부분은 사용자의 요청으로 G0, S1b, G1 보다 먼저 구현되었다: 호스트별 접속, 켜고 끄는 설정과 경고,
  사이드바의 원격 구역, 읽기 전용 대화 화면. 가짜 Gateway 로만 검증했다. 운영 구성에는 원격 호스트가 없으므로
  실제 클라우드 에이전트에는 연결되지 않는다. 그 연결은 여전히 G0 과 S1b 가 필요하다. 남은 것: 실제 계정 확인,
  서버 버전 경고 (§6), 수동 새로 고침 (§8), 인증 만료 알림 (§7).

## 6. 프로토콜 버전 차이

- 타입은 `codex app-server generate-ts --experimental` 의 결과만 쓴다. 공개 프로토콜에 없는 method 는 손으로 적지
  않는다.
- `initialize` 의 답에서 서버 버전을 읽는다. 생성 타입의 버전과 다르면 원격 구역에 경고를 보인다.
- 모르는 알림은 버린다. 모르는 item 종류는 "지원하지 않는 항목"으로 그린다.

## 7. 실패 모드

| 상황 | 동작 |
| --- | --- |
| 원격 접속 실패 | 원격 구역만 "연결 안 됨"이다. `local` 은 정상이다 |
| 인증 만료 | 한 번만 알린다. 스스로 다시 시도하지 않는다 |
| 응답이 WebSocket 한도(256 MiB)를 넘음 | `local` 과 같은 한도이다. 넘으면 세션이 닫힌다. 페이지 단위 읽기로 바꿔야 한다 |
| 서버가 method 를 거절 | 그 기능만 끈다 |
| Gateway 재시작 | 모든 호스트 접속이 다시 붙는다 |

## 8. 보안

- 자격 증명은 Gateway 프로세스 안에만 둔다. renderer, 로그, kernel 기록, Gateway 기록 파일에 넣지 않는다.
- 기본값은 꺼짐이다. 켤 때 경고 문장을 보인다 (약관과 계정 위험).
- 주기 폴링을 하지 않는다. 구역을 열 때와 수동 새로 고침에만 읽는다. 예외: 사용자가 dot 에게 메시지를 보낸 뒤에는
  답이 올 때까지만 2초 간격으로 읽는다 (최대 45초). 보낸 사람이 답을 기다리는 동안이고, 끝이 정해져 있다.

## 9. 미결 질문

| ID | 질문 | 기본값 (답이 없을 때) |
| --- | --- | --- |
| Q-C1 | TLS 의존성은 `native-tls` 인가 `rustls` 인가 | `native-tls`. 새 암호 코드가 들어오지 않는다 |
| Q-C2 | S1a 를 G0 과 별개로 먼저 하는가 | 예. R7 과 공유한다 |
| Q-C3 | 원격 접속에서 `akra/context/*` 를 답하는가 | 답한다. 장부에는 원격 thread 를 넣지 않는다 (§4) |
| Q-C4 | 원격 호스트가 공개 프로토콜의 method 를 받는가 | 모른다. G1 과 G2 에서 확인한다 |
