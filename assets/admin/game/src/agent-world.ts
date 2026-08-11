import {
  Container,
  Graphics,
  Rectangle,
  Sprite,
  Text,
  Texture,
} from "pixi.js";
import {
  alignmentForFacing,
  archetypeForProfile,
  buildAgentFrameSets,
  frameForFacing,
  resolveRestTexture,
  scaleForFacing,
  sourceFrameIndexForFacing,
  type AgentFrameSet,
} from "./agent-atlas";
import type {
  AgentAnimationKind,
  ArchetypeKey,
  DashboardSceneSnapshot,
  Facing,
  GameActorProjection,
  GamePrApproverProjection,
  GameStandbyProjection,
  GameValidationProjection,
  Point,
  PresenceKind,
  SceneInspection,
  SemanticZoomLevel,
  StaticPose,
  StatusSeverity,
  VisualState,
} from "./game-types";
import {
  PrApproverAnimator,
  buildPrApproverFrames,
  type PrApproverAnimationSnapshot,
} from "./pr-approver-atlas";
import {
  AGENT_SPRITE_SCALE,
  CLEANUP_STATION_POINTS,
  DELIVERY_STATION_POINTS,
  MAP_HEIGHT,
  MAP_WIDTH,
  OCCLUSION_POLYGONS,
  POINTS_OF_INTEREST,
  PR_APPROVER_POINT,
  PR_APPROVER_SPRITE_SCALE,
  QA_CI_SIGNAL_POINTS,
  QA_CI_STATION_POINT,
  REVIEW_STATION_POINTS,
  SLOT_SEATS,
  STANDBY_LOUNGE_PATROL_ROUTES,
  STANDBY_LOUNGE_POINTS,
  STATE_LABELS,
  STATUS_PALETTE,
  pointAt,
  seatPoint,
  stateTargetPoint,
} from "./scene-config";

type CharacterProjection = GameActorProjection | GameStandbyProjection;

interface AgentUnit {
  key: string;
  presenceKind: PresenceKind;
  actorId: string;
  agentId: string;
  slotId: string;
  seatIndex: number;
  locationIndex: number;
  displayName: string;
  visualState: VisualState;
  pose: StaticPose;
  severity: StatusSeverity;
  bubbleLabel: string;
  archetype: ArchetypeKey;
  group: Container;
  sprite: Sprite;
  blendSprite: Sprite;
  shadow: Graphics;
  marker: Graphics;
  label: Text;
  homePoint: Point;
  currentPoint: Point;
  targetPoint: Point;
  facing: Facing;
  flipX: boolean;
  motionPhase: number;
  hovered: boolean;
  animationKind: AgentAnimationKind;
  animationFrameIndex: number | null;
  sourceFrameIndex: number | null;
  animationBlend: number;
  gaitOffsetX: number;
  gaitOffsetY: number;
  restTexture: Texture;
  restFrameScale: number;
  frameScale: number;
  restResolvedAtlasFrameIndex: number | null;
  restPoseFallback: boolean;
  resolvedAtlasFrameIndex: number | null;
  poseFallback: boolean;
  standbyPatrolRoute: Point[];
  standbyPatrolIndex: number;
  ambientActivity: boolean;
}

interface SignalPacket {
  unitKey: string;
  graphic: Graphics;
  from: Point;
  to: Point;
  phase: number;
}

interface ValidationStationVisual {
  group: Container;
  panel: Graphics;
  beacon: Graphics;
  caption: Text;
  label: Text;
  modeLabel: Text;
  progressTrack: Graphics;
  progressFill: Graphics;
}

interface PrApproverVisual {
  group: Container;
  interactionTarget: Container;
  sprite: Sprite;
  shadow: Graphics;
  stateMarker: Graphics;
  animator: PrApproverAnimator;
  snapshot: PrApproverAnimationSnapshot;
}

const emptyApproverProjection = (): GamePrApproverProjection => ({
  state: "idle",
  qualifier: "none",
  recordKey: null,
  pullRequestNumber: null,
  evidenceShortSha: null,
  integrationMethod: null,
  requiredChecksSucceeded: 0,
  requiredChecksTotal: 0,
  findingCount: 0,
  remediationCount: 0,
  statusLabel: "통합 PR 대기",
  transitionKey: "idle",
});

const emptyValidationProjection = (): GameValidationProjection => ({
  stationState: "idle",
  severity: "muted",
  label: "QA/CI · 관찰 없음",
  recordKey: null,
  phase: null,
  packetKind: null,
  workerLeaseActive: false,
  approver: emptyApproverProjection(),
});

const distance = (from: Point, to: Point): number =>
  Math.hypot(to.x - from.x, to.y - from.y);

const stableNumber = (value: string): number =>
  [...value].reduce((sum, character) => sum + character.charCodeAt(0), 0);

const copyPoint = (point: Point): Point => ({ x: point.x, y: point.y });

const approverColor = (projection: GamePrApproverProjection): number => {
  switch (projection.state) {
    case "success":
      return STATUS_PALETTE.success;
    case "failure":
      return STATUS_PALETTE.danger;
    case "reviewing":
      return projection.qualifier === "waiting" || projection.qualifier === "stale"
        ? STATUS_PALETTE.warning
        : STATUS_PALETTE.info;
    default:
      return STATUS_PALETTE.muted;
  }
};

const approverStateLabel = (projection: GamePrApproverProjection): string => {
  switch (projection.state) {
    case "success":
      return "승인 완료";
    case "failure":
      return projection.qualifier === "recovering" ? "재작업 확인" : "반려 확인";
    case "reviewing":
      if (projection.qualifier === "paused") return "검토 일시정지";
      if (projection.qualifier === "stale") return "새 증거 대기";
      if (projection.qualifier === "waiting") return "공급자 응답 대기";
      return "문서 검토 중";
    default:
      return "통합 PR 대기";
  }
};

export const AGENT_MOVEMENT_SPEED_RATIO = 0.3;
export const AGENT_TRAVEL_SPEED_WORLD_PX_PER_SECOND = 168;
export const STANDBY_LOUNGE_TRAVEL_SPEED_WORLD_PX_PER_SECOND = 52;
export const WALK_IN_PLACE_CYCLE_MS = 760;
export const IDLE_IN_PLACE_CYCLE_MS = 960;
export const IDLE_IN_PLACE_AMPLITUDE_RATIO = 0.65;
export const WALK_SWAY_WORLD_PX = 0.7;
export const WALK_LIFT_WORLD_PX = 1.6;

