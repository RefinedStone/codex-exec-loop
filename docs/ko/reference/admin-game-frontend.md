# Admin 게임 프런트엔드

[English](../../reference/admin-game-frontend.md)

이 문서는 `/admin/akra` 게임 프런트엔드의 실제 제공 계약을 기록합니다. Native TUI, planning
authority, parallel policy, application control plane은 변경하지 않습니다.

## Runtime 경계

- 기존 Admin dashboard JSON이 semantic source입니다. `akra-dashboard.js`가 DOM panel을 검증·렌더한
  뒤 같은 dashboard object를 `window.AkraAdminGame`에 직접 전달합니다.
- Game bundle은 pixel에서 lifecycle 상태를 추론하지 않고 task, slot, lease, retry, delivery policy를
  소유하지 않습니다.
- `DashboardSceneStore`가 scene payload를 검증하고 immutable scene snapshot 하나를 renderer에
  발행합니다. DOM `data-*` attribute는 접근성/detail drawer projection이며 Pixi scene transport가
  아닙니다.
- 초기 dashboard/event request는 즉시 실행하고 browser는 `/api/admin/akra/stream`을 열어 최신 durable
  runtime-event sequence부터 이어받습니다.
- Stream에는 가벼운 event, control, invalidation frame만 싣습니다. Dashboard snapshot이 bootstrap과
  reconciliation의 authoritative read model입니다.
- `EventSource`가 없거나 끊기거나 stale이면 bounded dashboard/event/control polling으로 자동
  복귀합니다. 정상 stream도 더 느린 snapshot reconciliation을 유지합니다.

## Realtime과 command 계약

- Stream frame은 schema version `1`, SSE event 이름 `update`, 가능한 경우 runtime-event sequence를 SSE
  event ID로 사용합니다. Reconnect는 `Last-Event-ID`, 최초 연결은 `afterSequence`를 사용할 수 있습니다.
- 한 frame에 담을 수 없는 incremental event가 있으면 `cursorResetRequired`가 browser에 bounded event
  list를 fresh snapshot으로 교체하게 합니다. Gap을 숨기지 않습니다.
- `POST /api/admin/akra/control`은 typed application control plane과 CSRF header를 유지합니다. 응답에는
  stable `commandId`와 `accepted | running | completed | blocked` 상태의 bounded process-local command
  record가 들어갑니다.
- `GET /api/admin/akra/commands/{commandId}`는 retained command 결과를 제공합니다. Command tracking은
  browser feedback projection이며 durable runtime event와 planning authority가 운영 source of truth입니다.

## PR validation 운영

- Full-width Validation Rail은 application-owned bounded validation board를 소비합니다. Browser에서
  SQLite/GitHub를 읽지 않고 event 문장에서 phase/severity를 추론하지 않습니다.
- Typed rollout banner는 `OFF`, `SHADOW`, `REMEDIATION`을 구분하고 Planning Queue admission 차단 여부와
  Ruleset 변경의 별도 승인 필요 여부를 표시합니다.
- Detail drawer는 attestation, exact evidence SHA, required/optional check, latest attempt,
  provider/backoff, timeline, finding-task-slot correlation을 표시합니다. Mutation은 CSRF, expected
  revision, stable command identity, application command port를 사용합니다.
- Rollout evidence summary는 application이 계산합니다. Historical/projected Fast Gate와 독립 actual
  Fast Gate를 분리하고, 누락 표본은 unavailable로 표시하며, freshness/mode 불일치는 typed text/icon
  warning으로 노출합니다.
- Dashboard bootstrap은 최신 summary만 전달합니다. Evidence drawer를 열 때 cursor로 정렬된 history를
  최대 20개까지 lazy request하고, text-first trend 3개, typed collection ownership/recovery, warning을
  함께 읽습니다. Durable store는 redacted snapshot 최대 64개를 보존합니다.
- Evidence SSE frame에는 revision/cursor-reset invalidation만 들어갑니다. Hidden document는 EventSource를
  닫고 fallback polling을 건너뛰며, 다시 보이면 dashboard/event를 한 번 reconcile한 뒤 reconnect합니다.
- Passive validation은 QA/CI station과 signal packet으로 표시합니다. Correlated ordinary Queue task가
  실제 worker lease를 소유할 때만 worker character가 생깁니다.
- 별도의 PR 승인관은 항상 존재하는 environment NPC이며 worker가 아닙니다. 서버는 durable
  `github_rebase_merge` evidence가 있을 때만 `idle`, `reviewing`, `failure`, `success` 상태를 결정합니다.
  성공의 유일한 근거는 `Verified`이고 check 개수만으로 승인을 추론하지 않습니다. Retryable provider
  대기, stale, pause, remediation은 애니메이션 추측이 아니라 명시적인 qualifier로 전달됩니다.
