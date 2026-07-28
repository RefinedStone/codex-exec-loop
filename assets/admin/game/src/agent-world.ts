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
  type AgentFrameSet,
} from "./agent-atlas";
import type {
  AgentAnimationKind,
  ArchetypeKey,
  DashboardSceneSnapshot,
  Facing,
  GameActorProjection,
  GameStandbyProjection,
  Point,
  PresenceKind,
  SceneInspection,
  SemanticZoomLevel,
  StaticPose,
  StatusSeverity,
  VisualState,
} from "./game-types";
import {
  AGENT_SPRITE_SCALE,
  CLEANUP_STATION_POINTS,
  DELIVERY_STATION_POINTS,
  MAP_HEIGHT,
  MAP_WIDTH,
  OCCLUSION_POLYGONS,
  POINTS_OF_INTEREST,
  REVIEW_STATION_POINTS,
  SLOT_SEATS,
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
  animationBlend: number;
  gaitOffsetX: number;
  gaitOffsetY: number;
  restTexture: Texture;
  restResolvedAtlasFrameIndex: number | null;
  restPoseFallback: boolean;
  resolvedAtlasFrameIndex: number | null;
  poseFallback: boolean;
}

interface SignalPacket {
  unitKey: string;
  graphic: Graphics;
  from: Point;
  to: Point;
  phase: number;
}

const distance = (from: Point, to: Point): number =>
  Math.hypot(to.x - from.x, to.y - from.y);

const stableNumber = (value: string): number =>
  [...value].reduce((sum, character) => sum + character.charCodeAt(0), 0);

const copyPoint = (point: Point): Point => ({ x: point.x, y: point.y });

export const AGENT_MOVEMENT_SPEED_RATIO = 0.3;
export const AGENT_TRAVEL_SPEED_WORLD_PX_PER_SECOND = 168;
export const WALK_IN_PLACE_CYCLE_MS = 760;
export const IDLE_IN_PLACE_CYCLE_MS = 960;
export const IDLE_IN_PLACE_AMPLITUDE_RATIO = 0.65;
export const WALK_SWAY_WORLD_PX = 0.7;
export const WALK_LIFT_WORLD_PX = 1.6;
export const STEP_CROSSFADE_START = 0.68;

const MAX_MOVEMENT_DELTA_MS = 50;

const visibleStepState = (
  gaitProgress: number
): { frameIndex: number; nextFrameIndex: number; blend: number } => {
  const frameProgress = gaitProgress * 4;
  const frameBase = Math.floor(frameProgress);
  const frameFraction = frameProgress - frameBase;
  const rawBlend = Math.max(
    0,
    Math.min(
      1,
      (frameFraction - STEP_CROSSFADE_START) /
        (1 - STEP_CROSSFADE_START)
    )
  );
  const blend = rawBlend * rawBlend * (3 - 2 * rawBlend);
  return {
    frameIndex: frameBase % 4,
    nextFrameIndex: (frameBase + 1) % 4,
    blend,
  };
};