const MAX_MOVEMENT_DELTA_MS = 50;

const visibleStepFrameIndex = (gaitProgress: number): number =>
  Math.floor(gaitProgress * 4) % 4;

const applyVisibleStepAppearance = (
  unit: AgentUnit,
  frameSets: Record<ArchetypeKey, AgentFrameSet>,
  facing: Facing,
  gaitProgress: number,
  gaitOffsetX: number,
  gaitOffsetY: number
): void => {
  const frameIndex = visibleStepFrameIndex(gaitProgress);
  const sourceFrameIndex = sourceFrameIndexForFacing(
    unit.archetype,
    facing,
    frameIndex
  );
  const currentAlignment = alignmentForFacing(
    unit.archetype,
    facing,
    frameIndex
  );
  unit.frameScale = scaleForFacing(unit.archetype, facing, frameIndex);
  unit.sprite.texture = frameForFacing(
    frameSets,
    unit.archetype,
    facing,
    frameIndex
  );
  unit.sprite.position.set(
    gaitOffsetX + currentAlignment.x * AGENT_SPRITE_SCALE * unit.frameScale,
    gaitOffsetY + currentAlignment.y * AGENT_SPRITE_SCALE * unit.frameScale
  );
  // Pixel-art poses vary in silhouette width. Blending full poses makes that
  // silhouette temporarily wider, which reads as an unintended scale pulse.
  unit.sprite.alpha = 1;
  unit.blendSprite.alpha = 0;
  unit.sprite.roundPixels = false;
  unit.blendSprite.roundPixels = true;
  unit.animationFrameIndex = frameIndex;
  unit.sourceFrameIndex = sourceFrameIndex;
  unit.animationBlend = 0;
};

const relevantPacketTarget = (unit: AgentUnit): Point | null => {
  if (unit.presenceKind !== "active") return null;
  if (unit.visualState === "awaiting_review") {
    return pointAt(REVIEW_STATION_POINTS, unit.motionPhase);
  }
  if (unit.visualState === "delivering") {
    return pointAt(DELIVERY_STATION_POINTS, unit.motionPhase);
  }
  if (unit.visualState === "cleanup") {
    return pointAt(CLEANUP_STATION_POINTS, unit.motionPhase);
  }
  return null;
};

const dispatchSceneSelection = (
  detail:
    | { kind: "actor"; actorId: string }
    | { kind: "poi"; detailTarget: string; recordKey?: string | null }
): void => {
  window.dispatchEvent(new CustomEvent("akra:scene-selection-requested", { detail }));
};

const createMapOcclusionMask = (): Graphics => {
  const mask = new Graphics();
  for (const polygon of OCCLUSION_POLYGONS) {
    mask.poly(polygon.flatMap((point) => [point.x, point.y])).fill(0xffffff);
  }
  return mask;
};

const drawStaticMarker = (
  marker: Graphics,
  visualState: VisualState,
  color: number,
  presenceKind: PresenceKind
): void => {
  const width = presenceKind === "configured_standby" ? 26 : 22;
  const height = presenceKind === "configured_standby" ? 7.2 : 6;
  marker.clear();
  if (visualState === "blocked") {
    marker
      .moveTo(-width * 0.7, -height)
      .lineTo(width * 0.7, height)
      .moveTo(width * 0.7, -height)
      .lineTo(-width * 0.7, height)
      .stroke({ width: 2.4, color, alpha: 1 });
    return;
  }
  if (visualState === "delivering") {
    marker
      .poly([-width, -height, width * 0.35, -height, width, 0, width * 0.35, height, -width, height])
      .fill({ color, alpha: 0.28 })
      .stroke({ width: 2, color, alpha: 0.95 });
    return;
  }
  marker.ellipse(0, 0, width, height).stroke({ width: 2, color, alpha: 0.92 });
  if (presenceKind === "configured_standby" || visualState === "working") {
    marker.ellipse(0, 0, width * 0.72, height * 0.72).fill({ color, alpha: 0.18 });
  }
  if (visualState === "awaiting_review") {
    marker
      .ellipse(0, 0, width * 0.62, height * 0.62)
      .stroke({ width: 1.5, color, alpha: 0.9 });
  }
};

const actorHomePoint = (projection: CharacterProjection): Point =>
  "seatIndex" in projection
    ? seatPoint(projection.seatIndex)
    : pointAt(STANDBY_LOUNGE_POINTS, projection.locationIndex - 1);

const actorTargetPoint = (projection: CharacterProjection, stableIndex: number): Point => {
  if ("seatIndex" in projection) {
    return stateTargetPoint(projection.visualState, projection.seatIndex, stableIndex);
  }
  return pointAt(STANDBY_LOUNGE_POINTS, projection.locationIndex - 1);
};

const standbyPatrolRoute = (locationIndex: number): Point[] => {
  const configuredRoute =
    STANDBY_LOUNGE_PATROL_ROUTES[
      Math.max(0, locationIndex - 1) % STANDBY_LOUNGE_PATROL_ROUTES.length
    ] ?? [];
  const fallback = pointAt(STANDBY_LOUNGE_POINTS, Math.max(0, locationIndex - 1));
  return configuredRoute.length > 1
    ? configuredRoute.map(copyPoint)
    : [copyPoint(fallback)];
};

const initialPoint = (
  projection: CharacterProjection,
  homePoint: Point,
  targetPoint: Point,
  stableIndex: number
): Point => {
  if (!("seatIndex" in projection)) return copyPoint(targetPoint);
  if (projection.visualState === "starting") {
    return copyPoint(pointAt(STANDBY_LOUNGE_POINTS, stableIndex));
  }
  if (
    projection.visualState === "awaiting_review" ||
    projection.visualState === "delivering" ||
    projection.visualState === "cleanup"
  ) {
    return copyPoint(homePoint);
  }
  return copyPoint(targetPoint);
};

const movementFacing = (from: Point, to: Point): { facing: Facing; flipX: boolean } => {
  const deltaX = to.x - from.x;
  const deltaY = to.y - from.y;
  if (Math.abs(deltaX) > Math.abs(deltaY) * 0.72) {
    return { facing: "side", flipX: deltaX > 0 };
  }
  return { facing: deltaY < 0 ? "up" : "down", flipX: false };
};

export class AgentWorld {
  readonly root = new Container({ sortableChildren: true });