- 승인관 카드에는 정확한 PR, evidence SHA, required-check 진행도, 미해결 finding 수가 표시됩니다.
  카드나 캐릭터를 선택하면 DOM의 첫 검증 항목이 아니라 현재 연출 중인 `recordKey`의 상세를 엽니다.
  스테이션, signal packet, 캐릭터, 상세 drawer는 같은 record를 유지하고, 캐릭터 hit target은 넓은
  DELIVERY room POI보다 위에서 입력을 받습니다.
- Application-owned Debug Harness가 결정론적 validation scenario 열 개를 제공합니다. API, DOM, Pixi
  inspection은 같은 phase, severity, record identity, lease fact를 보고해야 합니다.

## Operations cockpit 정보 구조

- Command header만 readiness, branch, revision, observation time, active agent, idle slot, distributor queue
  depth를 항상 표시합니다. Realtime freshness와 manual refresh도 map에 반복하지 않고 이 header에 둡니다.
- Attention strip은 readiness가 degraded/blocked일 때만 나타나며 control보다 먼저 원인, 다음 안전 조치,
  bounded diagnostic을 제시합니다.
- 실제 loop control과 deterministic Debug Harness는 서로 배타적인 presentation mode입니다. Fake mode는
  compact scenario rail 하나만 표시하고 disabled production control을 중복 배치하지 않습니다.
- Operations scene에는 mission-count HUD가 없습니다. Agent/station motion을 시각적으로 우선하고 zoom
  control만 persistent map overlay로 둡니다.
- Pool rail은 slot authority, active-lane rail은 중복 KPI 없는 lane entity, event rail은 event count/row를
  소유합니다.
- 선택한 entity fact는 연결된 detail drawer를 사용합니다. 기존 always-visible selected-task card는 actor와
  drawer fact를 반복하고 empty state도 campaign copy를 반복해 제거했습니다.
- Delivery pipeline은 scene 아래 full-width evidence rail을 유지해 일곱 stage가 작은 side card로
  무너지지 않게 합니다.

## Debug Harness

- `akra admin --debug-harness`는 Admin UI/UX 작업용 process-local deterministic Fake를 켭니다.
- Fake clock, scenario selection, play/pause/step/reset command, stage history는 application layer의
  `AdminDebugHarnessService`가 소유합니다. HTTP adapter는 semantic projection을 기존 dashboard JSON으로
  매핑할 뿐입니다.
- Harness는 planning authority, lease, worktree, Git, GitHub state를 쓰지 않습니다. 활성화 중 실제
  parallel browser control은 `409 Conflict`를 반환하고 dashboard가 protected로 표시합니다.
- `GET|POST /api/admin/akra/debug-harness`는 다른 Admin API와 같은 인증을 사용하고 mutation은 기존
  cookie-bound CSRF header를 요구합니다.
- Built-in scenario는 normal delivery loop, blocked-lane recovery, queue pressure를 포함합니다. 각 stage가
  pool, actor, campaign, distributor, metric, event projection을 함께 교체해 DOM과 Pixi가 coherent
  snapshot 하나를 관찰하게 합니다.
- Fake stage revision은 기존 SSE reconciliation path로 전달됩니다. Browser는 fresh authoritative
  dashboard snapshot을 요청하고 worker movement는 presentation-only로 유지됩니다.

## Renderer

- Vite가 strict TypeScript와 PixiJS 8을 기존 IIFE static-asset 경계로 bundle합니다. Pixi 8 CSP adapter는
  Admin shell의 strict CSP에 맞게 renderer 초기화 전에 등록합니다.
- Renderer는 보통 Pixi `60 fps` requestAnimationFrame ticker를 따릅니다. Host가 rAF를 throttle할 때만
  visible-tab `60 fps` timer가 대신하며 hidden tab에서는 둘 다 갱신하지 않습니다. Scene inspection은
  browser-harness 검증을 위해 active frame driver를 표시합니다.
- `AgentWorld`는 map, actor, 고정 PR 승인관, signal packet, point of interest, label, depth ordering을
  소유합니다.
- `SceneCameraController`는 fit, pan, wheel/pinch zoom, zoom control, bounds,
  `overview -> operations -> detail` semantic zoom projection을 소유합니다.
- Actor state가 바뀌면 retained unit을 새 semantic destination으로 이동합니다. Front/rear/strict-side atlas
  row만 사용하며 diagonal character direction을 만들지 않습니다.
