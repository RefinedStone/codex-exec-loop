# 게임발전국 기반 AKRA Admin 고도화 기획서 v1.0

## 0. 기획 방향 요약

이번 고도화의 핵심은 **기존 AKRA 병렬 작업 운영 상태를 “게임발전국” 같은 오피스 경영 시뮬레이션 화면으로 시각화하는 Admin Web**입니다.

기존 TUI의 `Supersession Control Tower`가 운영자용 텍스트 관제 화면이라면, Admin Web은 다음 역할을 맡습니다.

```text
TUI
= 개발자/운영자가 직접 명령을 내리는 실시간 터미널 관제면

Admin Web
= 전체 병렬 작업 상태를 시각적으로 파악하고,
  agent / worktree pool / distributor / task / event를
  게임형 오피스 운영 화면으로 확인하는 관리 대시보드
```

현재 프로젝트의 Supersession은 `:parallel`을 통해 readiness, pool, roster, selected detail, distributor head, queue state를 보여주는 구조이며, queue work는 세 개의 local `akra` worktree slot 중 하나를 lease하고, completion 이후 official planning refresh를 거쳐 distributor 대상으로 들어갑니다. 이후 distributor는 source branch push, PR automation, `prerelease` integration, slot cleanup을 직렬로 처리합니다.

따라서 Admin Web의 게임화는 **실제 AKRA 도메인 상태를 왜곡하지 않고, 같은 상태를 픽셀 오피스 경영 화면으로 표현하는 것**이 목적입니다.

---

# 1. 프로젝트 기준 제약 조건

## 1.1 반드시 지켜야 할 현재 프로젝트 구조

현재 레포 기준으로 핵심 구조는 다음과 같습니다.

```text
adapter -> application -> domain
```

* `src/adapter/inbound/tui`: TUI 입력, overlay state, rendering
* `src/application/service/parallel_mode`: supersession, pool, distributor, turn boundary
* `src/domain`: UI-neutral model과 invariant
* application service는 orchestration과 port call 담당
* adapter는 transport, terminal rendering, DB row mapping, filesystem mapping 담당

Admin Web을 붙일 때도 이 구조를 유지해야 합니다.

```text
잘못된 방향:
Admin Frontend가 TUI projection 문자열을 그대로 파싱한다.
Admin API가 SQLite table을 직접 읽고 도메인 규칙을 재구성한다.
Frontend에서 slot 상태, distributor 상태를 임의 계산한다.

올바른 방향:
Admin API는 application service / domain snapshot을 사용한다.
Frontend는 받은 DTO를 시각화만 한다.
게임 표현은 view model layer에서만 처리한다.
```

## 1.2 Admin Web에서 사용해야 할 source of truth

Admin Web의 데이터 원천은 다음 순서로 정리합니다.

```text
ParallelModeService
  ├─ readiness snapshot
  ├─ supervisor snapshot
  │   ├─ pool board
  │   ├─ agent roster
  │   ├─ selected detail
  │   └─ distributor snapshot
  └─ runtime event snapshot
```

`ParallelModeService`에는 read-only supervisor snapshot을 만드는 `build_supervisor_snapshot`과, pool worktree/cleanup side effect가 있을 수 있는 `reconcile_supervisor_snapshot`이 분리되어 있습니다. Admin Web의 자동 새로고침은 반드시 read-only snapshot을 사용해야 하고, reconcile은 명시적 운영 버튼에서만 호출해야 합니다.

---

# 2. 제품 콘셉트

## 2.1 제품명

```text
게임발전국
AKRA Admin Control Center
```

## 2.2 핵심 은유

```text
AKRA 프로젝트
= 자동화된 개발 사무국

Agent
= 개발 요원 / 직원 캐릭터

Task
= 작업 의뢰 / 퀘스트 / 업무 카드

Worktree Pool
= 작업 좌석 / 개발 장비 / 슬롯 서버

Slot Lease
= 요원이 점유한 작업 자리

Distributor
= 작업 배분 담당자 / 배포 관리자

Runtime Event
= 사무국 실시간 활동 로그

Queue
= 대기 중인 작업 서류함

Official Refresh
= 작업 검수 / 공식 승인

Distributor Pipeline
= 검토 → 게이트 체크 → PR/Publish → 통합 → 정리
```

## 2.3 화면 톤

기존에 생성했던 “일반적인 SaaS Admin”보다 더 게임 쪽으로 이동합니다.

목표 스타일:

```text
오피스 경영 시뮬레이션 + Admin Dashboard
```

구체적으로는:

* 중앙에 **isometric pixel office board**
* 좌측에는 실제 Admin sidebar
* 상단에는 KPI 카드
* 오른쪽에는 pool/queue 상태 오버레이
* 하단에는 작업 상세, 이벤트 로그, 운영 지표, 배포 파이프라인
* 캐릭터 위에는 `작업중`, `완료`, `분배중`, `대기중`, `테스트 통과`, `부하 높음` 같은 말풍선
* 단, 실제 조작과 상태는 AKRA 도메인에 맞춰야 함

---

# 3. 용어 매핑표