  private readonly mapTexture: Texture;
  private readonly atlasTexture: Texture;
  private readonly frameSets: Record<ArchetypeKey, AgentFrameSet>;
  private readonly approverFrames: Texture[];
  private readonly agentLayer = new Container({ sortableChildren: true });
  private readonly packetLayer = new Container({ sortableChildren: true });
  private readonly labelLayer = new Container({ sortableChildren: true });
  private readonly poiLayer = new Container({ sortableChildren: true });
  private readonly units = new Map<string, AgentUnit>();
  private readonly packets = new Map<string, SignalPacket>();
  private readonly poiLabels: Text[] = [];
  private validation = emptyValidationProjection();
  private validationStation: ValidationStationVisual | null = null;
  private approverVisual: PrApproverVisual | null = null;
  private planningRevision: number | null = null;
  private zoomLevel: SemanticZoomLevel = "overview";
  private cameraZoom = 1;
  private renderCount = 0;
  private readonly reducedMotionQuery =
    window.matchMedia?.("(prefers-reduced-motion: reduce)") ?? null;
  private reducedMotion = this.reducedMotionQuery?.matches ?? false;
  private readonly onReducedMotionChange = (event: MediaQueryListEvent): void => {
    this.reducedMotion = event.matches;
    for (const unit of this.units.values()) {
      if (unit.presenceKind !== "configured_standby") continue;
      unit.ambientActivity =
        !this.reducedMotion && unit.standbyPatrolRoute.length > 1;
      if (!unit.ambientActivity) unit.targetPoint = copyPoint(unit.homePoint);
    }
    this.syncApproverPresentation();
  };

  constructor(mapTexture: Texture, atlasTexture: Texture, approverTexture: Texture) {
    this.mapTexture = mapTexture;
    this.atlasTexture = atlasTexture;
    this.frameSets = buildAgentFrameSets(atlasTexture);
    this.approverFrames = buildPrApproverFrames(approverTexture);
    this.reducedMotionQuery?.addEventListener("change", this.onReducedMotionChange);
    this.buildScene();
  }

  reconcile(snapshot: DashboardSceneSnapshot): void {
    this.planningRevision = snapshot.planningRevision;
    this.validation = snapshot.scene.validation;
    this.approverVisual?.animator.reconcile(this.validation.approver);
    const projections: CharacterProjection[] = [
      ...snapshot.scene.actors,
      ...snapshot.scene.standbyCharacters,
    ];
    const retained = new Set<string>();

    projections.forEach((projection, index) => {
      const key =
        "actorId" in projection
          ? `actor:${projection.actorId}`
          : `standby:${projection.characterId}`;
      retained.add(key);
      const stableIndex = stableNumber(key);
      const homePoint = actorHomePoint(projection);
      const targetPoint = actorTargetPoint(projection, stableIndex);
      const existing = this.units.get(key);
      if (existing) {
        existing.actorId = "actorId" in projection ? projection.actorId : "";
        existing.agentId = projection.agentId;
        existing.slotId = "slotId" in projection ? projection.slotId : "";
        existing.seatIndex = "seatIndex" in projection ? projection.seatIndex : 0;
        existing.locationIndex =
          "locationIndex" in projection ? projection.locationIndex : 0;
        existing.displayName = projection.displayName;
        existing.visualState = projection.visualState;
        existing.pose = projection.staticPose;
        existing.severity = projection.severity;
        existing.bubbleLabel = projection.bubbleLabel;
        existing.archetype = archetypeForProfile(projection.archetypeKey);
        existing.homePoint = copyPoint(homePoint);
        if (existing.presenceKind === "configured_standby") {
          existing.standbyPatrolRoute = standbyPatrolRoute(existing.locationIndex);
          existing.ambientActivity =
            !this.reducedMotion && existing.standbyPatrolRoute.length > 1;
          if (!existing.ambientActivity) {
            existing.standbyPatrolIndex = 0;
            existing.targetPoint = copyPoint(homePoint);
          }
        } else {
          existing.targetPoint = copyPoint(targetPoint);
        }
        this.syncUnitAppearance(existing);
        return;
      }
      const unit = this.createUnit(
        key,
        projection,
        initialPoint(projection, homePoint, targetPoint, stableIndex),
        homePoint,
        targetPoint,
        index
      );
      this.units.set(key, unit);
    });

    for (const [key, unit] of this.units) {
      if (retained.has(key)) continue;
      unit.group.destroy({ children: true });
      unit.label.destroy();
      this.units.delete(key);
    }
    this.syncValidationStation();
    this.syncApproverPresentation();
    this.rebuildSignalPackets();
    this.syncZoomPresentation();
  }