- Normal travel은 exponential tail 대신 `168` world-pixels/s 상수로 `30%` presentation-speed 목표를
  유지합니다. 네 방향 walk frame은 archetype/facing별 center와 foot alignment를 사용하며 전신 pose를
  crossfade하지 않습니다. 동기화된 `760 ms` procedural gait는 발자국과 shadow 크기를 고정한 채 `60 fps`
  driver에서 subpixel sway와 lift만 더합니다. Reduced motion은 walk cycle 없이 semantic destination으로
  snap합니다.
- 변경 없는 idle/configured-standby unit은 roam하거나 packet을 만들지 않습니다. Atlas pose가 있으면
  laptop/seated를 선택하고, 아니면 coordinate를 바꾸지 않는 `960 ms` four-frame in-place gait를 `65%`
  amplitude로 사용할 수 있습니다.
- Scene inspection은 unit별 animation kind/frame, frame blend, procedural gait offset, movement-speed ratio를
  보고해 canvas screenshot과 독립적으로 motion을 검증합니다.
- 정확한 map texture의 masked duplicate가 actor 위의 desk front를 복원해 두 번째 수작업 foreground asset
  없이 deterministic furniture occlusion을 만듭니다.
- Pixi hit target은 typed scene-selection event를 발행합니다. DOM dashboard는 접근 가능한 actor list와
  detail drawer를 유지합니다.

## Map과 sprite 계약

- World coordinate는 `1672 x 941`입니다.
- Production map은 `akra-operations-studio-v3.png`입니다.
- Worker source는 `gamebaljeonguk_atlas_128x192.png`입니다.
- Workstation foot anchor 다섯 개는 front/back-facing chair에 맞춥니다. Review, delivery, cleanup,
  standby destination은 분리된 빈 room block을 사용합니다.
- Worker sprite는 `0.72` world scale을 사용합니다. Working actor는 duplicate furniture가 들어간 legacy
  laptop emote 대신 map desk 뒤의 rear-facing row를 사용합니다.
- `pr-approver-atlas-128x192.png`는 별도의 `6 x 4` atlas입니다. 네 행은 intake, page review, failure,
  success이고 발 anchor와 scale은 모든 frame에서 고정됩니다. 종이, 팔, 표정, 제한된 status accent만
  움직입니다. Failure와 success는 한 번 재생한 뒤 마지막 pose에서 멈추고 reduced-motion 및
  paused/waiting qualifier는 대표 정지 pose를 사용합니다.

## Image generation provenance

V3 map은 2026-07-27 내장 ImageGen으로 생성했습니다. 기존 worker atlas를 camera, scale, silhouette,
pixel density의 주 reference로, v2 map을 functional zoning과 mood의 secondary reference로 사용했습니다.

Production prompt는 낮은 20–25도 orthographic dollhouse camera, 빈 front/back/strict-side workstation
다섯 개, command/review/delivery/standby room block, 넓은 walkway, warm walnut/amber task light, cool cyan
technical light, crisp character-scale pixel art를 요구했습니다. 사람, text, logo, floating UI, steep
isometric perspective, diagonal chair, painterly blur, baked status icon은 금지했습니다.

PR 승인관 atlas는 2026-08-11 내장 ImageGen으로 생성했고 worker atlas와 앞선 승인관 concept을 reference로
사용했습니다. Prompt는 한 명의 성인 승인관 identity, camera, head size, body height, foot baseline, scale을
24개 cell에 고정하고 intake, page review, failure, success를 각각 여섯 frame으로 구성했습니다. Magenta
chroma background, 흰 종이와 안경의 어두운 outline, shadow/text/white halo 금지를 명시했습니다. 공식
ImageGen chroma-removal helper가 alpha source를 만들고 repository preparation script가 각 cell을 공통
baseline으로 정규화한 뒤 half atlas를 nearest-neighbor로만 파생합니다. 전체 prompt와 generation
metadata는 `templates/admin/resources/pr_approver_sprite_pack/`에 있습니다.

## 검증

다음을 실행합니다.

```text
node --check scripts/capture_admin_validation_evidence.mjs
node --check scripts/capture_admin_pr_approver.mjs
node --check assets/admin/scripts/akra-dashboard.js
npm --prefix assets/admin/game run check
npm --prefix assets/admin/game run sprites:approver:check
npm --prefix assets/admin/game run build
cargo test game_approver --lib
cargo test akra_graphic_dashboard --lib
bash scripts/check_admin_graphic_visual.sh
```

Production composition evidence drawer 캡처와 15초 DOM/payload/performance 표본은
[`admin-pr-validation-approval-package-2026-08-11`](../../validation/artifacts/admin-pr-validation-approval-package-2026-08-11/README.md)에
있습니다.