| 게임발전국 표현      | 실제 AKRA 개념                      | 데이터 원천                              | 주의사항                                  |
| ------------- | ------------------------------- | ----------------------------------- | ------------------------------------- |
| 본부            | Admin Dashboard 전체              | Dashboard route                     | 단순 장식 아님, 실제 관제 화면                    |
| 요원            | Agent / Roster Entry            | `ParallelModeAgentRosterSnapshot`   | agent id를 캐릭터로 표현                     |
| 작업 / 퀘스트      | Planning Task / Queue Item      | planning queue, distributor queue   | “퀘스트”는 UI 표현, API 명칭은 task 유지         |
| 워크트리 풀        | Pool Board                      | `ParallelModePoolBoardSnapshot`     | fantasy map으로 표현하지 않음                 |
| 풀 슬롯          | Pool Slot                       | `ParallelModePoolSlotSnapshot`      | slot-1, slot-2, slot-3 고정 가능          |
| 자리 점유         | Slot Lease                      | `ParallelModeSlotLeaseSnapshot`     | lease 상태를 직접 표현                       |
| 분배관 / 분배기     | Distributor                     | `ParallelModeDistributorSnapshot`   | distributor는 serialized delivery lane |
| 배포 파이프라인      | Distributor delivery pipeline   | queue state + orchestrator status   | 실제 PR/push/integration 상태와 연결         |
| 실시간 이벤트       | Runtime Events                  | `ParallelModeRuntimeEventsSnapshot` | append-only audit 성격                  |
| 공식 승인         | official refresh / commit_ready | session detail state                | XP 보상처럼 과장 금지                         |
| 국가 성과 / 길드 성과 | Derived metrics                 | snapshot 기반 계산                      | MVP에서는 저장형 XP 금지                      |

---

# 4. 화면 정보 구조

## 4.1 전체 레이아웃

```text
┌────────────────────────────────────────────────────────────┐
│ Top Bar: 서비스명, epoch, 알림, 사용자                     │
├──────────────┬─────────────────────────────────────────────┤
│ Left Sidebar │ KPI Cards                                   │
│              ├─────────────────────────────────────────────┤
│              │ Isometric Office Board                      │
│              │ - Pool Slots                                │
│              │ - Agents                                    │
│              │ - Tasks                                     │
│              │ - Distributor                               │
│              │ - Event Log Desk                            │
│              ├─────────────────────────────────────────────┤
│              │ Bottom Cards                                │
│              │ - 작업 상세                                 │
│              │ - 실시간 이벤트 로그                        │
│              │ - 운영 지표                                 │
│              │ - 배포 파이프라인                           │
└──────────────┴─────────────────────────────────────────────┘
```

## 4.2 좌측 Sidebar

필수 메뉴:

```text
대시보드
워크트리 풀
요원
작업
배포 파이프라인
이벤트 로그
메트릭스
설정
```

선택 메뉴:

```text
길드 성과
정책
연동
알림
```

사이드바 하단 상태:

```text
AKRA v{version}
모든 시스템 정상
또는
주의 필요: readiness blocked
```

## 4.3 Top KPI 카드

상단 KPI는 한눈에 현재 운영 상태를 보여주는 영역입니다.

필수 카드:

| 카드     | 표시값                   | 계산 기준                                           |
| ------ | --------------------- | ----------------------------------------------- |
| 총 작업   | `128`                 | 최근 24h 또는 전체 완료 task                            |
| 성공률    | `96.7%`               | tests pass / total 또는 completion success        |
| 오늘 처리량 | `842`                 | runtime event / completed task count            |
| 활성 요원  | `18 / 24`             | active agents / known agents                    |
| 풀 슬롯   | `3 / 3`               | configured size / available slots               |
| 대기 작업  | `2`                   | distributor queue depth 또는 planning queue depth |
| 준비도    | `준비 완료`               | readiness snapshot                              |
| 분배기 상태 | `대기 중`, `처리 중`, `차단됨` | distributor head / barrier                      |

MVP에서는 모든 값을 실데이터로 못 채워도 됩니다. 단, **어떤 값이 mock인지, 어떤 값이 snapshot 기반인지 명확히 분리**해야 합니다.

---

# 5. 메인 Hero: Isometric Office Board

## 5.1 목적

중앙 isometric office는 단순 장식이 아닙니다.

이 영역의 역할은 다음입니다.

```text
1. pool slot 상태를 시각적으로 파악
2. agent가 어떤 작업을 하고 있는지 파악
3. distributor가 배포 대기열을 잡고 있는지 파악
4. event log가 최근 어떤 사건을 보여주는지 파악
5. 과부하, blocked, idle, ready 상태를 직관적으로 파악
```

## 5.2 Office Board 구역

### A. 풀 슬롯 구역

위치:

```text
좌상단 또는 서버랙 영역
```

표현:

```text
서버랙 3개 또는 작업 좌석 3개
slot-1 / slot-2 / slot-3 라벨
```

상태 표현:

| 실제 상태           | 보드 표현           | 색상       |
| --------------- | --------------- | -------- |
| Idle            | 비어 있는 슬롯 / “여유” | 초록 또는 파랑 |
| Leased          | 예약됨 / “점유됨”     | 노랑       |
| Running         | 작업중 / 캐릭터 또는 불빛 | 초록       |
| AwaitingCleanup | 정리중 / 빗자루 아이콘   | 주황       |
| Blocked         | 막힘 / 경고 표시      | 빨강       |
| Missing         | 사라짐 / 회색 점멸     | 회색       |
| Unavailable     | 사용 불가 / 잠금      | 회색+빨강    |

현재 domain의 pool slot state는 `Idle`, `Leased`, `Running`, `AwaitingCleanup`, `Blocked`, `Missing`, `Unavailable`로 정의되어 있으므로 이 enum을 UI 상태의 기준으로 써야 합니다.