  update(deltaMilliseconds: number, elapsedMilliseconds: number): void {
    const movementDeltaSeconds =
      Math.min(
        Math.max(0, deltaMilliseconds),
        MAX_MOVEMENT_DELTA_MS
      ) / 1000;
    for (const unit of this.units.values()) {
      this.advanceStandbyPatrol(unit);
      const remaining = distance(unit.currentPoint, unit.targetPoint);
      let moving = remaining > 1.4;
      if (moving && this.reducedMotion) {
        unit.currentPoint = copyPoint(unit.targetPoint);
        moving = false;
      } else if (moving) {
        const travelStep = Math.min(
          remaining,
          this.travelSpeedFor(unit) * movementDeltaSeconds
        );
        unit.currentPoint.x +=
          ((unit.targetPoint.x - unit.currentPoint.x) / remaining) * travelStep;
        unit.currentPoint.y +=
          ((unit.targetPoint.y - unit.currentPoint.y) / remaining) * travelStep;
      } else {
        unit.currentPoint = copyPoint(unit.targetPoint);
      }

      let offsetX = 0;
      let offsetY = 0;
      let spriteOffsetX = 0;
      let spriteOffsetY = 0;
      let gaitProgress = 0;
      const inPlaceWalking =
        !this.reducedMotion &&
        (moving ||
          (unit.visualState === "idle" &&
            unit.restResolvedAtlasFrameIndex === null));
      if (inPlaceWalking) {
        const gaitCycleMilliseconds = moving
          ? WALK_IN_PLACE_CYCLE_MS
          : IDLE_IN_PLACE_CYCLE_MS;
        const gaitAmplitude = moving ? 1 : IDLE_IN_PLACE_AMPLITUDE_RATIO;
        gaitProgress =
          (elapsedMilliseconds / gaitCycleMilliseconds +
            unit.motionPhase / 17) %
          1;
        const gaitAngle = gaitProgress * Math.PI * 2;
        const stride = Math.sin(gaitAngle);
        const lift = Math.abs(stride);
        spriteOffsetX = stride * WALK_SWAY_WORLD_PX * gaitAmplitude;
        spriteOffsetY = -lift * WALK_LIFT_WORLD_PX * gaitAmplitude;
        // Keep a stable footprint: a pulsing shadow reinforces the same false
        // size-change impression that a moving pixel silhouette can create.
        unit.shadow.scale.set(1);
        unit.shadow.alpha = unit.presenceKind === "active" ? 0.34 : 0.25;
      } else if (!this.reducedMotion && unit.visualState === "blocked") {
        offsetX = Math.sin(elapsedMilliseconds * 0.014 + unit.motionPhase) * 2.2;
      } else if (
        !this.reducedMotion &&
        unit.visualState === "working" &&
        !moving
      ) {
        offsetY = Math.sin(elapsedMilliseconds * 0.0045 + unit.motionPhase) * 1.2;
      }
      if (!inPlaceWalking) {
        unit.shadow.scale.set(1);
        unit.shadow.alpha = unit.presenceKind === "active" ? 0.34 : 0.25;
      }
      unit.gaitOffsetX = spriteOffsetX;
      unit.gaitOffsetY = spriteOffsetY;
      unit.group.position.set(
        unit.currentPoint.x + offsetX,
        unit.currentPoint.y + offsetY
      );
      unit.group.zIndex = unit.currentPoint.y;
      unit.label.position.set(unit.currentPoint.x, unit.currentPoint.y - 137);
      unit.label.zIndex = 20_000 + unit.currentPoint.y;
      unit.sprite.alpha = 1;
      unit.blendSprite.alpha = 0;
      unit.animationBlend = 0;

      if (moving) {
        const movement = movementFacing(unit.currentPoint, unit.targetPoint);
        unit.facing = movement.facing;
        unit.flipX = movement.flipX;
        applyVisibleStepAppearance(
          unit,
          this.frameSets,
          unit.facing,
          gaitProgress,
          spriteOffsetX,
          spriteOffsetY
        );
        unit.animationKind = "walk";
        unit.resolvedAtlasFrameIndex = null;
        unit.poseFallback = unit.pose !== "neutral";
      } else {
        unit.sprite.texture = unit.restTexture;
        unit.frameScale = unit.restFrameScale;
        unit.sprite.position.set(spriteOffsetX, spriteOffsetY);
        unit.sprite.roundPixels = !inPlaceWalking;
        unit.blendSprite.position.set(spriteOffsetX, spriteOffsetY);
        unit.blendSprite.roundPixels = !inPlaceWalking;
        unit.animationKind =
          !this.reducedMotion && unit.visualState === "idle"
            ? "idle"
            : !this.reducedMotion && unit.visualState === "working"
              ? "working"
              : !this.reducedMotion && unit.visualState === "blocked"
                ? "blocked"
                : "rest";
        unit.animationFrameIndex = null;
        unit.sourceFrameIndex = null;
        unit.resolvedAtlasFrameIndex = unit.restResolvedAtlasFrameIndex;
        unit.poseFallback = unit.restPoseFallback;
        unit.flipX = false;
        if (unit.animationKind === "idle" && inPlaceWalking) {
          unit.facing = "down";
          applyVisibleStepAppearance(
            unit,
            this.frameSets,
            unit.facing,
            gaitProgress,
            spriteOffsetX,
            spriteOffsetY
          );
          unit.resolvedAtlasFrameIndex = null;
        }
      }
      unit.sprite.scale.set(
        (unit.flipX ? -1 : 1) * AGENT_SPRITE_SCALE * unit.frameScale,
        AGENT_SPRITE_SCALE * unit.frameScale
      );
      unit.blendSprite.scale.set(
        (unit.flipX ? -1 : 1) * AGENT_SPRITE_SCALE * unit.frameScale,
        AGENT_SPRITE_SCALE * unit.frameScale
      );
    }

    for (const packet of this.packets.values()) {
      const travel = ((elapsedMilliseconds / 2700 + packet.phase) % 1 + 1) % 1;
      const eased = travel < 0.5 ? 2 * travel * travel : 1 - Math.pow(-2 * travel + 2, 2) / 2;
      packet.graphic.position.set(
        packet.from.x + (packet.to.x - packet.from.x) * eased,
        packet.from.y + (packet.to.y - packet.from.y) * eased
      );
      packet.graphic.rotation = Math.atan2(packet.to.y - packet.from.y, packet.to.x - packet.from.x);
      packet.graphic.alpha = 0.28 + Math.sin(travel * Math.PI) * 0.72;
    }
    this.updateApprover(deltaMilliseconds);
    if (this.validationStation && !this.reducedMotion) {
      const pulse = 0.82 + Math.sin(elapsedMilliseconds * 0.004) * 0.18;
      this.validationStation.beacon.alpha = this.validation.approver.state === "idle"
        ? 0.42
        : pulse;
    }
    this.renderCount += 1;
  }

  setZoomLevel(zoomLevel: SemanticZoomLevel, cameraZoom: number): void {
    if (this.zoomLevel === zoomLevel && Math.abs(this.cameraZoom - cameraZoom) < 0.005) return;
    this.zoomLevel = zoomLevel;
    this.cameraZoom = cameraZoom;
    this.syncZoomPresentation();
  }

