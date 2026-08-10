# Admin Game Frontend

[한국어](../ko/reference/admin-game-frontend.md)

This reference records the shipped `/admin/akra` game-frontend contract. It does not change the
native TUI, planning authority, parallel policy, or application control plane.

## Runtime boundary

- The existing Admin dashboard JSON remains the semantic source. `akra-dashboard.js` validates and
  renders the DOM panels, then gives the same dashboard object directly to `window.AkraAdminGame`.
- The game bundle never infers lifecycle state from pixels or owns task, slot, lease, retry, or
  delivery policy.
- `DashboardSceneStore` validates the scene payload and publishes one immutable scene snapshot to
  the renderer. DOM `data-*` attributes remain accessibility/detail-drawer projections, not the
  Pixi scene's state transport.
- The initial dashboard and event requests run immediately. The browser then opens
  `/api/admin/akra/stream` and resumes from its latest durable runtime-event sequence.
- The stream carries lightweight event, control, and invalidation frames. The dashboard snapshot
  remains the authoritative bootstrap and reconciliation read model.
- If `EventSource` is unavailable, disconnected, or stale, bounded dashboard/event/control polling
  resumes automatically. A healthy stream still performs a slower snapshot reconciliation.

## Realtime and command contract

- Stream frames use schema version `1`, the SSE `update` event name, and a runtime-event sequence as
  the SSE event ID when one is available. Browser reconnects can resume through `Last-Event-ID`;
  initial connections may use `afterSequence`.
- When more incremental events exist than fit in one frame, `cursorResetRequired` tells the browser
  to replace its bounded event list from a fresh snapshot instead of silently presenting a gap.
- `POST /api/admin/akra/control` continues to use the typed application control plane and CSRF
  header. Its response now includes a bounded, process-local command record with a stable
  `commandId` and `accepted`, `running`, `completed`, or `blocked` state.
- `GET /api/admin/akra/commands/{commandId}` exposes the retained command result. Command tracking
  is a browser feedback projection; durable runtime events and the planning authority remain the
  operational source of truth.

## PR validation operations

- The full-width Validation Rail consumes the application-owned bounded validation board. It never
  reads SQLite or GitHub from the browser and never derives phase or severity from event prose.
- A typed rollout banner distinguishes `OFF`, `SHADOW`, and `REMEDIATION`, and separately states
  whether Planning Queue admission is blocked and whether a Ruleset change still needs approval.
- The detail drawer exposes attestation, exact evidence SHA, required and optional checks, latest
  attempt, provider/backoff, timeline, and finding-task-slot correlation. Mutations use CSRF,
  expected revision, stable command identity, and the application command port.
- The rollout evidence summary is application-computed. It keeps historical/projected and
  independently actual Fast Gate metrics separate, reports missing samples as unavailable, and
  exposes freshness/mode mismatch through typed text and icon warnings.
- Dashboard bootstrap carries only the latest summary. Opening the evidence drawer lazily requests
  at most 20 cursor-ordered history rows, three text-first trend comparisons, typed collection
  ownership/recovery, and warnings. The durable store retains at most 64 redacted snapshots.
- Evidence SSE frames contain only revision/cursor-reset invalidation. Hidden documents close the
  EventSource and skip fallback polling; visibility recovery reconciles dashboard and events once
  before reconnecting.
- Passive validation appears as a QA/CI station and signal packet. A worker character exists only
  when a correlated ordinary Queue task owns an actual worker lease.
- The application-owned debug harness supplies ten deterministic validation scenarios. API, DOM,
  and Pixi inspection must report the same phase, severity, record identity, and lease fact.

## Operations cockpit information architecture

- The command header is the only always-visible summary of readiness, branch, revision, observation
  time, active agents, idle slots, and distributor queue depth. Realtime freshness and manual
  refresh live in that same header instead of being repeated over the map.
- The attention strip appears only when readiness is degraded or blocked. It presents the cause,
  next safe action, and bounded diagnostic before any control.
- Real loop controls and the deterministic debug harness are mutually exclusive presentation
  modes. Fake mode shows one compact scenario rail and does not leave disabled production controls
  occupying a second operator brief.
- The operations scene has no mission-count HUD. Agent and station motion stays visually primary;
  zoom controls are the only persistent map overlay.
- The pool rail is the slot authority surface, the active-lane rail contains lane entities without
  repeating KPI cards, and the event rail owns event counts and rows.
- Selected entity facts use the linked detail drawer. The former always-visible selected-task card
  was removed because it repeated actor and drawer facts while empty states repeated campaign copy.
- The delivery pipeline remains a full-width evidence rail below the scene so its seven stages do
  not collapse into an unreadable side card.

## Debug harness