### B. 요원 구역

위치:

```text
중앙 좌측 책상 그룹
```

표현:

```text
agent-1, agent-2, agent-3 캐릭터
각 캐릭터 책상 위 PC, 모니터, 말풍선
```

말풍선 규칙:

| agent 상태          | 말풍선   |
| ----------------- | ----- |
| running           | 작업중   |
| reported_complete | 보고 완료 |
| commit_ready      | 공식 승인 |
| failed            | 실패    |
| blocked           | 차단됨   |
| idle              | 대기중   |
| offline           | 오프라인  |

### C. 작업 구역

위치:

```text
중앙 하단 작업 테이블
```

표현:

```text
업무 카드, 문서 더미, 체크 아이콘, 테스트 통과 효과
```

상태:

```text
대기중
작업중
테스트 통과
공식 승인
배포 준비
완료
```

### D. 분배기 구역

위치:

```text
우상단 또는 별도 관리자 책상
```

표현:

```text
초록 모자 캐릭터 / 분배관 / 관리자 NPC
```

연결 데이터:

```text
distributor.head_summary
distributor.queue_depth()
distributor.orchestrator_status.barrier_state
distributor.orchestrator_status.integration_worktree_readiness
```

Distributor queue state는 `Queued`, `Pushing`, `PrPending`, `MergePending`, `Integrating`, `Cleaning`, `Done`, `Blocked`, `Failed` 등을 가집니다. 따라서 pipeline UI도 이 상태를 기준으로 표시해야 합니다.

### E. 이벤트 로그 구역

위치:

```text
우하단 책상 또는 게시판
```

표현:

```text
실시간 로그 책상
이벤트 말풍선
시계 / 알림 / 종 아이콘
```

연결 데이터:

```text
runtime event feed
```

Runtime event는 append-only orchestration log이고, projection write 순서와 planning revision을 보여주는 feed입니다.

---

# 6. 하단 카드 구성

## 6.1 작업 상세 카드

제목:

```text
작업 상세
```

필드:

```text
작업 ID
작업명
담당 요원
슬롯
브랜치
상태
진행률
검증 결과
아티팩트
최근 업데이트 시각
트레일
```

예시:

```text
TASK-8731
UI Pack: 타임라인 컴포넌트 구현

요원: agent-1
슬롯: slot-1
브랜치: feature/mud-timeline-ui
상태: running
진행률: 72%
검증: cargo test passed
아티팩트: ui.png, timeline.yaml, docs.md
트레일: assigned → running → reported → official → delivery
```

주의:

* `진행률`은 실제 progress가 없으면 lifecycle 기반으로 파생합니다.
* 테스트 개수는 실제 데이터가 없으면 표시하지 않습니다.
* mock 수치를 실데이터처럼 보이게 하지 않습니다.

## 6.2 실시간 이벤트 로그 카드

제목:

```text
실시간 이벤트 로그
```

컬럼:

```text
시간
이벤트
대상
상태
revision
요약
```

예시:

```text
09:14:22 slot_lease_upsert      slot-1   running   rev 31
09:14:18 session_detail_upsert  agent-1  official  rev 31
09:14:15 distributor_queue      head     queued    rev 31
09:14:07 worktree_status        slot-3   blocked   rev 31
```

필수 규칙:

```text
최신순 정렬
기본 20개
동일 sequence 중복 표시 금지
이벤트 종류별 아이콘 고정
```

## 6.3 길드 성과 / 운영 지표 카드

제목:

```text
길드 성과 / 운영 지표
```

MVP에서 허용되는 지표:

```text
풀 활용률
테스트 성공률
평균 큐 깊이
에러율
활성 요원 수
대기 작업 수
blocked slot 수
```

MVP에서 금지:

```text
저장형 XP
영구 레벨
영구 랭킹
누적 코인
실제 DB에 없는 업적
```

대신 snapshot 기반 파생 배지를 사용합니다.

| 배지     | 조건                              |
| ------ | ------------------------------- |
| 풀 관리자  | blocked slot = 0                |
| 테스트 장인 | 최근 테스트 성공률 95% 이상               |
| 분배 안정  | distributor barrier idle        |
| 과부하 경보 | 특정 slot utilization 90% 이상      |
| 정리 필요  | awaiting cleanup > 0            |
| 복구 필요  | blocked/missing/unavailable > 0 |

## 6.4 배포 파이프라인 카드

제목:

```text
배포 파이프라인
```

단계:

```text
빌드
테스트
검증
승인
배포
정리
```

실제 distributor 상태 매핑:

| Queue State  | UI 단계    |
| ------------ | -------- |
| Queued       | 대기       |
| Pushing      | 브랜치 Push |
| PrPending    | PR 대기    |
| MergePending | Merge 대기 |
| Integrating  | 통합 중     |
| Cleaning     | 슬롯 정리    |
| Done         | 완료       |
| Blocked      | 차단       |
| Failed       | 실패       |

---

# 7. API 설계

Admin Web이 별도 프론트라면, 다음 API를 기준으로 구현합니다.

## 7.1 Dashboard Snapshot API

```http
GET /api/admin/akra/dashboard
```

목적:

```text
대시보드 전체 초기 로딩
```

응답 DTO:

```ts
type AkraAdminDashboardResponse = {
  workspace: {
    path: string;
    branch: string | null;
    mode: "normal" | "parallel";
    readiness: "ready" | "blocked" | "degraded" | "unknown";
    topNotice?: string;
  };

  kpis: {
    totalTasks: number | null;
    successRate: number | null;
    todayThroughput: number | null;
    activeAgents: number;
    totalAgents: number;
    poolConfiguredSize: number;
    poolIdle: number;
    poolRunning: number;
    poolBlocked: number;
    queueDepth: number;
    distributorState: string;
  };

  pool: PoolBoardView;
  agents: AgentRosterView;
  selectedTask: SelectedTaskView | null;
  distributor: DistributorView;
  events: RuntimeEventView[];
  metrics: GuildMetricsView;
  generatedAt: string;
};
```

## 7.2 Pool API

```http
GET /api/admin/akra/pool
```

응답:

```ts
type PoolBoardView = {
  configuredSize: number;
  reconcileStatus: string;
  exhausted: boolean;
  summary: {
    idle: number;
    leased: number;
    running: number;
    cleanup: number;
    blocked: number;
    missing: number;
    unavailable: number;
  };
  slots: PoolSlotView[];
};

type PoolSlotView = {
  slotId: string;
  state:
    | "idle"
    | "leased"
    | "running"
    | "awaiting_cleanup"
    | "blocked"
    | "missing"
    | "unavailable";
  branchName: string;
  worktreeLabel: string;
  ownerLabel: string;
  ownerAgentId?: string;
  taskId?: string;
  note: string;
  severity: "normal" | "info" | "warning" | "danger" | "muted";
};
```

## 7.3 Agent API

```http
GET /api/admin/akra/agents
```

응답:

```ts
type AgentRosterView = {
  activeCount: number;
  entries: AgentView[];
};

type AgentView = {
  agentId: string;
  displayName: string;
  classLabel: "Artificer" | "Scribe" | "Ranger" | "Guardian" | "Seer" | "Runner";
  slotId?: string;
  taskTitle?: string;
  branchName?: string;
  lifecycleState: string;
  progressLabel: string;
  durationLabel: string;
  latestSummary: string;
  status: "running" | "idle" | "blocked" | "offline" | "cleanup";
  overload: boolean;
};
```

## 7.4 Distributor API

```http
GET /api/admin/akra/distributor
```

응답:

```ts
type DistributorView = {
  headSummary: string;
  note: string;
  queueDepth: number;
  barrierState: string;
  blockedReason?: string;
  integrationWorktreeReadiness: string;
  heldQueueCount: number;
  conflictFiles: string[];
  queueItems: DistributorQueueItemView[];
  pipeline: DistributorPipelineStep[];
};

type DistributorQueueItemView = {
  sourceAgent: string;
  taskTitle: string;
  queueState: string;
  branchName: string;
  commitShortSha: string;
  integrationNote: string;
};

type DistributorPipelineStep = {
  key: "review" | "gate_check" | "push" | "pr" | "merge" | "cleanup" | "done";
  label: string;
  state: "done" | "active" | "waiting" | "blocked" | "failed";
};
```

## 7.5 Event API

```http
GET /api/admin/akra/events?limit=50
GET /api/admin/akra/events?afterSequence=142&limit=50
```

응답:

```ts
type RuntimeEventView = {
  sequence: number;
  eventKind: string;
  projectionKind: string;
  projectionKey: string;
  observedPlanningRevision: number;
  summary: string;
  recordedAt: string;
  icon: string;
  severity: "info" | "success" | "warning" | "danger" | "muted";
};
```

Runtime event log request는 이미 limit과 projection filter를 가지는 port 계약으로 정의되어 있으므로 Admin API는 이 계약을 활용하는 것이 맞습니다.

---

# 8. Backend 구현 방침

## 8.1 권장 모듈 구조

현재 레포에 Admin API를 직접 추가한다면 다음 구조를 권장합니다.

```text
src/adapter/inbound/web_admin/
  mod.rs
  routes.rs
  dto.rs
  mapper.rs
  handlers/
    dashboard.rs
    pool.rs
    agents.rs
    distributor.rs
    events.rs

src/application/service/admin_dashboard/
  mod.rs
  dashboard_query.rs
  metrics_projection.rs
```

단, Admin Web이 별도 서버라면 Rust repo에는 API만 두고, 프론트 repo에서는 위 DTO를 소비합니다.

## 8.2 서비스 호출 규칙

자동 새로고침:

```rust
parallel_mode_service.build_supervisor_snapshot(...)
```

명시적 운영 버튼:

```rust
parallel_mode_service.reconcile_supervisor_snapshot(...)
```

금지:

```rust
자동 polling 때 reconcile_supervisor_snapshot 호출
자동 polling 때 reset_pool_on_parallel_enable 호출
Frontend에서 pool cleanup 직접 호출
Frontend에서 distributor queue를 임의 진행
```

## 8.3 Read-only vs Mutating Endpoint 분리

Read-only:

```http
GET /api/admin/akra/dashboard
GET /api/admin/akra/pool
GET /api/admin/akra/agents
GET /api/admin/akra/distributor
GET /api/admin/akra/events
```

Mutating:

```http
POST /api/admin/akra/parallel/refresh
POST /api/admin/akra/pool/reconcile
POST /api/admin/akra/distributor/tick
```

MVP에서는 mutating endpoint를 넣지 않는 것을 권장합니다.

이유:

```text
1. 기존 TUI와 운영 권한 충돌 방지
2. Admin Web을 먼저 read-only 관제면으로 안정화
3. pool reconcile, distributor tick은 side effect가 있음
4. 추후 권한/감사 로그/confirm modal이 붙은 뒤 활성화
```