  inspectScene(ready: boolean): SceneInspection {
    const activeUnits = [...this.units.values()].filter(
      (unit) => unit.presenceKind === "active"
    );
    const standbyUnits = [...this.units.values()].filter(
      (unit) => unit.presenceKind === "configured_standby"
    );
    return {
      ready,
      actorCount: activeUnits.length,
      characterCount: this.units.size,
      standbyCount: standbyUnits.length,
      ambientActivityCount: standbyUnits.filter(
        (unit) => unit.ambientActivity
      ).length,
      packetCount: this.packets.size,
      semanticMotionCount: this.reducedMotion
        ? 0
        : [...this.units.values()].filter(
            (unit) =>
              unit.presenceKind === "active" &&
              (unit.visualState !== "idle" ||
                distance(unit.currentPoint, unit.targetPoint) > 1.4)
          ).length,
      movementSpeedRatio: AGENT_MOVEMENT_SPEED_RATIO,
      renderCount: this.renderCount,
      planningRevision: this.planningRevision,
      zoomLevel: this.zoomLevel,
      cameraZoom: this.cameraZoom,
      validation: {
        stationState: this.validation.stationState,
        severity: this.validation.severity,
        label: this.validation.label,
        recordKey: this.validation.recordKey,
        phase: this.validation.phase,
        packetKind: this.validation.packetKind,
        packetVisible: [...this.packets.keys()].some((key) => key.startsWith("validation:")),
        workerLeaseActive: this.validation.workerLeaseActive,
        approver: (() => {
          const visual = this.approverVisual;
          const snapshot = visual?.snapshot
            ?? new PrApproverAnimator(this.validation.approver).snapshot(this.reducedMotion);
          const spriteBounds = visual?.sprite.getBounds();
          const boardPoint = visual?.group.getGlobalPosition();
          return {
            state: snapshot.state,
            qualifier: snapshot.qualifier,
            clip: snapshot.clip,
            frameIndex: snapshot.frameIndex,
            sourceFrameIndex: snapshot.sourceFrameIndex,
            settled: snapshot.settled,
            recordKey: this.validation.approver.recordKey,
            pullRequestNumber: this.validation.approver.pullRequestNumber,
            transitionKey: snapshot.transitionKey,
            visible: visual?.group.visible ?? false,
            reducedMotion: this.reducedMotion,
            displayWidth: Math.round(spriteBounds?.width ?? 0),
            displayHeight: Math.round(spriteBounds?.height ?? 0),
            boardX: Math.round(boardPoint?.x ?? 0),
            boardY: Math.round(boardPoint?.y ?? 0),
          };
        })(),
      },
      actors: activeUnits.map((unit) => {
        const boardPoint = unit.group.getGlobalPosition();
        const spriteBounds = unit.sprite.getBounds();
        return {
          actorId: unit.actorId,
          agentId: unit.agentId,
          slotId: unit.slotId,
          visualState: unit.visualState,
          pose: unit.pose,
          animationKind: unit.animationKind,
          animationFrameIndex: unit.animationFrameIndex,
          sourceFrameIndex: unit.sourceFrameIndex,
          animationBlend: Number(unit.animationBlend.toFixed(3)),
          gaitOffsetX: Number(unit.gaitOffsetX.toFixed(2)),
          gaitOffsetY: Number(unit.gaitOffsetY.toFixed(2)),
          frameScale: Number(unit.frameScale.toFixed(3)),
          resolvedAtlasFrameIndex: unit.resolvedAtlasFrameIndex,
          poseFallback: unit.poseFallback,
          displayWidth: Math.round(spriteBounds.width),
          displayHeight: Math.round(spriteBounds.height),
          opacity: unit.group.alpha,
          boardX: Math.round(boardPoint.x),
          boardY: Math.round(boardPoint.y),
          x: Math.round(unit.currentPoint.x),
          y: Math.round(unit.currentPoint.y),
        };
      }),
      standbyCharacters: standbyUnits.map((unit) => {
        const boardPoint = unit.group.getGlobalPosition();
        const spriteBounds = unit.sprite.getBounds();
        return {
          characterId: unit.key.replace(/^standby:/, ""),
          presenceKind: "configured_standby",
          ambientActivity: unit.ambientActivity,
          agentId: unit.agentId,
          visualState: unit.visualState,
          pose: unit.pose,
          animationKind: unit.animationKind,
          animationFrameIndex: unit.animationFrameIndex,
          sourceFrameIndex: unit.sourceFrameIndex,
          animationBlend: Number(unit.animationBlend.toFixed(3)),
          gaitOffsetX: Number(unit.gaitOffsetX.toFixed(2)),
          gaitOffsetY: Number(unit.gaitOffsetY.toFixed(2)),
          frameScale: Number(unit.frameScale.toFixed(3)),
          locationIndex: unit.locationIndex,
          resolvedAtlasFrameIndex: unit.resolvedAtlasFrameIndex,
          poseFallback: unit.poseFallback,
          displayWidth: Math.round(spriteBounds.width),
          displayHeight: Math.round(spriteBounds.height),
          opacity: unit.group.alpha,
          boardX: Math.round(boardPoint.x),
          boardY: Math.round(boardPoint.y),
          x: Math.round(unit.currentPoint.x),
          y: Math.round(unit.currentPoint.y),
        };
      }),
    };
  }

  destroy(): void {
    this.reducedMotionQuery?.removeEventListener("change", this.onReducedMotionChange);
    this.units.clear();
    this.packets.clear();
    this.root.destroy({ children: true });
  }

  private buildScene(): void {
    const background = new Sprite(this.mapTexture);
    background.position.set(0, 0);
    background.width = MAP_WIDTH;
    background.height = MAP_HEIGHT;
    background.zIndex = 0;
    this.root.addChild(background);

    this.packetLayer.zIndex = 7_500;
    this.agentLayer.zIndex = 8_000;
    this.poiLayer.zIndex = 12_000;
    this.labelLayer.zIndex = 20_000;
    this.root.addChild(this.packetLayer, this.agentLayer);
    this.buildApprover();

    const foreground = new Sprite(this.mapTexture);
    foreground.position.set(0, 0);
    foreground.width = MAP_WIDTH;
    foreground.height = MAP_HEIGHT;
    foreground.zIndex = 10_000;
    const occlusionMask = createMapOcclusionMask();
    occlusionMask.zIndex = 9_999;
    foreground.mask = occlusionMask;
    this.root.addChild(occlusionMask, foreground, this.poiLayer, this.labelLayer);
    this.buildValidationStation();
    this.buildPointsOfInterest();
  }

  private buildApprover(): void {
    const animator = new PrApproverAnimator(this.validation.approver);
    const snapshot = animator.snapshot(this.reducedMotion);
    const sprite = new Sprite(
      this.approverFrames[snapshot.sourceFrameIndex] ?? this.approverFrames[0]
    );
    sprite.anchor.set(0.5, 1);
    sprite.scale.set(PR_APPROVER_SPRITE_SCALE);
    sprite.roundPixels = true;

    const stateMarker = new Graphics();
    const shadow = new Graphics()
      .ellipse(0, -2, 27, 7.5)
      .fill({ color: 0x000000, alpha: 0.34 });
    const group = new Container();
    group.position.set(PR_APPROVER_POINT.x, PR_APPROVER_POINT.y);
    group.zIndex = PR_APPROVER_POINT.y;
    group.eventMode = "static";
    group.cursor = "pointer";
    group.hitArea = new Rectangle(-50, -152, 100, 158);
    group.addChild(stateMarker, shadow, sprite);
    group.on("pointertap", () => { this.requestApproverSelection(); });
    group.on("pointerover", () => { stateMarker.alpha = 1; });
    group.on("pointerout", () => { stateMarker.alpha = 0.7; });
    this.agentLayer.addChild(group);

    // The approval desk sits inside DELIVERY's broad point-of-interest hit
    // region. Keep an invisible, higher-priority interaction target in the POI
    // layer so selecting the officer always opens this exact validation record
    // instead of the distributor drawer underneath.
    const interactionTarget = new Container();
    interactionTarget.position.set(PR_APPROVER_POINT.x, PR_APPROVER_POINT.y);
    interactionTarget.zIndex = 2_100;
    interactionTarget.eventMode = "static";
    interactionTarget.cursor = "pointer";
    interactionTarget.hitArea = new Rectangle(-50, -152, 100, 158);
    interactionTarget.on("pointertap", () => { this.requestApproverSelection(); });
    interactionTarget.on("pointerover", () => { stateMarker.alpha = 1; });
    interactionTarget.on("pointerout", () => { stateMarker.alpha = 0.7; });
    this.poiLayer.addChild(interactionTarget);
    this.approverVisual = {
      group,
      interactionTarget,
      sprite,
      shadow,
      stateMarker,
      animator,
      snapshot,
    };
    this.syncApproverPresentation();
  }

