# Agent Canvas와 Akra 간 격차 행렬

[English](../../../competitive/agent-canvas/gap-matrix.md)

이 행렬은 [v1.2.1 분석](analysis.md)과 [증거 원장](evidence.md)을 Akra 관점의 결정으로 전환한다.
브라우저 화면 범위나 공급자 수를 목표로 취급하지 않는다. 수락하는 작업은 Akra의 Codex 우선 운영
및 전달 입지를 강화하고 작은 소유권 단위로 검토할 수 있어야 한다.

## 상대 비교 행렬

| 기능 | Agent Canvas v1.2.1 | Akra 기준선 | 결정 | 우선순위 | 증거 |
| --- | --- | --- | --- | --- | --- |
| 선택 세션 검사기 | 응집력 있는 채팅, 파일/diff, 관찰된 터미널, 브라우저 스크린샷, 태스크/플래너 탭 | 네이티브 inline 대화와 집중형 overlay 및 Admin 뷰 | 활동/출처 응집성만 채택 | P0/P1 | [Canvas](evidence.md#selected-conversation-ux), [Akra](evidence.md#akra-baseline-evidence) |
| 다중 세션 개요 | 거친 활동 상태를 보여 주는 필터 가능한 대화 레일 | 실제 에이전트/태스크/worktree/전달 토폴로지가 있지만 제한된 현재 도구/활동은 없음 | Akra 토폴로지 확장 | P0 | [Canvas](evidence.md#selected-conversation-ux), [Akra](evidence.md#akra-baseline-evidence) |
| Codex 프로토콜 | 출시된 내장 코어 ACP relay, 중요한 종단 간 손실 | 스키마 분류와 불완전한 풍부한 이벤트 축약을 갖춘 직접 app-server 어댑터 | 구조적 우위를 보존하고 측정 | 지속 | [ACP](evidence.md#released-codex-acp-path), [Akra](evidence.md#akra-baseline-evidence) |
| 권한 | 바깥쪽 확인 꺼짐, ACP 브리지가 옵션 0 선택 | 직접 경계이지만 accept/decline만 지원; file-change approval과 MCP elicitation은 양식 없이 fail closed | 자동 선택 거부, Akra의 선택 충실도 개선 | 불변 조건/P1 | [ACP](evidence.md#released-codex-acp-path), [Akra](evidence.md#akra-baseline-evidence) |
| 병렬 격리 | 연결된 작업 공간 기본값은 로컬, scratch는 worktree 요청으로 폴백, 정리 경로 없음 | 병렬 lane별 leased worktree | 전달 권한으로 차별화 | 출시됨 | [Canvas](evidence.md#conversation-creation-reconnect-and-workspaces), [Akra](evidence.md#akra-baseline-evidence) |
| Git 전달 | pull/push/PR 명령이 에이전트 프롬프트가 됨 | 고정 범위, 리뷰/검사 게이트, 직렬화된 통합, 원격 검증, 정리가 출시됨; 정확한 SHA 검증 증거는 계획됨 | 현재 검증을 과장하지 않고 차별화 | 출시됨 + 기존 P0/P1 | [Canvas](evidence.md#selected-conversation-ux), [Akra](evidence.md#akra-baseline-evidence), [계획된 검증](../jcode/gap-matrix.md#p0-frozen-source-validation-evidence) |
| 원격 백엔드 | 로컬/원격/클라우드용 브라우저 레지스트리와 상태 | loopback Admin, Telegram 제어 영역 | 향후 읽기 전용 Akra 노드로 채택 | P2 | [Canvas](evidence.md#backend-registry-and-telemetry), [Akra](evidence.md#akra-baseline-evidence) |
| 자동화 | 일정, webhook, 영속 실행, 대화/로그 링크, 일부 UI | 일정/webhook/실행 원장 없음 | 검토된 코드 변경 수신용으로 좁게 채택 | P1 | [Canvas](evidence.md#automation-114), [Akra](evidence.md#akra-baseline-evidence) |
| 자동화 정확성 | 재전송 중복 제거 없음, 6개 상태 백엔드/4개 상태 UI 불일치 | 아직 대응 도메인 없음 | 복사된 결함 방지 | P1 불변 조건 | [Canvas](evidence.md#automation-114), [Akra](evidence.md#akra-baseline-evidence) |
| 게임 운영 | 비교할 수 있는 플릿 게임 없음 | 관련 이벤트와 무관하게 작업을 애니메이션하는 diorama | Akra의 진실성 격차 수정 | P0 | [Akra](evidence.md#akra-baseline-evidence) |
| 자격 증명 | 브라우저 스토리지의 백엔드 키, 광범위한 ACP 하위 환경 | 필터링된 app-server 하위 환경, 원격 레지스트리 없음 | 브라우저 소유 거부 | 불변 조건 | [Canvas](evidence.md#backend-registry-and-telemetry), [Akra](evidence.md#akra-baseline-evidence) |
| 대화형 성능 | 비교 가능한 증거 없음 | 완전한 네이티브 성능 artifact가 아직 없음 | 기존 증거 계약 확장 | P0 의존성 | [제한](evidence.md#audit-limits), [계획된 계약](../jcode/gap-matrix.md#p0-native-performance-evidence-contract) |

## 채택

### 1. 플릿 진실로서의 현재 활동

Canvas는 선택한 한 대화 안에서 상세한 명령, 파일, reasoning, 태스크 상태가 갖는 가치를 보여 준다.
Akra에는 이미 더 가치 있는 플릿 토폴로지, 선택형 `:peek`, 검증, 리뷰, 전달, 정리 상세가 있다.
빠진 부분은 공식 app-server 이벤트에서 동일한 병렬 진실로 전달되는 제한된 현재 활동 envelope다.

다음을 채택한다.

- 자유 형식 상태 문자열 대신 타입이 있는 활동 종류와 단계
- 현재 app-server 항목 식별자와 제한된 운영자 요약
- 수명주기 경과 시간과 분리된 마지막 활동 시간
- Admin 애니메이션과 클라이언트가 관련 실제 변경을 식별할 수 있도록 authority가 할당하는 단조 증가
  runtime-event sequence
- 출력 delta마다 SQLite를 한 번 쓰는 대신 병합된 영속화와 종결 flush
- TUI, Admin JSON, 선택 상세에서 동일한 투영

이는 기존 jcode
[병렬 활동 및 완료 증거](../jcode/gap-matrix.md#p1-parallel-activity-and-completion-evidence)
구현 단위를 수정하고 우선순위를 높인다. 두 번째 이벤트 스토어, transcript 모델 또는 검사기가 되면
안 된다.

현재 Akra 대화 활동에는 완료된 명령 및 파일 변경 요약만 있다. Item start, command delta,
MCP/tool progress, reasoning을 유지하려면 먼저 기존 jcode
[프로토콜 네이티브 실시간 실행 레일](../jcode/gap-matrix.md#p0-protocol-native-live-execution-rail)이
타입 있는 이벤트를 보존해야 한다. 병렬 활동 구현 단위는 그 도메인 출력을 소비하며 원시 app-server
JSON을 다시 파싱하지 않는다.

### 2. 영속적인 트리거 및 실행 출처

유용한 Automation 패턴은 임의의 워크플로 실행이 아니다. 상태, 타임스탬프, 입력 메타데이터,
연결된 대화, 명령/로그 증거, 취소, 복구를 갖춘 트리거 정의와 특정 실행의 영속적인 구분이다.

Akra 코드 변경 수신을 위해 다음을 채택한다.

- 일정 및 서명된 이벤트 트리거
- 활성화, 비활성화, 즉시 실행, 취소, 변경 불가능한 시도 이력
- 타입 있는 매개변수를 갖춘 운영자 작성 태스크 템플릿
- 태스크 생성 전 트리거별 멱등성
- 수신 처분, 자동화 실행/시도 상태, 권위 있는 전달 결과의 분리
- 전달 성공을 다시 정의하지 않고 skipped, cancelled, timed out, failed, completed, unknown external
  state를 모두 다루는 완전한 실행 상태
- 트리거에서 수락 태스크, lease/session, 검증, PR, 전달 결과, 정리까지 하나의 출처 체인
- 조용한 상태 수정이 아닌 기록된 이유를 갖춘 watchdog 복구

원시 webhook 페이로드를 에이전트 프롬프트로 전달하지 않는다. 페이로드는 신뢰할 수 없는 데이터이며
운영자 소유 템플릿이 수락한 제한된 필드만 채울 수 있다.

Automation은 `integrated` 또는 `terminal_failed`를 자동화 enum에 추가하는 대신 기존 jcode
[권위 있는 전달 결과](../jcode/gap-matrix.md#p1-authoritative-delivery-outcomes)를 참조해야 한다.
이 구현 단위가 완료되기 전에도 수신을 수락할 수 있지만 종결된 검토 전달 추적을 주장할 수는 없다.

### 3. 읽기 전용 원격 노드 인식

Canvas는 endpoint 상태, 백엔드 연결, 상태 저하 UI를 검증한다. 운영자 마찰을 줄일 수 있다고
합리적으로 추론할 수 있지만 그 효과는 측정하지 않았다. Akra는 로컬 전달 결과와 자동화 출처가
권위 있게 된 후에 이 운영 형태를 차용해야 한다.

첫 원격 계약은 읽기 전용이다.

- Akra 노드 ID, 버전, 기능, 작업 공간 범위
- 상태, 마지막 성공 시간, 스냅샷 경과 시간, 명시적인 오래된 상태
- 동일 애플리케이션 투영에서 가져온 선택 태스크/세션/전달 요약
- 상호 인증된 HTTPS ID와 버전 협상
- 브라우저 스토리지가 아닌 OS 비공개 또는 서버 측 스토리지의 자격 증명

첫 구현 단위에는 원격 변경, 공급자 전환 또는 ACP 레지스트리가 들어가지 않는다.

### 4. 선택 실행 딥 링크

Canvas의 실행-대화 및 로그 링크는 좋은 정보 아키텍처 패턴이다. Akra는 안정적인 도메인 ID를
사용해 더 나아가야 한다.

```text
trigger -> run -> accepted direction/task -> lease -> agent session -> app-server thread
-> frozen source -> validation -> PR/review -> delivery attempt -> integration/cleanup
```

TUI와 Admin은 서로 다른 밀도로 렌더링할 수 있지만 둘 다 같은 식별자와 종결 결과를 해석해야 한다.

## 거부

### Akra 핵심 런타임으로서의 ACP

출시된 Canvas Codex 경로는 내장 Codex crate 위의 Agent Server/ACP relay다. 이후 SDK 경로는 이
Rust 브리지를 공식 app-server 하위 프로세스를 실행하는 Node ACP 브리지로 교체한다. 둘 다 Akra에
불필요한 버전 및 의미 변환 경계를 만든다.

다음을 거부한다.

- Akra와 app-server 사이의 ACP-to-Codex 어댑터
- Codex 프로토콜 충실도를 약화할 이유로서의 다중 공급자 동등성
- Canvas 소비자까지 추적하지 않고 래퍼 지원을 종단 간 지원으로 해석하는 것
- 출시된 ACP 경로에서 plan, permission, fork, diff, identity 또는 reasoning 의미를 복사하는 것

대신 Akra는 직접 이벤트 투영을 개선하고 golden trace로 필드 보존을 검증해야 한다.

### 브라우저 IDE 복제

Monaco, 읽기 전용 xterm 로그 또는 브라우저 스크린샷 패널을 복제하지 않는다. Akra의 주 화면은
네이티브 inline 터미널이고, 운영자는 두 번째 editor shell보다 실시간 실행 및 전달 진실이 더
필요하다. 향후 Admin 상세 뷰는 기존 애플리케이션 서비스를 통해 제한된 diff, 로그, 출처를 보여 줄
수 있지만 완전한 IDE가 되면 안 된다.

### 범용 자동화 마켓플레이스

범용 Slack, Notion, 임의 SaaS 액션 또는 제한 없는 스크립트 마켓플레이스를 만들지 않는다. Akra
자동화는 인증된 이벤트를 검토된 Codex 코드 변경 전달로 바꾸기 위해 존재한다. 새 소스는 독립적인
워크플로 권한을 얻지 말고 해당 애플리케이션 명령에 매핑되어야 한다.

### 프롬프트만 사용하는 전달 제어

에이전트에게 pull, push 또는 PR 생성을 요청하는 버튼은 의도한 브랜치, 소스 SHA, 검사, 리뷰, 통합
ref 또는 정리 상태가 올바르다는 증거가 아니다. 명시적인 전제 조건과 영속 결과가 있는 애플리케이션
서비스 및 어댑터에 Git과 GitHub 전달을 유지한다.

### 선택적 격리

병렬 전달에서 worktree 격리를 사용자가 선택하는 편의 기능으로 만들지 않는다. lane은 계약상 자신의
lease와 worktree를 소유한다. 공유 checkout 대화 사용은 병렬 모드 전달 권한과 분리하여 유지할 수
있다.

### 브라우저 저장 자격 증명과 자동 승인

원격 노드 자격 증명은 브라우저 로컬 스토리지에 들어가면 안 된다. ACP의 첫 옵션 권한 선택은 Akra
권한 설계에 영향을 주면 안 된다. 운영자 승인, 상위 정책, 명시적인 리뷰 생략 전달 출처는 서로 다른
권한이며 구별되어야 한다.

## 차별화

### 대화 완료가 아닌 전달

Canvas는 에이전트가 멈췄음을 보여 주고 자동화 실행을 대화에 연결할 수 있다. Akra는 더 강한 종결
결과를 증명해야 한다.

```text
accepted intent
-> leased isolated worktree
-> source range frozen
-> validation bound to the exact frozen source SHA
-> source published
-> reviewed PR gates satisfied by default, or explicit high-risk exception recorded
-> integration applied in serialized authority
-> integration ref pushed and remotely verified
-> PR and source lane cleaned
```

각 전환에는 소유자, 시간, 소스 ID, 이유, 재시도/복구 상태가 필요하다. 에이전트 텍스트가 이 증거를
대체할 수 없다.

### 직접 Codex 프로토콜 책임

Canvas의 ACP 경로는 프로토콜 relay가 기능을 광고하면서 운영자가 보기 전에 그 기능을 잃을 수 있는
이유를 보여 준다. Akra의 직접 경계는 그 차이를 측정 가능하게 만들어야 한다.

- 모든 공식 알림을 명시적으로 계속 분류한다.
- 보존하는 이벤트는 애플리케이션에 필요한 네이티브 thread, turn, item ID를 유지한다.
- 제한된 축약은 무엇을 생략하는지 선언한다.
- 권한 및 오류 타입을 일반 텍스트로 축약하지 않는다.
- 결정론적 golden trace가 보존, 변환, 삭제된 필드를 보고한다.
- 프로토콜 drift가 UI를 조용히 바꾸기 전에 계약 테스트를 실패시킨다.

이는 모든 원시 프레임을 TUI에 노출하라는 요청이 아니라 지속적인 아키텍처 불변 조건이다.

### 진실한 게임 운영

Akra Admin diorama는 진실을 말할 때만 차별점이 될 수 있다. 현재 worker는 영속 작업 전환 없이
돌아다니고 패킷은 흐르며, 시각 검사는 idle 픽셀이 바뀌기를 기대한다. 이를 의미 있는 상태 및
이벤트 트리거 이동으로 교체한다.

- `idle`
- `starting`
- `working`
- `awaiting_review`
- `blocked`
- `delivering`
- `cleanup`
- `stale`

이동과 색 패킷은 새 활동 또는 수명주기 시퀀스의 결과여야 한다. 주변 애니메이션을 유지한다면
시각적으로 중립이어야 하며 처리량을 암시하면 안 된다. 알 수 없는 metric은 조작된 진행률을 받지
않고 알 수 없음으로 남는다.

### 하나의 진실, 여러 화면

Canvas는 응집력 있는 브라우저의 이점을 누리지만 여러 핵심 진실을 독립 서비스에 위임한다. Akra는
애플리케이션 서비스를 권위 있게 유지하고 네이티브 TUI, Admin, CLI, Telegram, scheduler/webhook
어댑터, 향후 읽기 전용 원격 노드에서 동일한 상태를 노출해야 한다. Inbound handler가 태스크, Git,
GitHub 또는 SQLite 상태를 직접 변경하면 안 된다.

<a id="implementation-slices"></a>

## 구현 단위

<a id="p0-parallel-current-activity-envelope"></a>
### P0: 병렬 현재 활동 Envelope

이는 기존 jcode 병렬 활동 작업 항목을 더 높은 우선순위와 더 정확한 계약으로 강화한 것이며 새 병렬
하위 시스템이 아니다.

**전제 조건**

기존
[프로토콜 네이티브 실시간 실행 레일](../jcode/gap-matrix.md#p0-protocol-native-live-execution-rail)은
item start, command delta, MCP/tool progress, reasoning의 파싱 및 제한된 축약을 소유한다. 이것이
완료되기 전에는 이 구현 단위가 Akra의 기존 완료 명령/파일 활동만 투영할 수 있고 이를 `running`으로
표시하면 안 된다.

**소유 경계**

- `src/domain/parallel_mode/agent_session.rs`
- 병렬 세션 상세 영속화 및 authority 어댑터
- `src/application/service/parallel_mode/{turn,orchestrator_loop,session_detail}`
- 제한된 turn-stream receiver 외부의 고정 용량 last-write-wins 활동 coalescer
- supervisor/control-plane 투영
- 기존 TUI 병렬 roster/detail 및 Admin agent/task 투영

**계약**

- `activity_kind`, `phase`, 선택적 turn/item ID, 제한된 요약, UTC `last_activity_at`, lease
  generation, authority 할당 runtime-event sequence를 저장한다.
- presentation 텍스트를 파싱하지 않고 command, file change, MCP/tool, reasoning,
  other/unknown의 폐쇄된 종류 집합을 정의한다.
- validation, review, delivery, cleanup, blocked 상태를 app-server 활동 종류로 위장하지 않고 기존
  수명주기 필드에 유지한다.
- 현재 lease generation의 이벤트만 수락하고 늦은 stale-lane 이벤트는 버린다.
- authority가 관련 이벤트를 영속화할 때만 sequence를 할당한다. 입력 스트림 이벤트가 전역 sequence를
  가지고 도착한 것처럼 가장하지 않는다.
- 용량 8의 turn receiver 안에서 delta마다 SQLite 쓰기 하나를 await하지 않는다.
- lease당 item key를 최대 32개 유지한다. 같은 key의 progress를 교체하고, active key보다 가장 오래된
  terminal key를 먼저 제거하며, 더 많은 서로 다른 active item은 메모리를 키우지 말고 명시적인
  overflow 개수로 집계한다.
- coalescer 항목 중 8개를 phase/terminal update용으로 예약하며 progress가 이를 밀어내지 못한다.
- 유지된 item의 terminal update는 해당 progress 항목을 교체한다.
- 유지되지 않은 overflow item의 terminal update는 다른 key를 할당하거나 사라지는 대신 제한된
  결과 counter와 truncation marker를 증가시킨다.
- 유지/예약된 용량이 모두 소진되면 교체 가능한 progress만 버리고 영속된 dropped-update counter를
  증가시키며 truncation을 표시한다.
- progress snapshot은 초당 최대 한 번 영속화하고 receiver 밖에서 terminal state를 flush한다.
- 프로세스 내 terminal 보존과 crash 내구성을 별도로 정의한다. turn completion은 동기식 제한된 최종
  handoff를 수행하고, 프로세스 crash는 마지막 coalescing window까지만 잃을 수 있으며 terminal을
  주장하지 말고 stale/unknown으로 복구해야 한다.
- 수명주기 경과 시간과 마지막 app-server 활동 경과 시간을 별도로 유지한다.
- 기존 validation, review, cleanup, conflict, distributor 이력을 보존한다.

**사용자 결과**

프로토콜 전제 조건이 완료되면 운영자는 각 transcript를 열지 않고 `cargo test running`, `patching
src/...`, `quiet for 43s`를 `awaiting review` 같은 별도 수명주기 상태와 함께 훑어볼 수 있다. TUI와
Admin은 동일한 레코드를 상세 표시한다.

**필수 증명**

- command, file, MCP/tool, phase transition, truncation, unknown-event reducer 테스트
- stale lease generation, late item, 동시 authority-sequence 할당 테스트
- write-rate/coalescing, 32-key overflow, reserved-terminal saturation, dropped-progress 계산,
  synchronous final-handoff, crash-window, bounded-receiver nonblocking 테스트
- SQLite 재시작 round trip
- supervisor, TUI narrow/wide, Admin JSON 투영 테스트
- desktop/mobile Admin capture 및 실제 두 lane 실행 한 번

<a id="p0-truthful-diorama-state-machine"></a>
### P0: 진실한 Diorama 상태 머신

**소유 경계**

- 기존 domain/application 수명주기 및 활동 진실
- `DioramaVisualState`와 에이전트별 `visual_transition_id`를 위한 Admin inbound read-model 매핑
- `assets/admin/game/src/akra-diorama.ts`
- Admin dashboard 통합 및 현재 시각 capture 스크립트

**계약**

- 타입 있는 애플리케이션 수명주기/활동 데이터에서 하나의 명시적 visual state를 도출한다.
- 각 에이전트의 `visual_transition_id`를 전역 스냅샷 revision이 아니라 해당 lease generation과
  최근의 관련 영속 runtime-event sequence에서 도출한다.
- 첫 스냅샷에 transition ID가 이미 있어도 애니메이션 없는 기준선으로 초기화한다.
- allowlist에 있는 수명주기/활동 이벤트 때문에 해당 에이전트의 transition ID가 바뀔 때만 의미 있는
  이동과 패킷을 시작한다.
- 동시에 존재하는 사실을 타입 있는 우선순위로 해석한다.
  `stale > blocked > awaiting_review > delivering > cleanup > working > starting > idle`
- 모든 우선순위 상태를 서로 다르게 렌더링하고 우선순위가 낮은 사실도 상세에 유지한다.
- polling이 여러 관련 sequence를 건너뛰면 최신 권위 상태로 한 번만 조정하고 누락된 이벤트마다
  패킷 하나를 조작하지 않는다.
- 주변 애니메이션을 중립적으로 유지하고 의미 있는 작업 애니메이션과 분리한다.
- 사람용 요약을 파싱하여 상태를 계산하지 않는다.
- 텍스트/상태 상세를 제거하지 않으면서 reduced-motion을 존중한다.
- 알 수 없는 KPI 값은 수집되지 않았다고 눈에 보이게 유지한다.

**사용자 결과**

게임 계층은 정직하게 압축된 운영 뷰가 된다. 이동은 기록된 전환을 뜻한다. 정지 상태가 blocker를
숨기지 않고 장식적 이동이 작업 발생을 주장하지 않는다.

**필수 증명**

- 모든 visual state와 unknown data에 대한 투영 테스트
- 비어 있지 않은 transition ID를 사용한 첫 hydration에서 이동이나 패킷이 발생하지 않음
- 동일한 스냅샷 반복 시 의미 있는 위치와 패킷 수가 바뀌지 않음
- 관련 이벤트 하나가 정확히 에이전트 transition 하나를 만들고 관련 없는/전역 이벤트는 만들지 않음
- 건너뛴 sequence catch-up이 재생 패킷 없이 조정 transition 하나를 만듦
- pairwise 및 대표적인 다중 상태 우선순위 테스트
- blocked/idle/stale 및 reduced-motion 테스트
- desktop/mobile canvas pixel 검사와 키보드 접근 가능한 상세
- 현재 idle-motion assertion을 갱신하여 조작된 활동이 아니라 안정성을 증명

<a id="existing-p0-dependency-native-performance-evidence-contract"></a>
### 기존 P0 의존성: 네이티브 성능 증거 계약

두 번째 벤치마크 스키마를 도입하지 않는다. 활성 `fix/native-validation-evidence-contract` lane이
capture/attestation 스키마 소유권을 해제한 후 jcode
[네이티브 성능 증거 계약](../jcode/gap-matrix.md#p0-native-performance-evidence-contract)을 사용한다.
해당 활성 lane은 검증 artifact를 강화하며 benchmark driver, PSS 계산 또는 percentile 계약을
구현하지 않는다. 성능은 별도의 이후 구현 단위로 남는다.

그때에만 Canvas 비교 프로필을 추가한다.

- cold usable browser UI 및 warm conversation resume
- submit to first event 및 first assistant delta
- 3개 대화 list/detail 전파와 Akra의 3개 slot board 전파 비교
- idle 및 active 전체 프로세스 트리 메모리
- browser, static proxy, Agent Server, Automation, ACP, Codex의 명시적 토폴로지 레이블
- 비교 가능한 모드마다 시도된 샘플 최소 30개, 원시 실패 보존, 정확한 버전 및 인증 상태 기록

Canvas 릴리스 게이트 시간과 청크 크기는 이 비교의 입력이 아니다. 현재 성능 승자는 정해지지 않았다.

<a id="p1-automation-intake-core"></a>
### P1: 자동화 수신 코어

**소유 경계**

- 새 도메인 trigger definition, intake key/disposition, run identity, task-template 타입
- `AutomationIntakeService`
- 변경 불가능한 `AutomationIntakeInvocation`, canonical `AutomationIntakeReceipt`,
  accepted-intent/outbox, trigger/run store port, SQLite 어댑터
- intake-to-planning 조정 서비스
- 기존 planning mutation 서비스
- 계약 우선 실행을 위한 JSON CLI inbound 어댑터

**계약**

- 작업 공간 범위, 시간대, 활성 상태, 타입 있는 매개변수, 운영자 소유 태스크 템플릿을 갖춘 일정 및
  이벤트 트리거를 정의한다.
- 태스크를 수락하기 전에 트리거별 멱등성 키를 요구한다. schedule은
  `(trigger_id, scheduled_for_utc)`, webhook은 `(trigger_id, provider_event_id)`, run-now는
  `(trigger_id, operator_request_id)`를 사용한다.
- invocation disposition을 accepted, duplicate, disabled, rejected, conflict로 저장하고 run phase 및
  delivery outcome과 분리한다.
- 서로 다른 모든 inbound request ID에 대해 source, actor, idempotency key, 제한된 parameter hash,
  disposition, reason, receive time, 선택적 canonical receipt 참조를 포함하는 변경 불가능한 invocation
  하나를 영속화한다.
- 각 canonical key claim을 workspace, trigger, source, actor, normalized parameter hash,
  task-template revision에 묶는다.
- 동일 request ID 및 fingerprint의 정확한 replay는 같은 invocation/result를 반환한다.
- 이미 claim된 key와 같은 fingerprint를 가진 새 request ID는 duplicate invocation을 저장하고
  `Duplicate { canonical_receipt_id }`를 반환하지만 run이나 task를 만들지 않는다.
- request ID 또는 idempotency key를 서로 다른 bound fingerprint와 함께 재사용하면 변경 불가능한
  conflict invocation/result를 저장하고 run이나 task를 만들지 않는다.
- idempotency key의 첫 invocation에서는 비활성 또는 거부 상태라도 invocation과 canonical
  receipt/key claim을 원자적으로 영속화하여 이후 replay가 정책/구성 변경 후 수락 상태가 되지 못하게
  한다.
- 해당 canonical disposition이 accepted이면 같은 transaction에서 deterministic run/task ID와
  pending planning outbox intent도 planning 경계를 호출하기 전에 영속화한다. disabled 및 rejected
  receipt에는 run/task가 없다.
- deterministic task ID로 planning task 생성을 멱등적으로 만들고 반환된 task/run을 pending intent에
  CAS-bind한다. 재시작 조정은 묶이지 않은 intent를 완료하거나 두 번째 task를 수락하지 않고 terminal
  binding failure를 기록한다.
- 원시 이벤트 페이로드 텍스트를 system/developer instruction 또는 shell command로 승격하지 않는다.
- 모든 태스크 변경에 애플리케이션 명령을 사용한다.

**사용자 결과**

명시적 멱등성 키를 가진 인증된 호출자는 제한된 코드 변경을 정확히 한 번 요청할 수 있고, 운영자는
재시작 후 수신이 accepted, duplicated, disabled, rejected, conflicted 중 무엇이었는지 증명할 수
있다. 이 구현 단위는 전달 성공을 주장하지 않는다.

**필수 증명**

- schedule, webhook, run-now 키 형태에 대한 duplicate, out-of-order, concurrent idempotency-key
  테스트
- same-request replay, distinct-request duplicate invocation/audit, 같은 key의 서로 다른 payload,
  actor, template revision, concurrent conflict 테스트
- disabled trigger, invalid timezone, malformed parameter, template-bound 테스트
- original key가 나중에 작업을 만들 수 없음을 증명하는 disabled/rejected 후 reenabled replay 테스트
- receipt/intent commit 후, task 생성 후, run-task CAS binding 전/후 crash/restart 테스트와 같은
  intent를 두 reconciler가 경쟁하는 테스트
- cross-workspace 격리 및 untrusted-payload 테스트
- request에서 intake disposition, run identity, accepted task까지의 JSON CLI trace

<a id="p1-schedule-intake-control-loop"></a>
### P1: 일정 수신 제어 루프

Automation Intake Core가 merge된 후에만 시작한다.

**소유 경계**

- scheduler inbound 제어 루프 및 주입된 clock
- 영속 due-trigger claim port/adapter
- `AutomationIntakeService` 명령 경계

**계약**

- scheduler는 명시적인 IANA timezone과 영속 claim을 사용하여 due run을 한 번 수락한다.
- 재시작 및 DST 경계에서 `(trigger_id, scheduled_for_utc)`를 결정론적으로 계산한다.
- `misfire_policy`가 `skip` 또는 `latest_once`이도록 요구하고 기본값은 `latest_once`로 한다. 한 번의
  scan은 trigger당 catch-up intent를 최대 하나 만들고 건너뛴 interval/count를 기록하며, 제한 없이
  과거 run을 만들지 않는다.
- intake 애플리케이션 서비스만 호출하고 planning 또는 run row를 직접 쓰지 않는다.
- skipped/disabled intake를 실패한 delivery로 조작하지 않고 disposition으로 기록한다.

**사용자 결과**

활성 일정은 정시 due instant마다 제한된 planning task 하나를 요청한다. 중단 후에는 누락된 instant가
기록된 `skip` 또는 `latest_once` 정책에 따라 catch-up task를 0개 또는 1개 만들며, 제한 없이
재생하거나 중복하지 않는다.

**필수 증명**

- timezone/DST gap 및 overlap 테스트
- 중복 선점, 두 프로세스 경합, 장기 중단 시 `skip`/`latest_once`, 제한된 따라잡기, 비활성화된 트리거,
  시계 편차 테스트
- 실제 schedule-to-accepted-task trace 한 번

<a id="p1-signed-webhook-intake-adapter"></a>
### P1: 서명된 Webhook 수신 어댑터

Automation Intake Core가 merge된 후에만 시작한다.

**소유 경계**

- HMAC webhook inbound 어댑터
- source identity, timestamp, body-bound, rate-limit 정책
- `AutomationIntakeService` 명령 경계

**계약**

- 타입 있는 intake request를 만들기 전에 source identity, 제한된 body, timestamp window, signature,
  provider event ID를 검증한다.
- `(trigger_id, provider_event_id)`를 도출하고 재시작 후에도 replay를 멱등적으로 만든다.
- 페이로드 필드를 신뢰할 수 없는 타입 있는 매개변수로 취급하며 프롬프트나 shell 권한으로 사용하지
  않는다.
- intake 애플리케이션 서비스만 호출하고 handler에서 task/run storage를 직접 변경하지 않는다.
- source identity, time, signature, event ID 또는 ingress rate policy를 사용할 수 없으면 fail closed
  한다.

**사용자 결과**

서명된 공급자 이벤트는 제한된 코드 변경 태스크 하나를 요청할 수 있는 반면 replay, malformed
payload, unauthenticated traffic은 추가 작업을 만들 수 없다.

**필수 증명**

- signature, timestamp expiry, replay, body-size, rate, missing-event-ID, malformed-parameter 테스트
- restart 및 concurrent-delivery idempotency 테스트
- cross-workspace 및 adversarial payload 테스트
- 실제 signed webhook-to-accepted-task trace 한 번

<a id="p1-automation-run-reconciliation-and-provenance"></a>
### P1: 자동화 실행 조정 및 출처

이 구현 단위가 종결된 검토 전달 추적을 주장하려면 기존 jcode
[고정 소스 검증 증거](../jcode/gap-matrix.md#p0-frozen-source-validation-evidence),
[활성 Turn 종료 및 재시작 복구](../jcode/gap-matrix.md#p0-active-turn-exit-and-restart-recovery),
[중요 리뷰 응답 루프](../jcode/gap-matrix.md#p1-critical-review-response-loop),
[권위 있는 전달 결과](../jcode/gap-matrix.md#p1-authoritative-delivery-outcomes)가 먼저 완료되어야 한다.

**소유 경계**

- automation run/attempt phase 및 linkage 도메인
- 조정 애플리케이션 서비스, versioned `CancelAutomationRun` 명령, 영속 run store
- planning, parallel session, validation, PR, delivery-attempt 진실을 위한 read port
- 기존 소유 서비스의 공유 `CancelQueuedPlanningTask`, lease-bound `StopParallelLane`,
  `AbandonDeliveryAttempt` 애플리케이션 명령

**계약**

- queued, active, cancelling, cancelled, skipped, timed-out, failed, finished, unknown run phase를
  intake disposition과 별도로 모델링한다.
- delivery outcome enum을 다시 정의하지 않고 attempt 및 recovery reason을 연결한다.
- 변경 불가능한 권위 있는 delivery-attempt outcome이 존재하면 이를 참조한다.
- conversation completion, agent text, PR presence 또는 legacy validation summary에서
  `integrated`를 추론하지 않는다.
- 재시작 후 compare-and-set 상태로 조정하며 모든 수정 이유를 기록한다.
- cancellation request ID, actor, expected run version, CAS/idempotency semantics를 요구한다.
- 묶이지 않은 intent에서는 outbox 항목을 취소하고 task, lease, worktree를 남기지 않는다.
- 수락되었지만 lease가 없는 task에서는 `CancelQueuedPlanningTask`를 사용하고 권위 있는 terminal
  task result를 영속화하며 lease 또는 worktree를 남기지 않는다.
- 활성 lane에서는 run을 `cancelling`으로 옮기고 lease-bound `StopParallelLane`을 사용하여 worker를
  terminal하게 중지하며 lease를 해제하기 전에 소유 planning task를 terminal cancelled state로
  CAS하고 재디스패치를 막는다. 해당 CAS가 실패하면 lease/worktree를 유지하고 차단 상태를 눈에
  보이게 유지한다.
- delivery attempt가 있을 때만 abandon하며, 아직 delivery에 도달하지 않은 활성 lane에 attempt나
  outcome을 조작해서 만들지 않는다.
- 정상 cleanup이 깨끗하게 해제된 slot을 확인할 때까지 lease/worktree 소유권을 유지한다. 그 후에만
  run을 cancelled로 표시한다.
- 소스 게시 후 통합 시작 전에는 Authoritative Delivery Outcomes의 terminal-abandon 명령을 사용하고
  PR/audit 이력을 보존하며 정상 branch/worktree 정리 정책을 적용하고 terminal attempt가 영속화된
  후에만 취소를 확인한다.
- integration application이 시작되었거나 원격 integration이 이미 검증된 후에는 변경 없이
  `too_late`를 반환한다.
- requested, confirmed, failed, too-late cancellation을 별도로 기록하고 변경 불가능한 권위 있는
  delivery outcome을 절대 덮어쓰지 않는다.
- cancellation 명령에서 task, slot, SQLite, Git, worktree 또는 GitHub 상태를 직접 편집하지 않는다.

**사용자 결과**

운영자는 run phase와 delivery success를 혼동하지 않고 수락된 자동화 실행 하나를 태스크, 세션,
정확한 소스 검증, 리뷰, 전달 시도, 정리까지 추적할 수 있다.

**필수 증명**

- phase/outcome 분리, retry-attempt, cancellation race, timeout, unknown-state, restart reconciliation
  테스트
- 연결되지 않은 의도, 대기 중인 태스크, 활성 턴, 소스 게시 후 전달, 통합 시작, 이미 통합됨, 오래된
  버전, 중복 요청, 비정상 종료 복구 상황의 취소 테스트
- lease/worktree가 소유된 상태를 유지하고 run이 너무 일찍 cancelled가 되지 않음을 증명하는
  controlled-stop 및 cleanup failure 테스트
- delivery attempt가 있는 경우와 없는 경우의 active cancellation, planning-task CAS failure,
  post-cancellation no-redispatch 테스트
- conversation completion 및 PR open이 integration을 뜻하지 않음을 증명하는 테스트
- 변경 불가능한 delivery outcome 및 cleanup까지의 출처 round trip
- 원격으로 검증된 integration 또는 권위 있는 terminal failure로 끝나는 실제 run trace 한 번

<a id="p1-admin-automation-run-trace-and-guarded-controls"></a>
### P1: Admin 자동화 실행 추적 및 보호된 컨트롤

run reconciliation 후 시작하고 jcode
[보호된 Admin 제어 액션](../jcode/gap-matrix.md#p1-guarded-admin-control-actions)의 인증, CSRF,
version/CAS, idempotency, confirmation, actor audit 계약을 재사용한다.

**소유 경계**

- 공유 automation run read 투영
- 인증된 Admin list/detail/run-now/cancel handler 및 template
- `AutomationIntakeService` 위의 보호된 `RunAutomationNow` wrapper와 versioned
  `CancelAutomationRun` 명령

**계약**

- 트리거, 수신 처리 결과, 시도, 대기열 경과 시간, 현재 활동, 복구 이유, 태스크/세션, 정확한 소스
  검증, PR/리뷰, 전달 결과, 정리 딥 링크를 보여 준다.
- 알려진 모든 phase와 unknown phase를 안전하게 렌더링한다.
- run-now에 새로운 operator request ID를 부여하고 run-now/cancel을 해당 보호된 애플리케이션 명령을
  통해서만 구현한다.
- HTTP handler에서 task, run, SQLite, Git, worktree 또는 GitHub 상태를 직접 변경하지 않는다.

**사용자 결과**

운영자는 전체 검토 전달 추적을 살펴보고 Admin을 독립 자동화 권한으로 만들지 않으면서 감사 가능한
run-now 또는 cancellation 명령을 내릴 수 있다.

**필수 증명**

- exhaustive/unknown status 렌더링 및 link-integrity 테스트
- authorization, CSRF, stale-version, idempotency, actor-audit, cancellation-race 테스트
- Admin JSON/template 테스트 및 desktop/mobile Playwright capture
- 실제 감사된 run-now 또는 cancel trail 한 번

<a id="p2-read-only-node-snapshot-protocol-and-server"></a>
### P2: 읽기 전용 노드 스냅샷 프로토콜 및 서버

**소유 경계**

- 버전이 지정된 노드 식별자/기능/스냅샷 전송 DTO
- 기존 task/session/delivery 투영 위의 애플리케이션 query 서비스
- 기본적으로 비활성화된 별도 읽기 전용 HTTPS listener와 Rustls mutual TLS
- 응답 최소화 및 크기 제한 정책

**계약**

- Akra 노드 식별자, 프로토콜/제품 버전, 기능, 작업 공간 범위가 적용된 제한 스냅샷, 서버 관측 시각을
  제공한다.
- 운영자가 제공한 server certificate/key와 client-CA bundle을 요구하고 server certificate의 URI
  SAN에 node identity를 묶는다.
- client certificate의 SHA-256 fingerprint로 식별하는 운영자 소유 ACL을
  `(client_id, workspace_set)`의 유일한 authorization source로 삼고 등록 client는 최대 64개로
  제한한다.
- 60초 이내의 timestamp와 unique request ID를 요구한다. client fingerprint당 최대 256개 ID를
  2분 동안 유지하고 duplicate를 거부하며, 해당 client cache가 가득 차면 만료되지 않은 ID를
  제거하지 말고 새 요청을 거부한다.
- rotation을 위해 기존/신규 client CA가 겹치는 trust-bundle reload와 명시적인 revoked-certificate
  fingerprint 목록을 지원한다.
- 이 listener를 loopback Admin listener와 분리한다.
- task/session/delivery 데이터를 감독에 필요한 필드로 최소화하고 prompt body, secret, raw tool
  output, credential, 임의 filesystem path를 제외한다.
- 투영 전에 unknown version, unauthorized workspace, expired/revoked certificate,
  replayed/stale request ID, oversized request를 거부한다.
- 원격 변경 메서드를 노출하지 않는다.

**사용자 결과**

신뢰할 수 있는 원격 클라이언트는 로컬 Admin UI를 열거나 변경 권한을 얻지 않고 제한된 작업 공간
범위의 Akra 운영 스냅샷 하나를 읽을 수 있다.

**필수 증명**

- protocol-version, capability, bound, deterministic serialization 테스트
- 상호 TLS 식별, 만료, 폐기, CA 교체, 재생/오래된 요청 ID, 작업 공간 간 실패 테스트
- 만료되지 않은 항목을 제거하지 않는 client별 request-ID flood 및 cache-full fail-closed 테스트
- 최소화된 필드와 credential/prompt/output 비공개를 증명하는 응답 fixture
- 선택한 전송을 통한 실제 원격 읽기 한 번

<a id="p2-private-node-registry-and-read-client"></a>
### P2: 비공개 노드 레지스트리 및 읽기 클라이언트

node snapshot protocol이 merge된 후 시작한다.

**소유 경계**

- remote node registry 도메인 및 port
- OS-private/server-side credential 및 trust-store 어댑터
- enrollment, certificate rotation, revocation 서비스
- 제한된 mutual-TLS HTTPS snapshot client

**계약**

- model provider가 아니라 Akra node를 등록한다.
- 엔드포인트 DNS 이름, 예상 노드 식별자, 서버 CA, 클라이언트 인증서/키 참조, 지원 버전, 허용 작업
  공간 범위를 등록한다.
- HTTPS peer identity를 검증하고 redirect, DNS/SAN mismatch, certificate mismatch 또는 expiry,
  revoked identity, unsupported version, oversized response, cross-workspace data를 거부한다.
- 명시적인 overlap deadline을 두고 server/client trust를 교체하며 audit history를 삭제하지 않고 전체
  SHA-256 certificate fingerprint로 폐기한다.
- health, last success/failure, remote observation time, local receipt time, 명시적인 snapshot age를
  유지한다.
- 자격 증명을 브라우저 상태 밖에 두고 log, telemetry, API response, error message에서 redact한다.
- 읽기만 수행한다.

**사용자 결과**

Akra는 신뢰 자료를 브라우저에 저장하거나 provider/runtime relay를 추가하지 않고 원격 Akra 노드를
등록하고, 교체하고, 폐기하고, 상태를 확인할 수 있다.

**필수 증명**

- 2개 노드 enrollment, read, rotation, revocation, recovery 테스트
- certificate/host/version/redirect/size/workspace failure 테스트
- stale, timeout, clock-skew, bounded-cache 테스트
- storage, log, telemetry, API output 전체의 credential non-disclosure 검사

<a id="p2-remote-node-tui-and-admin-projection"></a>
### P2: 원격 노드 TUI 및 Admin 투영

protocol과 private registry가 merge된 후 시작한다.

**소유 경계**

- 공유 node-health 및 snapshot-age 애플리케이션 투영
- TUI 노드 선택기/상세 화면
- 서버 측 데이터만 사용하는 Admin node list/detail

**계약**

- node identity, version, capability, workspace, health, last success/failure, snapshot age를 보여
  준다.
- stale/cached 상태를 현재 상태인 것처럼 나타내지 않고 레이블로 표시한다.
- 제한된 프로토콜이 노출한 원격 task/session/delivery ID에 대한 안정적인 링크를 보존한다.
- node credential을 HTML, JSON, JavaScript 또는 브라우저 로컬 스토리지로 직렬화하지 않는다.
- 이 구현 단위에서는 원격 명령을 노출하지 않는다.

**사용자 결과**

운영자는 TUI 또는 Admin에서 어느 노드가 세션이나 전달을 소유하는지, 해당 뷰가 정상인지 오래된
것인지 볼 수 있고 모든 신뢰 자료는 서버 측에 남는다.

**필수 증명**

- 2개 노드 selection, stale/timeout/recovery, identity-link 테스트
- narrow/wide TUI snapshot 및 desktop/mobile Admin capture
- 브라우저 저장소·HTML·JSON·로그·텔레메트리 전반의 자격 증명 카나리 검사

원격 명령에는 이후 프로토콜, actor/audit 모델, 별도 리뷰가 필요하다. 이 구현 단위에는 포함되지
않는다.

## 비교 실험

이 실험은 차별점을 검증하지만 처음 두 P0 제품 수정의 차단 조건은 아니다.

### Golden 프로토콜 충실도 Trace

텍스트 단계, 추론, 계획, 턴 diff, 패치 delta, 명령 출력 delta, 권한, MCP 요청, 이미지, 사용량 상세를
포함하는 공통 접두부로 결정론적 fake app-server 시나리오를
구동한다. 상호 배타적인 terminal path를 최소한 normal completion, generic stream/error,
auth/quota failure, interrupt 시나리오로 나눈다. permission 및 elicitation은 accept, reject, cancel
분기로 나눈다. `CODEX_PATH`가 허용하는 경우 같은 시나리오를 Akra 및 이후 유지보수 ACP 경로로
전달한다. app-server, ACP, SDK, Canvas consumer, Akra reducer 경계에서 canonical JSON을 기록한다.
순서와 correlation identity를 포함하여 모든 필드를 preserved, transformed, dropped로 표시한다.

출시된 내장 브리지는 같은 fake app-server를 받을 수 없다. 경로가 같은 것처럼 가장하지 말고 검사된
fixture를 별도 계층으로 취급한다.

<a id="permission-and-secret-canary"></a>

### 권한 및 시크릿 Canary

프로토콜이 지원하는 경우 allow-once, allow-session, reject, cancel을 포함하여 광고된 결정 집합을
다양하게 바꾼 command, file, network, MCP 요청을 생성한다. 각 계층이 실제로 광고하는 것을 열거하고
지원되지 않거나 축약된 선택을 동등하다고 가정하지 말고 결과로 보존한다. 바깥쪽 OpenHands
confirmation off/on, ACP session mode, permission option order를 별도 차원으로 바꾼다. 요청이
운영자 UI에 도달하는지와 정확히 선택된 option ID를 기록한다. Akra에서는 현재 accept/decline 축약,
accept-for-session-only command request 거부, 검사할 수 없는 file-change approval 자동 거부, MCP
elicitation 자동 거부를 명시적으로 기록한다.
상위 환경, 등록된 environment secret, file secret, tool child environment, 저장된 이벤트에 고유
canary를 둔다. 환경 로그인 및 `CODEX_AUTH_JSON`을 별도 계층으로 두고 동일 공급자의 대화를 동시에
실행한다. 공급자 data-directory path/inode를 기록하고 auth refresh, config, lock 활동을 유발한다.
이후 브리지에서는 별도의 startup auth/config, prompt, app-server frame canary로 `APP_SERVER_LOGS`
off/on을 테스트한다. Akra API-key forwarding off 및 on과 비교한다.

### Crash, Resume 및 Drift 행렬

출시된 토폴로지에서는 initialization, turn acceptance, first delta, active tool, pending approval,
completion-before-client acknowledgement 시점에 Agent Server 또는 내장 코어 Zed 브리지를 종료한다.
이후 토폴로지에서는 같은 경계에서 Agent Server, 유지보수 Node 브리지, app-server를 각각 종료한다.
orphan process, duplicate turn, retained ID, history hash, visible notice, fresh-session fallback을
기록한다. 모호한 active turn은 자동으로 다시 제출하면 안 된다. 두 브리지에 대해 SDK
`ask_agent()`를 직접 호출하고 capability gating, error code, child/session side effect를 기록한다.

Python ACP 0.10.1과 0.11.x를 고정 및 다음 래퍼에 대해 테스트한다. 0.11.x는 Canvas의 출시 제약
밖의 negative test로 취급한다. 출시된 내장 Codex patch를 바꾸려면 Rust 브리지를 다시 빌드해야
하고 이후 브리지만 Codex 실행 파일을 교체할 수 있다. Akra와 이후 브리지에 unknown app-server
notification을 주입한 다음 Python/출시 경계에 unknown ACP `session/update`와 wire message를
주입한다. Akra의 classification test가 실패하는지 확인하고 적용 가능한 각 경계의 동작을 기록한다.

<a id="recommended-order"></a>
## 권장 순서

이 순서는 Canvas에서 도출한 작업만 다루며 jcode
[전역 순서](../jcode/gap-matrix.md#recommended-order)를 대체하지 않는다.

1. 기존의 서로 겹치지 않는 protocol/TUI lane에서 실시간 병렬 활동을 주장하기 전에 Protocol-Native
   Live Execution Rail을 완료한다. 그다음 기존 병렬 활동 소유 lane에서 Parallel Current Activity
   Envelope를 구현한다.
2. 타입 있는 activity/lifecycle 투영을 사용할 수 있게 된 후 Admin diorama를 진실하게 만든다.
3. 활성 validation-schema lane이 소유권을 해제한 후 별도의 Native Performance Evidence Contract를
   시작하고, 나중에 그 스키마를 바꾸지 않고 Canvas 프로필을 추가한다.
4. Automation Intake Core는 겹치지 않는 domain/application lane에서 진행할 수 있다. core 이후
   schedule 및 signed webhook intake를 별도 PR로 추가한다.
5. Automation Run Reconciliation이 검토 전달을 주장하기 전에 jcode 순서대로 Frozen-Source
   Validation Evidence, Critical Review Response Loop, Authoritative Delivery Outcomes를 완료한다.
   reconciliation 및 Guarded Admin Control Actions 이후에만 Admin run trace를 추가한다.
6. 공개적인 ACP 충실도 또는 성능 주장을 하기 전에 golden protocol, permission/secret,
   crash/resume 실험을 실행한다.
7. 로컬 전달 및 자동화 결과가 권위 있게 된 후에만 protocol/server, private registry/client,
   TUI/Admin projection PR 순서로 원격 지원을 추가한다.

## 성공 감사

이 비교는 이후 증거가 다음을 입증할 때에만 가치를 낸다.

- 모든 활성 병렬 lane이 제한된 현재 활동, 마지막 활동 경과 시간, lease generation, validation
  source, review/delivery state, cleanup truth를 노출한다.
- Admin 이동과 패킷이 기록된 sequence transition에 대응하고 idle snapshot은 의미상 안정적으로
  유지된다.
- 이벤트가 최대 한 번 수락되고 원시 페이로드 텍스트를 에이전트 권한으로 바꿀 수 없다.
- 트리거별 intake disposition이 재시작 후에도 유지되고 schedule, webhook, run-now 키가 충돌하지
  않는다.
- run state가 재시작 후에도 유지되고 trigger, task, session, exact-source validation, PR, delivery
  attempt, cleanup을 연결하며 권위 있는 outcome을 다시 정의하지 않고 참조한다.
- cancelled, skipped, timed-out, failed, unknown, successful state를 빠짐없이 렌더링한다.
- remote snapshot server가 데이터를 최소화하고 원격 자격 증명이 브라우저 스토리지에 절대 들어가지
  않으며 모든 1세대 node protocol/client/UI 메서드가 읽기 전용이다.
- 토폴로지 직관이 아니라 버전이 지정된 artifact가 모든 성능 또는 충실도 비교를 뒷받침한다.
- 검토된 Codex 전달 목표를 ACP runtime, provider registry, browser IDE clone 또는 generic automation
  marketplace가 밀어내지 않는다.