---

# 9. Frontend 구현 방침

## 9.1 컴포넌트 구조

```text
src/features/akra-admin/
  pages/
    AkraDashboardPage.tsx

  components/
    AppShell/
      Sidebar.tsx
      TopBar.tsx
      KpiCard.tsx

    OfficeBoard/
      OfficeBoard.tsx
      OfficeBackground.tsx
      PoolSlotZone.tsx
      AgentDeskZone.tsx
      TaskDeskZone.tsx
      DistributorZone.tsx
      EventDeskZone.tsx
      FloatingBadge.tsx
      SpeechBubble.tsx

    Panels/
      PoolSummaryPanel.tsx
      TaskDetailPanel.tsx
      AgentRosterPanel.tsx
      EventLogPanel.tsx
      GuildMetricsPanel.tsx
      DistributorPipelinePanel.tsx

  api/
    akraAdminApi.ts

  model/
    types.ts
    statusMapping.ts
    viewModel.ts

  styles/
    tokens.css
    office-board.css
    dashboard-layout.css
```

## 9.2 Isometric Office 구현 방식

MVP 권장:

```text
DOM + CSS absolute positioning
```

구현 방식:

```text
1. office background 이미지를 한 장 둔다.
2. 각 zone을 absolute 좌표로 배치한다.
3. agent, slot, distributor, event log 아이콘도 sprite/image로 배치한다.
4. 상태 bubble은 React component로 얹는다.
```

비권장:

```text
처음부터 Canvas/WebGL로 구현
```

이유:

```text
1. Admin Web 유지보수 어려움
2. 접근성 낮음
3. click/hover/debug 복잡도 증가
4. QA에서 DOM query 어려움
```

## 9.3 좌표 시스템

```ts
type BoardAnchor = {
  key: string;
  x: number; // percent
  y: number; // percent
  zIndex: number;
};
```

예시:

```ts
const BOARD_ANCHORS = {
  poolSlot1: { x: 18, y: 38, zIndex: 20 },
  poolSlot2: { x: 24, y: 39, zIndex: 20 },
  poolSlot3: { x: 30, y: 37, zIndex: 20 },

  agent1: { x: 18, y: 68, zIndex: 30 },
  agent2: { x: 31, y: 65, zIndex: 30 },

  taskDesk: { x: 52, y: 70, zIndex: 30 },
  distributor: { x: 71, y: 42, zIndex: 30 },
  eventLog: { x: 84, y: 70, zIndex: 30 },
};
```

## 9.4 상태 색상

```text
success green: running, ready, pass, done
info blue: idle, waiting, queued
warning orange: leased, cleanup, high load
danger red: blocked, failed, dirty
muted gray: offline, missing, unavailable
gold: reward, rank, completion highlight
```

## 9.5 Character Class 규칙

초기에는 실제 scheduling에 영향을 주지 않는 **표시용 class**로만 둡니다.

```ts
function deriveAgentClass(taskTitle: string): AgentClass {
  const title = taskTitle.toLowerCase();

  if (title.includes("doc") || title.includes("readme")) return "Scribe";
  if (title.includes("test") || title.includes("validation")) return "Guardian";
  if (title.includes("branch") || title.includes("merge")) return "Ranger";
  if (title.includes("ui") || title.includes("timeline")) return "Artificer";
  if (title.includes("analysis")) return "Seer";

  return "Runner";
}
```

주의:

```text
agent specialization by task category는 추후 확장 항목이다.
MVP에서 class는 UI 표시용이다.
class가 실제 task 배정 로직에 영향을 주면 안 된다.
```

---

# 10. 게임화 설계

## 10.1 MVP에서 허용되는 게임화

허용:

```text
픽셀 오피스 배경
캐릭터 아바타
말풍선
상태 배지
파생 업적
일시적 코인/별 애니메이션
레벨 표시 mock 또는 snapshot-derived level
```

주의:

```text
코인/XP는 저장하지 않는다.
영구 랭킹은 만들지 않는다.
실제 성과와 무관한 수치를 표시하지 않는다.
```

## 10.2 추후 확장 가능한 게임화

Phase 4 이후:

```text
achievement projection table
score event replay
agent별 누적 성공률
pool stewardship 점수
blocked recovery 점수
daily streak
weekly guild rank
```

이때는 반드시 DB schema, migration, event idempotency, replay policy가 필요합니다.

---

# 11. 상태 매핑 상세

## 11.1 Pool Slot 상태

| Domain State     | Admin Label | Board Bubble | Severity | 클릭 시 상세             |
| ---------------- | ----------- | ------------ | -------- | ------------------- |
| idle             | 여유          | 대기중          | info     | 사용 가능 slot          |
| leased           | 점유됨         | 배정됨          | warning  | owner/task 표시       |
| running          | 작업중         | 작업중          | success  | agent/task/detail   |
| awaiting_cleanup | 정리 대기       | 정리중          | warning  | cleanup reason      |
| blocked          | 차단됨         | 막힘           | danger   | blocked reason      |
| missing          | 누락          | 누락           | danger   | worktree missing    |
| unavailable      | 사용 불가       | 사용 불가        | muted    | runtime unavailable |

## 11.2 Agent 상태