  private requestApproverSelection(): void {
    const recordKey = this.validation.approver.recordKey;
    if (!recordKey) return;
    dispatchSceneSelection({
      kind: "poi",
      detailTarget: "validation",
      recordKey,
    });
  }

  private updateApprover(deltaMilliseconds: number): void {
    const visual = this.approverVisual;
    if (!visual) return;
    const next = visual.animator.update(deltaMilliseconds, this.reducedMotion);
    if (next.sourceFrameIndex !== visual.snapshot.sourceFrameIndex) {
      visual.sprite.texture = this.approverFrames[next.sourceFrameIndex]
        ?? this.approverFrames[0]
        ?? Texture.EMPTY;
    }
    visual.snapshot = next;
  }

  private syncApproverPresentation(): void {
    const visual = this.approverVisual;
    if (!visual) return;
    const projection = this.validation.approver;
    const snapshot = visual.animator.snapshot(this.reducedMotion);
    const color = approverColor(projection);
    visual.snapshot = snapshot;
    visual.sprite.texture = this.approverFrames[snapshot.sourceFrameIndex]
      ?? this.approverFrames[0]
      ?? Texture.EMPTY;
    visual.group.visible = true;
    visual.group.alpha = projection.state === "idle" ? 0.84 : 1;
    visual.group.cursor = projection.recordKey ? "pointer" : "default";
    visual.interactionTarget.cursor = projection.recordKey ? "pointer" : "default";
    visual.group.position.set(PR_APPROVER_POINT.x, PR_APPROVER_POINT.y);
    visual.sprite.position.set(0, 0);
    visual.sprite.scale.set(PR_APPROVER_SPRITE_SCALE);
    visual.shadow.alpha = projection.state === "idle" ? 0.24 : 0.34;
    visual.shadow.scale.set(1);
    visual.stateMarker
      .clear()
      .ellipse(0, -2, 33, 10)
      .stroke({ width: 2, color, alpha: projection.state === "idle" ? 0.42 : 0.82 })
      .ellipse(0, -2, 27, 7.5)
      .stroke({ width: 1, color, alpha: 0.36 });
    visual.stateMarker.alpha = 0.7;
  }

  private buildValidationStation(): void {
    const group = new Container();
    group.position.set(QA_CI_STATION_POINT.x, QA_CI_STATION_POINT.y);
    group.zIndex = 2_000;
    group.eventMode = "static";
    group.cursor = "pointer";
    group.hitArea = new Rectangle(-102, -58, 204, 116);

    const panel = new Graphics();
    const beacon = new Graphics();
    const caption = new Text({
      text: "PR APPROVER",
      style: {
        fontFamily: "Galmuri11, monospace",
        fontSize: 9,
        fontWeight: "bold",
        fill: 0x98abc4,
        letterSpacing: 1.5,
        stroke: { color: 0x03101d, width: 2 },
      },
    });
    caption.anchor.set(0, 0.5);
    caption.position.set(-84, -39);
    const label = new Text({
      text: "통합 PR 대기",
      style: {
        fontFamily: "Galmuri11, monospace",
        fontSize: 13,
        fontWeight: "bold",
        fill: 0xf2f7ff,
        letterSpacing: 1.2,
        stroke: { color: 0x03101d, width: 3 },
      },
    });
    label.anchor.set(0, 0.5);
    label.position.set(-84, -13);
    const modeLabel = new Text({
      text: "CHECKS 0/0 · FINDINGS 0",
      style: {
        fontFamily: "Galmuri11, monospace",
        fontSize: 9,
        fontWeight: "bold",
        fill: 0x98abc4,
        letterSpacing: 0.8,
        stroke: { color: 0x03101d, width: 2 },
      },
    });
    modeLabel.anchor.set(0, 0.5);
    modeLabel.position.set(-84, 13);
    const progressTrack = new Graphics();
    const progressFill = new Graphics();
    group.addChild(panel, beacon, caption, label, modeLabel, progressTrack, progressFill);
    group.on("pointertap", () => { this.requestApproverSelection(); });
    group.on("pointerover", () => { panel.alpha = 1; });
    group.on("pointerout", () => { panel.alpha = 0.92; });
    this.poiLayer.addChild(group);
    this.validationStation = {
      group,
      panel,
      beacon,
      caption,
      label,
      modeLabel,
      progressTrack,
      progressFill,
    };
    this.syncValidationStation();
  }