const applyVisibleStepAppearance = (
  unit: AgentUnit,
  frameSets: Record<ArchetypeKey, AgentFrameSet>,
  facing: Facing,
  gaitProgress: number,
  gaitOffsetX: number,
  gaitOffsetY: number
): void => {
  const step = visibleStepState(gaitProgress);
  const currentAlignment = alignmentForFacing(
    unit.archetype,
    facing,
    step.frameIndex
  );
  const nextAlignment = alignmentForFacing(
    unit.archetype,
    facing,
    step.nextFrameIndex
  );
  unit.sprite.texture = frameForFacing(
    frameSets,
    unit.archetype,
    facing,
    step.frameIndex
  );
  unit.blendSprite.texture = frameForFacing(
    frameSets,
    unit.archetype,
    facing,
    step.nextFrameIndex
  );
  unit.sprite.position.set(
    gaitOffsetX + currentAlignment.x * AGENT_SPRITE_SCALE,
    gaitOffsetY + currentAlignment.y * AGENT_SPRITE_SCALE
  );
  unit.blendSprite.position.set(
    gaitOffsetX + nextAlignment.x * AGENT_SPRITE_SCALE,
    gaitOffsetY + nextAlignment.y * AGENT_SPRITE_SCALE
  );
  unit.sprite.alpha = Math.sqrt(1 - step.blend);
  unit.blendSprite.alpha = Math.sqrt(step.blend);
  unit.sprite.roundPixels = false;
  unit.blendSprite.roundPixels = false;
  unit.animationFrameIndex = step.frameIndex;
  unit.animationBlend = step.blend;
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
    | { kind: "poi"; detailTarget: string }
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
  private readonly agentLayer = new Container({ sortableChildren: true });
  private readonly packetLayer = new Container({ sortableChildren: true });
  private readonly labelLayer = new Container({ sortableChildren: true });
  private readonly poiLayer = new Container({ sortableChildren: true });
  private readonly units = new Map<string, AgentUnit>();
  private readonly packets = new Map<string, SignalPacket>();
  private readonly poiLabels: Text[] = [];
  private planningRevision: number | null = null;
  private zoomLevel: SemanticZoomLevel = "overview";
  private cameraZoom = 1;
  private renderCount = 0;
  private reducedMotion =
    window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;

  constructor(mapTexture: Texture, atlasTexture: Texture) {
    this.mapTexture = mapTexture;
    this.atlasTexture = atlasTexture;
    this.frameSets = buildAgentFrameSets(atlasTexture);
    this.buildScene();
  }

  reconcile(snapshot: DashboardSceneSnapshot): void {
    this.planningRevision = snapshot.planningRevision;
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
        existing.targetPoint = copyPoint(targetPoint);
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
      const remaining = distance(unit.currentPoint, unit.targetPoint);
      let moving = remaining > 1.4;
      if (moving && this.reducedMotion) {
        unit.currentPoint = copyPoint(unit.targetPoint);
        moving = false;
      } else if (moving) {
        const travelStep = Math.min(
          remaining,
          AGENT_TRAVEL_SPEED_WORLD_PX_PER_SECOND * movementDeltaSeconds
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
        unit.shadow.scale.set(
          1 - lift * 0.075 * gaitAmplitude,
          1 - lift * 0.035 * gaitAmplitude
        );
        unit.shadow.alpha =
          (unit.presenceKind === "active" ? 0.34 : 0.25) -
          lift * 0.055 * gaitAmplitude;
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
        (unit.flipX ? -1 : 1) * AGENT_SPRITE_SCALE,
        AGENT_SPRITE_SCALE
      );
      unit.blendSprite.scale.set(
        (unit.flipX ? -1 : 1) * AGENT_SPRITE_SCALE,
        AGENT_SPRITE_SCALE
      );
    }

    for (const packet of this.packets.values()) {
      const travel = ((elapsedMilliseconds / 2700 + packet.phase) % 1 + 1) % 1;
      const eased = travel < 0.5 ? 2 * travel * travel : 1 - Math.pow(-2 * travel + 2, 2) / 2;
      packet.graphic.position.set(
        packet.from.x + (packet.to.x - packet.from.x) * eased,
        packet.from.y + (packet.to.y - packet.from.y) * eased
      );
      packet.graphic.alpha = 0.28 + Math.sin(travel * Math.PI) * 0.72;
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
      packetCount: this.packets.size,
      semanticMotionCount: this.reducedMotion
        ? 0
        : [...this.units.values()].filter(
            (unit) =>
              unit.visualState !== "idle" ||
              distance(unit.currentPoint, unit.targetPoint) > 1.4
          ).length,
      movementSpeedRatio: AGENT_MOVEMENT_SPEED_RATIO,
      renderCount: this.renderCount,
      planningRevision: this.planningRevision,
      zoomLevel: this.zoomLevel,
      cameraZoom: this.cameraZoom,
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
          animationBlend: Number(unit.animationBlend.toFixed(3)),
          gaitOffsetX: Number(unit.gaitOffsetX.toFixed(2)),
          gaitOffsetY: Number(unit.gaitOffsetY.toFixed(2)),
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
          agentId: unit.agentId,
          visualState: unit.visualState,
          pose: unit.pose,
          animationKind: unit.animationKind,
          animationFrameIndex: unit.animationFrameIndex,
          animationBlend: Number(unit.animationBlend.toFixed(3)),
          gaitOffsetX: Number(unit.gaitOffsetX.toFixed(2)),
          gaitOffsetY: Number(unit.gaitOffsetY.toFixed(2)),
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

    const foreground = new Sprite(this.mapTexture);
    foreground.position.set(0, 0);
    foreground.width = MAP_WIDTH;
    foreground.height = MAP_HEIGHT;
    foreground.zIndex = 10_000;
    const occlusionMask = createMapOcclusionMask();
    occlusionMask.zIndex = 9_999;
    foreground.mask = occlusionMask;
    this.root.addChild(occlusionMask, foreground, this.poiLayer, this.labelLayer);
    this.buildPointsOfInterest();
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
      targetPoint: copyPoint(targetPoint),
      facing: "down",
      flipX: false,
      motionPhase: (stableNumber(key) + index) % 17,
      hovered: false,
      animationKind: "rest",
      animationFrameIndex: null,
      animationBlend: 0,
      gaitOffsetX: 0,
      gaitOffsetY: 0,
      restTexture:
        resolved.texture ?? frameForFacing(this.frameSets, archetype, "down", 0),
      restResolvedAtlasFrameIndex: resolved.resolvedAtlasFrameIndex,
      restPoseFallback: resolved.poseFallback,
      resolvedAtlasFrameIndex: resolved.resolvedAtlasFrameIndex,
      poseFallback: resolved.poseFallback,
    };
    group.on("pointerover", () => {
      unit.hovered = true;
      unit.group.scale.set(1.055);
      this.syncUnitLabel(unit);
    });
    group.on("pointerout", () => {
      unit.hovered = false;
      unit.group.scale.set(1);
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
    unit.restResolvedAtlasFrameIndex = resolved.resolvedAtlasFrameIndex;
    unit.restPoseFallback = resolved.poseFallback;
    const color = STATUS_PALETTE[unit.severity];
    drawStaticMarker(unit.marker, unit.visualState, color, unit.presenceKind);
    this.syncUnitLabel(unit);
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
  }
}

export {
  CLEANUP_STATION_POINTS,
  DELIVERY_STATION_POINTS,
  REVIEW_STATION_POINTS,
  SLOT_SEATS,
  STANDBY_LOUNGE_POINTS,
};