| Source State      | Admin Label | Bubble | Icon   |
| ----------------- | ----------- | ------ | ------ |
| assigned          | 배정됨         | 배정됨    | 문서     |
| running           | 작업중         | 작업중    | 키보드    |
| reported_complete | 보고 완료       | 보고 완료  | 체크     |
| ledger_refreshing | 검수 중        | 검수 중   | 돋보기    |
| commit_ready      | 공식 승인       | 승인됨    | 도장     |
| merge_queued      | 배포 대기       | 배포 준비  | 박스     |
| pushing           | Push 중      | 업로드 중  | 화살표    |
| pr_pending        | PR 대기       | PR 대기  | GitHub |
| merge_pending     | Merge 대기    | 통합 대기  | 브랜치    |
| integrating       | 통합 중        | 통합 중   | 렌치     |
| failed            | 실패          | 실패     | 경고     |
| blocked           | 차단          | 차단     | 금지     |

## 11.3 Distributor 상태

| Queue State   | Admin Label | Pipeline Step |
| ------------- | ----------- | ------------- |
| queued        | 대기열 등록      | review        |
| pushing       | 브랜치 Push    | push          |
| pr pending    | PR 대기       | pr            |
| merge pending | Merge 대기    | merge         |
| integrating   | 통합 중        | integration   |
| cleaning      | 정리 중        | cleanup       |
| done          | 완료          | done          |
| blocked       | 차단          | blocked       |
| failed        | 실패          | failed        |

---

# 12. 사용자 인터랙션

## 12.1 Board 클릭

### slot 클릭

동작:

```text
1. slot card 선택
2. 우측 또는 하단 상세 패널에 slot detail 표시
3. 연결된 agent/task가 있으면 강조
```

필수 표시:

```text
slot id
state
branch
worktree label
owner
task
blocked reason
cleanup state
```

### agent 클릭

동작:

```text
1. agent desk 강조
2. 작업 상세 패널 갱신
3. slot도 같이 highlight
```

### distributor 클릭

동작:

```text
1. 배포 파이프라인 패널로 scroll/focus
2. queue head 상세 표시
```

### event 클릭

동작:

```text
1. event detail drawer 열기
2. projection kind/key 표시
3. 관련 slot/agent/task가 있으면 highlight
```

## 12.2 Hover

Hover tooltip은 짧게 유지합니다.

예시:

```text
slot-3
상태: blocked
원인: dirty worktree
다음 조치: TUI에서 pool reconcile 또는 수동 정리 필요
```

## 12.3 자동 새로고침

MVP:

```text
10초 polling
```

추후:

```text
SSE 또는 WebSocket
```

Polling 규칙:

```text
1. dashboard snapshot은 10초마다 갱신
2. events는 afterSequence 기반으로 incremental fetch
3. 실패 시 3회까지 exponential backoff
4. stale 상태는 UI 상단에 표시
```

---

# 13. 에러 / 예외 상태

## 13.1 Readiness blocked

표시:

```text
상단 KPI: 준비도 = 차단됨
Office board 전체에 어두운 overlay
문구: readiness blocker를 먼저 해결해야 합니다.
```

상세:

```text
capability list
top alert
next action
```

## 13.2 Pool exhausted

표시:

```text
풀 슬롯 KPI 빨간색
slot 영역에 “용량 부족”
```

문구:

```text
사용 가능한 worktree slot이 없습니다.
cleanup 또는 distributor 진행을 확인하세요.
```

## 13.3 Dirty worktree / blocked slot

표시:

```text
slot card 빨간 테두리
캐릭터 위 불꽃 또는 경고 아이콘
```

문구:

```text
worktree dirty
lease held but unusable
```

## 13.4 Event feed unavailable

표시:

```text
실시간 이벤트 로그를 불러올 수 없습니다.
최근 supervisor snapshot만 표시 중입니다.
```

## 13.5 Admin API stale

표시:

```text
마지막 갱신: 09:15:02
현재 연결 상태: 지연됨
```

---

# 14. 권한 / 안전 정책

## 14.1 MVP 권한

MVP는 read-only 권한으로 시작합니다.

```text
Dashboard 조회
Pool 조회
Agent 조회
Task 조회
Distributor 조회
Event 조회
```

## 14.2 운영 액션 권한

추후 운영 버튼을 넣을 경우 권한을 분리합니다.

```text
admin:read
admin:operate
admin:dangerous
```

예시:

| 액션                | 권한        | Confirm 필요 |
| ----------------- | --------- | ---------- |
| dashboard refresh | read      | 없음         |
| pool reconcile    | operate   | 있음         |
| distributor tick  | operate   | 있음         |
| parallel off      | dangerous | 있음         |
| pool reset        | dangerous | 강한 confirm |

---

# 15. 기획 산출물

작업자에게 전달할 산출물은 다음과 같습니다.

```text
1. Admin IA 문서
2. Dashboard wireframe
3. Isometric board zone map
4. Component spec
5. API DTO spec
6. Status mapping table
7. Error state spec
8. QA checklist
9. Release checklist
```

---

# 16. 구현 단계

## Phase 0. 디자인/기술 준비

목표:

```text
게임발전국 디자인 토큰과 Admin layout 확정
```

작업:

```text
- 디자인 시안 확정
- 색상 토큰 정의
- sprite/icon 스타일 확정
- office board 구역 좌표 확정
- Admin API DTO 확정
- mock data 작성
```

완료 기준:

```text
- Figma 또는 HTML prototype 완성
- 개발자가 데이터 바인딩 없이 화면을 구현할 수 있음
- 모든 상태 색상과 문구 확정
```

## Phase 1. Read-only Dashboard MVP

목표:

```text
실제 AKRA snapshot을 Admin Web에서 시각화
```