  private syncValidationStation(): void {
    const station = this.validationStation;
    if (!station) return;
    const approver = this.validation.approver;
    const color = approverColor(approver);
    station.panel
      .clear()
      .roundRect(-96, -52, 192, 104, 8)
      .fill({ color: 0x04131f, alpha: 0.86 })
      .stroke({ width: 2.5, color, alpha: 0.86 })
      .roundRect(-92, -48, 4, 96, 2)
      .fill({ color, alpha: 0.78 });
    station.panel.alpha = 0.92;
    station.beacon
      .clear()
      .circle(77, -38, 6)
      .fill({ color, alpha: approver.state === "idle" ? 0.32 : 0.9 })
      .circle(77, -38, 10)
      .stroke({ width: 1.5, color, alpha: 0.42 });
    station.caption.text = approver.evidenceShortSha
      ? `PR APPROVER · ${approver.evidenceShortSha}`
      : "PR APPROVER";
    station.label.text = approver.pullRequestNumber
      ? `PR #${approver.pullRequestNumber} · ${approverStateLabel(approver)}`
      : approverStateLabel(approver);
    station.label.style.fill = color;
    const unresolvedFindings = Math.max(
      0,
      approver.findingCount - approver.remediationCount
    );
    station.modeLabel.text = `${approver.requiredChecksSucceeded}/${approver.requiredChecksTotal} CHECKS · ${unresolvedFindings} FINDING${unresolvedFindings === 1 ? "" : "S"}`;
    station.modeLabel.style.fill = approver.state === "failure"
      ? STATUS_PALETTE.danger
      : STATUS_PALETTE.muted;
    const progress = approver.requiredChecksTotal > 0
      ? Math.min(1, approver.requiredChecksSucceeded / approver.requiredChecksTotal)
      : approver.state === "success" ? 1 : 0;
    station.progressTrack
      .clear()
      .roundRect(-84, 34, 168, 5, 2.5)
      .fill({ color: 0x17304a, alpha: 0.9 });
    station.progressFill.clear();
    if (progress > 0) {
      station.progressFill
        .roundRect(-84, 34, Math.max(5, 168 * progress), 5, 2.5)
        .fill({ color, alpha: 0.96 });
    }
    station.group.alpha = approver.state === "idle" ? 0.78 : 1;
    station.group.cursor = approver.recordKey ? "pointer" : "default";
  }

  private buildPointsOfInterest(): void {
    for (const spec of POINTS_OF_INTEREST) {
      const group = new Container();
      group.position.set(spec.x, spec.y);
      group.eventMode = "static";
      group.cursor = "pointer";
      group.hitArea = new Rectangle(
        -spec.width / 2,
        -spec.height / 2,
        spec.width,
        spec.height
      );
      const bracket = new Graphics()
        .roundRect(-52, -18, 104, 36, 8)
        .fill({ color: 0x04131f, alpha: 0.56 })
        .stroke({ width: 2, color: spec.color, alpha: 0.52 });
      bracket.alpha = 0.42;
      const label = new Text({
        text: spec.label,
        style: {
          fontFamily: "Galmuri11, monospace",
          fontSize: 13,
          fontWeight: "bold",
          fill: spec.color,
          letterSpacing: 1.4,
          stroke: { color: 0x03101d, width: 3 },
        },
      });
      label.anchor.set(0.5);
      group.addChild(bracket, label);
      group.on("pointerover", () => {
        bracket.alpha = 0.96;
        label.alpha = 1;
      });
      group.on("pointerout", () => {
        bracket.alpha = 0.42;
        label.alpha = this.zoomLevel === "detail" ? 1 : 0.8;
      });
      group.on("pointertap", () => {
        dispatchSceneSelection({ kind: "poi", detailTarget: spec.detailTarget });
      });
      this.poiLayer.addChild(group);
      this.poiLabels.push(label);
    }
  }

  private createUnit(
    key: string,
    projection: CharacterProjection,
    currentPoint: Point,
    homePoint: Point,
    targetPoint: Point,
    index: number
  ): AgentUnit {
    const presenceKind: PresenceKind =
      "actorId" in projection ? "active" : "configured_standby";
    const standbyRoute =
      "locationIndex" in projection
        ? standbyPatrolRoute(projection.locationIndex)
        : [];
    const ambientActivity = !this.reducedMotion && standbyRoute.length > 1;
    const standbyPatrolIndex = ambientActivity ? 1 : 0;
    const archetype = archetypeForProfile(projection.archetypeKey);
    const resolved = resolveRestTexture(
      this.atlasTexture,
      this.frameSets,
      archetype,
      projection.visualState,
      projection.staticPose
    );
    const sprite = new Sprite(
      resolved.texture ?? frameForFacing(this.frameSets, archetype, "down", 0)
    );
    sprite.roundPixels = true;
    sprite.anchor.set(0.5, 1);
    sprite.scale.set(AGENT_SPRITE_SCALE);
    const blendSprite = new Sprite(sprite.texture);
    blendSprite.alpha = 0;
    blendSprite.roundPixels = true;
    blendSprite.anchor.set(0.5, 1);
    blendSprite.scale.set(AGENT_SPRITE_SCALE);
    const shadow = new Graphics()
      .ellipse(0, -2, 29, 8)
      .fill({ color: 0x000000, alpha: presenceKind === "active" ? 0.34 : 0.25 });
    const marker = new Graphics();
    marker.position.set(0, -2);
    const group = new Container();
    group.position.set(currentPoint.x, currentPoint.y);
    group.eventMode = "static";
    group.cursor = presenceKind === "active" ? "pointer" : "default";
    group.hitArea = new Rectangle(-52, -142, 104, 148);
    group.addChild(shadow, marker, sprite, blendSprite);

    const label = new Text({
      text: projection.displayName,
      style: {
        fontFamily: "Galmuri11, monospace",
        fontSize: 14,
        fontWeight: "bold",
        fill: 0xf2f7ff,
        align: "center",
        stroke: { color: 0x03101d, width: 4 },
      },
    });
    label.anchor.set(0.5, 1);
    label.position.set(currentPoint.x, currentPoint.y - 137);

    const unit: AgentUnit = {
      key,
      presenceKind,
      actorId: "actorId" in projection ? projection.actorId : "",
      agentId: projection.agentId,
      slotId: "slotId" in projection ? projection.slotId : "",
      seatIndex: "seatIndex" in projection ? projection.seatIndex : 0,
      locationIndex: "locationIndex" in projection ? projection.locationIndex : 0,
      displayName: projection.displayName,
      visualState: projection.visualState,
      pose: projection.staticPose,
      severity: projection.severity,
      bubbleLabel: projection.bubbleLabel,
      archetype,
      group,
      sprite,
      blendSprite,
      shadow,
      marker,
      label,
      homePoint: copyPoint(homePoint),
      currentPoint: copyPoint(currentPoint),
      targetPoint:
        ambientActivity
          ? copyPoint(standbyRoute[standbyPatrolIndex] ?? targetPoint)
          : copyPoint(targetPoint),
      facing: "down",
      flipX: false,
      motionPhase: (stableNumber(key) + index) % 17,
      hovered: false,
      animationKind: "rest",
      animationFrameIndex: null,
      sourceFrameIndex: null,
      animationBlend: 0,
      gaitOffsetX: 0,
      gaitOffsetY: 0,
      restTexture:
        resolved.texture ?? frameForFacing(this.frameSets, archetype, "down", 0),
      restFrameScale: resolved.frameScale,
      frameScale: resolved.frameScale,
      restResolvedAtlasFrameIndex: resolved.resolvedAtlasFrameIndex,
      restPoseFallback: resolved.poseFallback,
      resolvedAtlasFrameIndex: resolved.resolvedAtlasFrameIndex,
      poseFallback: resolved.poseFallback,
      standbyPatrolRoute: standbyRoute,
      standbyPatrolIndex,
      ambientActivity,
    };
    group.on("pointerover", () => {
      unit.hovered = true;
      this.syncUnitLabel(unit);
    });
    group.on("pointerout", () => {
      unit.hovered = false;
      this.syncUnitLabel(unit);
    });
    group.on("pointertap", () => {
      if (unit.presenceKind === "active" && unit.actorId) {
        dispatchSceneSelection({ kind: "actor", actorId: unit.actorId });
      } else {
        dispatchSceneSelection({ kind: "poi", detailTarget: "standby" });
      }
    });
    this.agentLayer.addChild(group);
    this.labelLayer.addChild(label);
    this.syncUnitAppearance(unit);
    return unit;
  }