- `akra admin --debug-harness` enables a process-local deterministic Fake for Admin UI/UX work.
- The Fake clock, scenario selection, play/pause/step/reset commands, and stage history are owned by
  `AdminDebugHarnessService` in the application layer. The HTTP adapter only maps its semantic
  projection into the existing dashboard JSON.
- The harness never writes planning authority, leases, worktrees, Git, or GitHub state. While it is
  enabled, real parallel browser controls return `409 Conflict` and the dashboard labels them as
  protected.
- `GET|POST /api/admin/akra/debug-harness` is authenticated like the other Admin APIs; mutation
  requires the existing cookie-bound CSRF header.
- The built-in scenarios cover a normal delivery loop, blocked-lane recovery, and queue pressure.
  Each stage replaces pool, actor, campaign, distributor, metric, and event projections together so
  the DOM and Pixi world observe one coherent snapshot.
- Fake stage revision changes travel through the existing SSE reconciliation path. The browser
  requests a fresh authoritative dashboard snapshot and worker movement remains presentation-only.

## Renderer

- Vite bundles strict TypeScript and PixiJS 8 into the existing IIFE static-asset boundary. The
  Pixi 8 CSP adapter is registered before renderer initialization for the Admin shell's strict CSP.
- The renderer normally follows Pixi's `60 fps` requestAnimationFrame ticker. A visible-tab
  `60 fps` timer takes over only when the host throttles requestAnimationFrame; hidden tabs update
  neither path. Scene inspection exposes the active frame driver for browser-harness verification.
- `AgentWorld` owns the map, actors, signal packets, points of interest, labels, and depth ordering.
- `SceneCameraController` owns fit, pan, wheel zoom, pinch zoom, zoom controls, bounds, and the
  `overview -> operations -> detail` semantic zoom projection.
- Actor state changes move the retained unit toward its new semantic destination. Movement uses
  front, rear, or strict side atlas rows; diagonal character directions are not fabricated.
- Normal-motion travel keeps the `30%` presentation-speed target as a constant `168` world-pixels
  per second instead of an exponential interpolation tail. The four directional walk frames use
  archetype/facing-specific center and foot alignment, then briefly crossfade at each step boundary.
  A synchronized `760 ms` procedural gait adds subpixel sway, lift, and matching shadow compression
  on the `60 fps` frame driver. Reduced-motion mode still snaps to the semantic destination without
  a walk cycle.
- An unchanged idle or configured-standby unit never roams and never emits a packet. It may use a
  slower `960 ms` four-frame in-place gait at `65%` amplitude without changing its semantic
  coordinate; explicit laptop or seated poses remain selected when the atlas provides them.
- Scene inspection reports each unit's current animation kind/frame, frame blend, procedural gait
  offset, and movement-speed ratio so the debug harness can verify motion independently of canvas
  screenshots.
- A masked duplicate of the exact map texture restores workstation desk fronts above actors. This
  gives deterministic furniture occlusion without a second hand-painted foreground asset.
- Pixi hit targets emit typed scene-selection events. The DOM dashboard retains the accessible
  actor list and detail drawer.

## Map and sprite contract

- World coordinates are `1672 x 941`.
- `akra-operations-studio-v3.png` is the production map.
- `gamebaljeonguk_atlas_128x192.png` remains the exact worker source.
- Five workstation foot anchors align to front/back-facing chairs. Review, delivery, cleanup, and
  standby destinations use separate uncluttered room blocks.
- Worker sprites use a `0.72` world scale. Working actors use their rear-facing row behind the map's
  desks instead of legacy laptop emotes that contain duplicate furniture.

## Image generation provenance

The v3 map was generated with the built-in ImageGen tool on 2026-07-27. The existing worker atlas
was the primary camera, scale, silhouette, and pixel-density reference; the v2 map was a secondary
functional-zoning and mood reference.

The production prompt required a low 20–25 degree orthographic dollhouse camera, five empty
front/back/strict-side workstations, command/review/delivery/standby room blocks, wide walkways,
warm walnut and amber task lighting, cool cyan technical lighting, and crisp character-scale pixel
art. It explicitly prohibited people, text, logos, floating UI, steep isometric perspective,
diagonal chairs, painterly blur, and baked status icons.

## Verification

Run:

```text
node --check scripts/capture_admin_validation_evidence.mjs
node --check assets/admin/scripts/akra-dashboard.js
npm --prefix assets/admin/game run check
npm --prefix assets/admin/game run build
cargo test akra_graphic_dashboard --lib
bash scripts/check_admin_graphic_visual.sh
```

The checked-in production-composition evidence drawer capture and 15-second DOM/payload/performance
sample are in
[`admin-pr-validation-approval-package-2026-08-11`](../validation/artifacts/admin-pr-validation-approval-package-2026-08-11/README.md).