작업:

```text
- GET /api/admin/akra/dashboard 구현
- pool/agent/distributor/event DTO mapper 구현
- dashboard shell 구현
- KPI row 구현
- office board static 배치 구현
- slot/agent/task/distributor 상태 바인딩
- event log 카드 구현
```

완료 기준:

```text
- 실제 supervisor snapshot이 화면에 표시됨
- slot 상태가 정확히 표시됨
- agent 상태가 정확히 표시됨
- distributor queue depth가 정확히 표시됨
- event log가 최신순 표시됨
- 자동 새로고침 가능
```

## Phase 2. Drilldown / Interaction

목표:

```text
보드에서 slot/agent/task/event를 클릭해 상세 확인
```

작업:

```text
- slot click highlight
- agent click highlight
- task detail drawer
- event detail drawer
- distributor pipeline detail
- filter: all/running/blocked/idle
```

완료 기준:

```text
- 클릭한 대상과 상세 패널이 일치
- 잘못된 agent/slot 연결 없음
- event 클릭 시 projection key 표시
```

## Phase 3. Safe Operations

목표:

```text
Admin Web에서 제한적 운영 액션 지원
```

작업:

```text
- 수동 refresh
- pool reconcile
- distributor tick
- confirm modal
- audit log
- 권한 체크
```

완료 기준:

```text
- 자동 polling은 side effect 없음
- 운영 버튼은 confirm 후 실행
- 실행 결과가 event log에 반영
```

## Phase 4. Persistent Achievement

목표:

```text
실제 저장형 성과/점수 시스템
```

작업:

```text
- achievement projection 설계
- schema migration
- event replay
- 중복 반영 방지
- weekly guild score
- agent-specific stats
```

완료 기준:

```text
- score가 event 기반으로 재현 가능
- reset/restart 후에도 일관성 유지
- 동일 event 중복 처리 없음
```

---

# 17. 작업 체크리스트

## 17.1 Product 체크리스트

```text
[ ] Admin Web의 목적이 read-only 운영 관제인지, 운영 액션 포함인지 확정했다.
[ ] MVP에서 XP/코인/영구 레벨을 저장하지 않기로 합의했다.
[ ] “게임발전국” 용어와 실제 AKRA 도메인 용어의 매핑표를 확정했다.
[ ] fantasy map/realm map 표현을 제거하고 office management 표현으로 통일했다.
[ ] slot-1/slot-2/slot-3가 worktree pool slot이라는 점을 명확히 했다.
[ ] distributor가 “배포 관리자/분배기” 역할이라는 점을 명확히 했다.
[ ] queue depth가 planning queue인지 distributor queue인지 화면별로 명확히 표시했다.
[ ] readiness blocked일 때의 화면 문구를 정의했다.
[ ] blocked slot일 때 운영자가 무엇을 해야 하는지 문구를 정의했다.
```

## 17.2 Backend 체크리스트

```text
[ ] Admin API가 TUI projection 문자열에 의존하지 않는다.
[ ] Admin API가 domain/application snapshot을 사용한다.
[ ] 자동 dashboard API는 read-only `build_supervisor_snapshot` 계열만 호출한다.
[ ] 자동 polling에서 `reconcile_supervisor_snapshot`을 호출하지 않는다.
[ ] pool reset, distributor tick 등 side effect API는 MVP에서 제외하거나 confirm/권한을 붙였다.
[ ] PoolSlotState enum 전체를 DTO에 매핑했다.
[ ] DistributorQueueItemState 전체를 DTO에 매핑했다.
[ ] RuntimeEvent snapshot을 limit 기반으로 조회한다.
[ ] Event API는 sequence 기반 incremental fetch를 지원한다.
[ ] DTO에 `generatedAt` 또는 `snapshotAt`을 포함했다.
[ ] 오류 발생 시 500만 반환하지 않고 admin-friendly error payload를 반환한다.
[ ] readiness blocked reason을 payload에 포함했다.
[ ] conflict files가 있으면 distributor detail에 포함했다.
[ ] missing/unavailable slot 상태를 blocked와 구분했다.
[ ] queue depth 계산 기준을 문서화했다.
[ ] mock metric과 실 metric을 구분했다.
```

## 17.3 Frontend 체크리스트

```text
[ ] 좌측 sidebar가 모든 주요 화면으로 이동 가능하다.
[ ] 상단 KPI 카드가 loading/success/error 상태를 가진다.
[ ] office board는 DOM 기반 absolute anchor 구조로 구현했다.
[ ] office board 배경과 overlay 상태가 분리되어 있다.
[ ] slot 상태별 색상과 아이콘이 고정되어 있다.
[ ] agent 상태별 말풍선 문구가 고정되어 있다.
[ ] blocked 상태는 색상만이 아니라 아이콘/문구로도 구분된다.
[ ] slot 클릭 시 관련 agent/task가 highlight된다.
[ ] agent 클릭 시 관련 slot/task가 highlight된다.
[ ] distributor 클릭 시 pipeline panel이 focus된다.
[ ] event 클릭 시 event detail이 열린다.
[ ] 자동 새로고침 중 화면 깜빡임이 없다.
[ ] polling 실패 시 stale 상태를 표시한다.
[ ] 모바일 또는 작은 화면에서는 office board가 깨지지 않고 축소된다.
[ ] 텍스트 overflow는 ellipsis 처리한다.
[ ] 긴 branch name은 tooltip으로 전체 확인 가능하다.
[ ] Korean UI text가 줄바꿈으로 레이아웃을 깨지 않는다.
```