  private syncUnitAppearance(unit: AgentUnit): void {
    const resolved = resolveRestTexture(
      this.atlasTexture,
      this.frameSets,
      unit.archetype,
      unit.visualState,
      unit.pose
    );
    if (resolved.texture) unit.restTexture = resolved.texture;
    unit.restFrameScale = resolved.frameScale;
    unit.restResolvedAtlasFrameIndex = resolved.resolvedAtlasFrameIndex;
    unit.restPoseFallback = resolved.poseFallback;
    const color = STATUS_PALETTE[unit.severity];
    drawStaticMarker(unit.marker, unit.visualState, color, unit.presenceKind);
    this.syncUnitLabel(unit);
  }

  private advanceStandbyPatrol(unit: AgentUnit): void {
    if (
      !unit.ambientActivity ||
      unit.presenceKind !== "configured_standby" ||
      unit.standbyPatrolRoute.length < 2 ||
      distance(unit.currentPoint, unit.targetPoint) > 1.4
    ) {
      return;
    }
    unit.standbyPatrolIndex =
      (unit.standbyPatrolIndex + 1) % unit.standbyPatrolRoute.length;
    unit.targetPoint = copyPoint(
      unit.standbyPatrolRoute[unit.standbyPatrolIndex] ?? unit.homePoint
    );
  }

  private travelSpeedFor(unit: AgentUnit): number {
    return unit.presenceKind === "configured_standby"
      ? STANDBY_LOUNGE_TRAVEL_SPEED_WORLD_PX_PER_SECOND
      : AGENT_TRAVEL_SPEED_WORLD_PX_PER_SECOND;
  }

  private syncUnitLabel(unit: AgentUnit): void {
    const showStatus = this.zoomLevel === "detail" || unit.hovered;
    unit.label.text = showStatus
      ? `${unit.displayName}\n${STATE_LABELS[unit.visualState]}`
      : unit.displayName;
    unit.label.style.fill = STATUS_PALETTE[unit.severity];
    unit.label.visible =
      unit.hovered ||
      this.zoomLevel === "detail" ||
      (this.zoomLevel === "operations" && unit.presenceKind === "active");
  }

  private rebuildSignalPackets(): void {
    const retained = new Set<string>();
    for (const unit of this.units.values()) {
      const target = relevantPacketTarget(unit);
      if (!target) continue;
      retained.add(unit.key);
      const existing = this.packets.get(unit.key);
      if (existing) {
        existing.from = copyPoint(unit.homePoint);
        existing.to = copyPoint(target);
        continue;
      }
      const color = STATUS_PALETTE[unit.severity];
      const graphic = new Graphics()
        .poly([0, -6, 8, 0, 0, 6, -8, 0])
        .fill({ color, alpha: 0.94 })
        .stroke({ width: 1.5, color: 0xffffff, alpha: 0.55 });
      graphic.zIndex = unit.homePoint.y;
      this.packetLayer.addChild(graphic);
      this.packets.set(unit.key, {
        unitKey: unit.key,
        graphic,
        from: copyPoint(unit.homePoint),
        to: copyPoint(target),
        phase: unit.motionPhase / 17,
      });
    }
    const validationSignal = this.validation.packetKind
      ? QA_CI_SIGNAL_POINTS[this.validation.packetKind]
      : null;
    if (validationSignal && this.validation.recordKey) {
      const validationKey = `validation:${this.validation.recordKey}:${this.validation.packetKind}:${this.validation.severity}`;
      retained.add(validationKey);
      const existing = this.packets.get(validationKey);
      if (existing) {
        existing.from = copyPoint(validationSignal.from);
        existing.to = copyPoint(validationSignal.to);
      } else {
        const color = STATUS_PALETTE[this.validation.severity];
        const graphic = new Graphics()
          .roundRect(-8, -5, 16, 10, 2)
          .fill({ color, alpha: 0.96 })
          .stroke({ width: 1.5, color: 0xffffff, alpha: 0.72 })
          .moveTo(-4, 0)
          .lineTo(4, 0)
          .stroke({ width: 1, color: 0x04131f, alpha: 0.78 });
        graphic.zIndex = Math.max(validationSignal.from.y, validationSignal.to.y);
        this.packetLayer.addChild(graphic);
        this.packets.set(validationKey, {
          unitKey: validationKey,
          graphic,
          from: copyPoint(validationSignal.from),
          to: copyPoint(validationSignal.to),
          phase: stableNumber(validationKey) % 17 / 17,
        });
      }
    }
    for (const [key, packet] of this.packets) {
      if (retained.has(key)) continue;
      packet.graphic.destroy();
      this.packets.delete(key);
    }
  }

  private syncZoomPresentation(): void {
    for (const unit of this.units.values()) this.syncUnitLabel(unit);
    const showPoiLabels = this.zoomLevel !== "overview";
    for (const label of this.poiLabels) label.visible = showPoiLabels;
    if (this.validationStation) {
      this.validationStation.caption.visible = showPoiLabels;
      this.validationStation.modeLabel.visible = showPoiLabels;
      this.validationStation.progressTrack.visible = showPoiLabels;
      this.validationStation.progressFill.visible = showPoiLabels;
    }
  }
}

export {
  CLEANUP_STATION_POINTS,
  DELIVERY_STATION_POINTS,
  REVIEW_STATION_POINTS,
  SLOT_SEATS,
  STANDBY_LOUNGE_POINTS,
};