## 17.4 디자인 체크리스트

```text
[ ] “게임발전국” 로고와 AKRA 서브타이틀이 명확하다.
[ ] 중앙 보드는 게임 느낌이 있지만 Admin UI 기능성을 해치지 않는다.
[ ] 캐릭터/픽셀 요소가 과해서 데이터 가독성을 떨어뜨리지 않는다.
[ ] gold accent는 rank/reward 계열에만 사용한다.
[ ] red는 blocked/failed/danger에만 사용한다.
[ ] green은 ready/running/pass/done에 사용한다.
[ ] orange는 warning/cleanup/high load에 사용한다.
[ ] tooltip, badge, chip의 스타일이 통일되어 있다.
[ ] 작업 상세 카드와 event log는 실제 운영자가 읽기 쉬운 밀도를 유지한다.
[ ] office board 안의 floating label이 서로 겹치지 않는다.
```

## 17.5 QA 체크리스트

```text
[ ] readiness ready 상태 snapshot 테스트
[ ] readiness blocked 상태 snapshot 테스트
[ ] pool idle/running/blocked 혼합 상태 테스트
[ ] pool exhausted 상태 테스트
[ ] slot missing 상태 테스트
[ ] slot unavailable 상태 테스트
[ ] agent roster empty 상태 테스트
[ ] active agents 여러 명 상태 테스트
[ ] distributor queue empty 상태 테스트
[ ] distributor queue depth > 0 상태 테스트
[ ] distributor blocked 상태 테스트
[ ] runtime event empty 상태 테스트
[ ] runtime event 50개 이상 pagination 테스트
[ ] polling 실패 후 복구 테스트
[ ] 긴 branch name 렌더링 테스트
[ ] 긴 task title 렌더링 테스트
[ ] 한글/영문 혼합 텍스트 렌더링 테스트
[ ] click highlight 동작 테스트
[ ] event detail drawer 동작 테스트
[ ] 자동 새로고침 중 선택 상태 유지 테스트
[ ] API 응답 지연 시 skeleton 표시 테스트
[ ] 권한 없는 사용자 접근 테스트
```

## 17.6 Release 체크리스트

```text
[ ] Admin route가 staging에서 접근 가능하다.
[ ] 실제 prerelease workspace snapshot과 연결했다.
[ ] mock data가 production build에 남아 있지 않다.
[ ] API base URL 환경변수가 분리되어 있다.
[ ] feature flag로 dashboard 노출을 제어할 수 있다.
[ ] read-only endpoint만 production에 먼저 배포했다.
[ ] 운영 액션 버튼은 비활성 또는 숨김 처리했다.
[ ] 에러 로그가 수집된다.
[ ] dashboard polling interval이 과도하지 않다.
[ ] event API가 DB에 과부하를 주지 않는다.
[ ] 시각 회귀 테스트 기준 스크린샷을 저장했다.
[ ] 장애 시 기존 TUI 운영 흐름에 영향이 없다.
```

---

# 18. 작업자용 명확한 금지 사항

```text
[금지] Frontend에서 slot 상태를 임의 계산하지 말 것.
[금지] TUI 문자열을 파싱해서 Admin UI를 만들지 말 것.
[금지] 자동 polling에서 reconcile/reset/tick 같은 side effect를 실행하지 말 것.
[금지] XP, 코인, 랭킹을 실제 값처럼 표시하지 말 것.
[금지] blocked와 offline을 같은 상태로 표시하지 말 것.
[금지] missing과 unavailable을 단순 idle로 처리하지 말 것.
[금지] distributor queue depth와 planning queue depth를 섞지 말 것.
[금지] event sequence 중복 표시를 허용하지 말 것.
[금지] 긴 branch/task 이름 때문에 카드 레이아웃이 깨지게 두지 말 것.
[금지] 게임 요소 때문에 실제 운영 상태가 읽히지 않게 만들지 말 것.
```

---

# 19. 최종 MVP 범위

이번 1차 개발의 권장 범위는 다음입니다.

```text
1. 게임발전국 Admin shell
2. Top KPI row
3. Isometric Office Board
4. Worktree Pool 상태 표시
5. Agent 상태 표시
6. Task 상세 표시
7. Distributor pipeline 표시
8. Runtime event log 표시
9. Guild Performance 파생 지표 표시
10. 10초 read-only polling
```

제외:

```text
1. 실제 XP 저장
2. 영구 랭킹
3. pool reset 버튼
4. distributor 강제 진행 버튼
5. agent 직접 제어
6. worktree cleanup 실행
7. GitHub PR approve/deny
```

---

# 20. 한 줄 결론

이번 Admin Web 고도화는 **AKRA의 병렬 작업 운영 상태를 “게임발전국 본부”라는 isometric office simulation으로 시각화하는 read-only 관제 대시보드**로 시작하는 것이 가장 안전합니다.

핵심은 다음입니다.

```text
실제 상태는 AKRA domain/application snapshot에서 가져온다.
게임화는 frontend view model에서만 표현한다.
운영 side effect는 MVP에서 제외한다.
blocked, readiness, distributor barrier는 과장 없이 정확히 보여준다.
```

이 방향이면 작업자는 게임형 UI를 구현하면서도 기존 `Supersession`, `Worktree Pool`, `Agent Roster`, `Distributor`, `Runtime Event` 흐름을 깨지 않고 진행할 수 있습니다.
