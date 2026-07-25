import "@pixi/unsafe-eval";
import * as PIXI_RUNTIME from "pixi.js";

type StatusSeverity = "normal" | "success" | "warning" | "danger" | "info" | "muted";
type VisualState =
  | "idle"
  | "starting"
  | "working"
  | "awaiting_review"
  | "blocked"
  | "delivering"
  | "cleanup";
type StaticPose = "neutral" | "laptop" | "callout" | "alert" | "sit";
type PresenceKind = "active" | "configured_standby";
type Facing = "down" | "side" | "up";
type ArchetypeKey = "planner" | "coffee_addict" | "ai_researcher" | "designer";
type AssetKey =
  | "fdDesk1"
  | "fdDesk2"
  | "fdDesk3"
  | "fdDesk4"
  | "fdDesk5"
  | "fdBossDesk"
  | "fdDistributorDesk"
  | "fdEventLogTower"
  | "fdSofa"
  | "fdPlant"
  | "agentAtlas";

interface Point {
  x: number;
  y: number;
}

interface PixiScale {
  set: (...values: number[]) => void;
}

interface PixiDisplayObject {
  x: number;
  y: number;
  alpha: number;
  zIndex: number;
  scale: PixiScale;
  destroy: (options?: { children?: boolean; texture?: boolean; baseTexture?: boolean }) => void;
}

interface PixiGraphics extends PixiDisplayObject {
  beginFill: (color: number, alpha?: number) => PixiGraphics;
  lineStyle: (width: number, color: number, alpha?: number) => PixiGraphics;
  drawPolygon: (points: number[]) => PixiGraphics;
  drawEllipse: (x: number, y: number, width: number, height: number) => PixiGraphics;
  endFill: () => PixiGraphics;
  clear: () => PixiGraphics;
  moveTo: (x: number, y: number) => PixiGraphics;
  lineTo: (x: number, y: number) => PixiGraphics;
}

interface PixiContainer extends PixiDisplayObject {
  sortableChildren: boolean;
  addChild: (...children: PixiDisplayObject[]) => void;
  removeChildren: () => PixiDisplayObject[];
}

interface PixiSprite extends PixiDisplayObject {
  anchor: PixiScale;
  texture: PixiTexture;
  width: number;
  height: number;
}

interface PixiTexture {
  baseTexture?: unknown;
}

interface PixiApplication {
  view: HTMLCanvasElement;
  stage: PixiContainer;
  renderer: {
    resize: (width: number, height: number) => void;
    render: (displayObject: PixiDisplayObject) => void;
  };
  destroy: (
    removeView?: boolean,
    options?: { children?: boolean; texture?: boolean; baseTexture?: boolean }
  ) => void;
}

interface AgentFrameSet {
  down: PixiTexture[];
  side: PixiTexture[];
  up: PixiTexture[];
}

interface AgentUnit {
  characterId: string;
  presenceKind: PresenceKind;
  actorId: string;
  agentId: string;
  slotId: string;
  visualState: VisualState;
  pose: StaticPose;
  node: HTMLElement;
  group: PixiContainer;
  sprite: PixiSprite | null;
  marker: PixiGraphics;
  homePoint: Point;
  point: Point;
  motionPhase: number;
  archetype: ArchetypeKey;
  resolvedAtlasFrameIndex: number | null;
  poseFallback: boolean;
}

interface SignalPacket {
  unit: AgentUnit;
  graphic: PixiGraphics;
  target: Point;
  phase: number;
}

interface StructureSpec {
  key: AssetKey;
  x: number;
  y: number;
  scale: number;
  anchorX?: number;
  anchorY?: number;
  zOffset?: number;
}

interface StructureSprite {
  spec: StructureSpec;
  sprite: PixiSprite;
}

interface SceneInspection {
  ready: boolean;
  actorCount: number;
  characterCount: number;
  standbyCount: number;
  packetCount: number;
  semanticMotionCount: number;
  renderCount: number;
  actors: Array<{
    actorId: string;
    agentId: string;
    slotId: string;
    visualState: VisualState;
    pose: StaticPose;
    resolvedAtlasFrameIndex: number | null;
    poseFallback: boolean;
    displayWidth: number;
    displayHeight: number;
    opacity: number;
    boardX: number;
    boardY: number;
    x: number;
    y: number;
  }>;
  standbyCharacters: Array<{
    characterId: string;
    presenceKind: "configured_standby";
    agentId: string;
    visualState: VisualState;
    pose: StaticPose;
    locationIndex: number;
    resolvedAtlasFrameIndex: number | null;
    poseFallback: boolean;
    displayWidth: number;
    displayHeight: number;
    opacity: number;
    boardX: number;
    boardY: number;
    x: number;
    y: number;
  }>;
}

interface DioramaHandle {
  app: PixiApplication;
  destroy: () => void;
  rebuildAgentUnits: () => void;
  syncLayout: () => void;
  inspectScene: () => SceneInspection;
}

interface AkraAdminGameBridge {
  mountDiorama?: () => DioramaHandle | null;
  inspectScene?: () => SceneInspection | null;
  [key: string]: unknown;
}

const AGENT_FRAME_WIDTH = 128;
const AGENT_FRAME_HEIGHT = 192;
const AGENT_SPRITE_SCALE = 0.72;
const AGENT_SHADOW_WIDTH = 26.35;
const AGENT_SHADOW_HEIGHT = 6.8;
const AGENT_MARKER_WIDTH = 21.25;
const AGENT_MARKER_HEIGHT = 5.95;
const MAP_WIDTH = 1672;
const MAP_HEIGHT = 940;

const SLOT_SEATS: Point[] = [
  { x: 742, y: 332 },
  { x: 742, y: 502 },
  { x: 742, y: 675 },
  { x: 610, y: 565 },
  { x: 900, y: 565 },
];

const STANDBY_LOUNGE_POINTS: Point[] = [
  { x: 225, y: 785 },
  { x: 320, y: 800 },
  { x: 405, y: 770 },
];

// The v2 ImageGen map already contains furniture. Pixi owns only live semantic overlays.
const STRUCTURE_SPECS: StructureSpec[] = [];

const REVIEW_STATION: Point = { x: 310, y: 245 };
const DELIVERY_STATION: Point = { x: 1295, y: 505 };
const CLEANUP_STATION: Point = { x: 1370, y: 765 };

const NEUTRAL_FRAME_MANIFEST: Record<VisualState, { facing: Facing; frameIndex: number }> = {
  idle: { facing: "down", frameIndex: 0 },
  starting: { facing: "side", frameIndex: 0 },
  working: { facing: "down", frameIndex: 0 },
  awaiting_review: { facing: "up", frameIndex: 0 },
  blocked: { facing: "side", frameIndex: 0 },
  delivering: { facing: "up", frameIndex: 0 },
  cleanup: { facing: "down", frameIndex: 0 },
};

// `null` is intentional: a visually similar emote must not be mislabeled as another pose.
const STATIC_POSE_MANIFEST: Record<ArchetypeKey, Record<StaticPose, number | null>> = {
  planner: { neutral: null, laptop: 40, callout: null, alert: 41, sit: null },
  coffee_addict: { neutral: null, laptop: 44, callout: null, alert: 45, sit: 47 },
  ai_researcher: { neutral: null, laptop: 48, callout: null, alert: null, sit: null },
  designer: { neutral: null, laptop: null, callout: 50, alert: 49, sit: null },
};

const ARCHETYPE_BY_PROFILE: Record<string, ArchetypeKey> = {
  Artificer: "planner",
  Seer: "planner",
  Scribe: "coffee_addict",
  Runner: "coffee_addict",
  Guardian: "ai_researcher",
  Ranger: "designer",
};

const PIXI = PIXI_RUNTIME as unknown as {
  Application: new (options: Record<string, unknown>) => PixiApplication;
  BaseTexture: { defaultOptions: { scaleMode?: unknown } };
  SCALE_MODES: { NEAREST: unknown };
  Graphics: new () => PixiGraphics;
  Container: new () => PixiContainer;
  Sprite: new (texture: PixiTexture) => PixiSprite;
  Texture: new (baseTexture: unknown, frame: unknown) => PixiTexture;
  Rectangle: new (x: number, y: number, width: number, height: number) => unknown;
  Assets: { load: (url: string) => Promise<PixiTexture> };
};

declare global {
  interface Window {
    AkraAdminGame?: AkraAdminGameBridge;
  }
}

(() => {
  let activeHandle: DioramaHandle | null = null;

  const isStatusSeverity = (value: string | undefined): value is StatusSeverity =>
    value === "normal" ||
    value === "success" ||
    value === "warning" ||
    value === "danger" ||
    value === "info" ||
    value === "muted";

  const isVisualState = (value: string | undefined): value is VisualState =>
    value === "idle" ||
    value === "starting" ||
    value === "working" ||
    value === "awaiting_review" ||
    value === "blocked" ||
    value === "delivering" ||
    value === "cleanup";

  const isStaticPose = (value: string | undefined): value is StaticPose =>
    value === "neutral" ||
    value === "laptop" ||
    value === "callout" ||
    value === "alert" ||
    value === "sit";

  const isPresenceKind = (value: string | undefined): value is PresenceKind =>
    value === "active" || value === "configured_standby";

  const isPixiTexture = (texture: PixiTexture | null): texture is PixiTexture =>
    texture !== null;

  const mountDiorama = (): DioramaHandle | null => {
    const container = document.getElementById("pixi-diorama");
    if (!container) return null;
    const boardEl = container.closest<HTMLElement>(".office-board");
    if (!boardEl || container.dataset.akraDioramaMounted === "true") return null;

    container.dataset.akraDioramaMounted = "true";
    const initialWidth = boardEl.offsetWidth || 900;
    const initialHeight = boardEl.offsetHeight || 540;
    const app = new PIXI.Application({
      width: initialWidth,
      height: initialHeight,
      backgroundAlpha: 0,
      antialias: false,
      resolution: Math.min(window.devicePixelRatio || 1, 2),
      autoDensity: true,
      autoStart: false,
      preserveDrawingBuffer: true,
      hello: false,
    });
    container.appendChild(app.view);
    PIXI.BaseTexture.defaultOptions.scaleMode = PIXI.SCALE_MODES.NEAREST;

    const basePath = "/admin/assets/graphics/";
    const assets: Record<AssetKey, string> = {
      fdDesk1: basePath + "sprite_fd_desk_1.png",
      fdDesk2: basePath + "sprite_fd_desk_2.png",
      fdDesk3: basePath + "sprite_fd_desk_3.png",
      fdDesk4: basePath + "sprite_fd_desk_4.png",
      fdDesk5: basePath + "sprite_fd_desk_5.png",
      fdBossDesk: basePath + "sprite_fd_boss_desk.png",
      fdDistributorDesk: basePath + "sprite_fd_distributor_desk.png",
      fdEventLogTower: basePath + "sprite_fd_event_log_tower.png",
      fdSofa: basePath + "sprite_fd_sofa.png",
      fdPlant: basePath + "sprite_fd_potted_plant.png",
      agentAtlas: basePath + "gamebaljeonguk_atlas_128x192.png",
    };

    const root = boardEl.closest<HTMLElement>("[data-admin-graphic]");
    const structureLayer = new PIXI.Container();
    const packetLayer = new PIXI.Container();
    const agentLayer = new PIXI.Container();
    structureLayer.sortableChildren = true;
    packetLayer.sortableChildren = true;
    agentLayer.sortableChildren = true;
    app.stage.addChild(structureLayer, packetLayer, agentLayer);

    const statusPalette: Record<StatusSeverity, number> = {
      normal: 0x35d07f,
      success: 0x35d07f,
      warning: 0xf5c84b,
      danger: 0xff6b6b,
      info: 0x5da9ff,
      muted: 0x98abc4,
    };

    let textures: Partial<Record<AssetKey, PixiTexture>> = {};
    let agentFrameSets: Partial<Record<ArchetypeKey, AgentFrameSet>> = {};
    let agentUnits: AgentUnit[] = [];
    let signalPackets: SignalPacket[] = [];
    let structureSprites: StructureSprite[] = [];
    let resizeObserver: ResizeObserver | null = null;
    let renderRequestId = 0;
    let animationRequestId = 0;
    let renderCount = 0;
    let lastAnimationTime = 0;
    let lastLayoutWidth = 0;
    let lastLayoutHeight = 0;
    let ready = false;

    const syncInspectionDataset = (): void => {
      const activeUnits = agentUnits.filter((unit) => unit.presenceKind === "active");
      const standbyUnits = agentUnits.filter(
        (unit) => unit.presenceKind === "configured_standby"
      );
      container.dataset.sceneReady = String(ready);
      container.dataset.sceneActorCount = String(activeUnits.length);
      container.dataset.sceneCharacterCount = String(agentUnits.length);
      container.dataset.sceneStandbyCount = String(standbyUnits.length);
      container.dataset.scenePacketCount = String(signalPackets.length);
      container.dataset.sceneSemanticMotionCount = String(
        reducedMotion ? 0 : agentUnits.filter((unit) => unit.visualState !== "idle").length
      );
      container.dataset.sceneRenderCount = String(renderCount);
      container.dataset.sceneActorSignature = JSON.stringify(
        activeUnits.map((unit) => ({
          actorId: unit.actorId,
          agentId: unit.agentId,
          slotId: unit.slotId,
          visualState: unit.visualState,
          pose: unit.pose,
        }))
      );
      container.dataset.sceneStandbySignature = JSON.stringify(
        standbyUnits.map((unit) => ({
          characterId: unit.characterId,
          agentId: unit.agentId,
          visualState: unit.visualState,
          pose: unit.pose,
          resolvedAtlasFrameIndex: unit.resolvedAtlasFrameIndex,
          poseFallback: unit.poseFallback,
        }))
      );
    };

    const boardSize = () => ({
      width: boardEl.offsetWidth || initialWidth,
      height: boardEl.offsetHeight || initialHeight,
    });

    const designToBoardPoint = (point: Point): Point => {
      const { width, height } = boardSize();
      return { x: (point.x / MAP_WIDTH) * width, y: (point.y / MAP_HEIGHT) * height };
    };

    const boardVisualScale = (): number => {
      const { width, height } = boardSize();
      return Math.min(width / MAP_WIDTH, height / MAP_HEIGHT) || 1;
    };

    const reducedMotion = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;

    const lerp = (from: number, to: number, progress: number): number =>
      from + (to - from) * progress;

    const pointBetween = (from: Point, to: Point, progress: number): Point => ({
      x: lerp(from.x, to.x, progress),
      y: lerp(from.y, to.y, progress),
    });

    const motionTargetFor = (unit: AgentUnit): Point => {
      if (unit.visualState === "awaiting_review") return REVIEW_STATION;
      if (unit.visualState === "delivering") return DELIVERY_STATION;
      if (unit.visualState === "cleanup") return CLEANUP_STATION;
      return unit.homePoint;
    };

    const semanticProgress = (unit: AgentUnit, elapsedSeconds: number): number => {
      const wave = (Math.sin(elapsedSeconds * 1.35 + unit.motionPhase) + 1) / 2;
      if (unit.visualState === "starting") return Math.min((elapsedSeconds * 0.22 + unit.motionPhase) % 1, 1);
      if (
        unit.visualState === "awaiting_review"
        || unit.visualState === "delivering"
        || unit.visualState === "cleanup"
      ) {
        return 0.12 + wave * 0.22;
      }
      return 0;
    };

    const clamp = (value: number, min: number, max: number): number =>
      Math.min(Math.max(value, min), max);

    const requestSceneRender = (): void => {
      if (renderRequestId) return;
      renderRequestId = window.requestAnimationFrame(() => {
        renderRequestId = 0;
        app.renderer.render(app.stage);
        renderCount += 1;
        syncInspectionDataset();
      });
    };

    const makeAtlasFrame = (
      texture: PixiTexture | undefined,
      col: number,
      row: number
    ): PixiTexture | null => {
      const baseTexture = texture?.baseTexture;
      if (!baseTexture || typeof PIXI.Rectangle === "undefined") return null;
      return new PIXI.Texture(
        baseTexture,
        new PIXI.Rectangle(
          col * AGENT_FRAME_WIDTH,
          row * AGENT_FRAME_HEIGHT,
          AGENT_FRAME_WIDTH,
          AGENT_FRAME_HEIGHT
        )
      );
    };

    const makeAtlasFrameByIndex = (
      texture: PixiTexture | undefined,
      atlasFrameIndex: number
    ): PixiTexture | null =>
      makeAtlasFrame(texture, atlasFrameIndex % 8, Math.floor(atlasFrameIndex / 8));

    const makeFrameRow = (
      texture: PixiTexture | undefined,
      row: number,
      startCol: number
    ): PixiTexture[] =>
      Array.from({ length: 4 }, (_, index) => makeAtlasFrame(texture, startCol + index, row)).filter(
        isPixiTexture
      );

    const buildAgentFrameSets = (
      texture: PixiTexture | undefined
    ): Partial<Record<ArchetypeKey, AgentFrameSet>> => ({
      planner: {
        down: makeFrameRow(texture, 0, 0),
        side: makeFrameRow(texture, 1, 0),
        up: makeFrameRow(texture, 2, 0),
      },
      coffee_addict: {
        down: makeFrameRow(texture, 0, 4),
        side: makeFrameRow(texture, 1, 4),
        up: makeFrameRow(texture, 2, 4),
      },
      ai_researcher: {
        down: makeFrameRow(texture, 3, 0),
        side: makeFrameRow(texture, 4, 0),
        up: makeFrameRow(texture, 4, 0),
      },
      designer: {
        down: makeFrameRow(texture, 3, 4),
        side: makeFrameRow(texture, 4, 4),
        up: makeFrameRow(texture, 4, 4),
      },
    });

    const agentSpriteScale = (): number =>
      AGENT_SPRITE_SCALE * clamp(boardVisualScale(), 0.58, 1.08);

    const syncStructureSprites = (): void => {
      const scale = boardVisualScale();
      for (const { spec, sprite } of structureSprites) {
        const point = designToBoardPoint(spec);
        sprite.x = point.x;
        sprite.y = point.y;
        sprite.scale.set(spec.scale * scale);
        sprite.zIndex = point.y + (spec.zOffset || 0);
      }
    };

    const buildStructureSprites = (): void => {
      for (const child of structureLayer.removeChildren()) child.destroy({ children: true });
      structureSprites = [];
      for (const spec of STRUCTURE_SPECS) {
        const texture = textures[spec.key];
        if (!texture) continue;
        const sprite = new PIXI.Sprite(texture);
        sprite.anchor.set(spec.anchorX ?? 0.5, spec.anchorY ?? 1);
        sprite.alpha = 0.96;
        structureLayer.addChild(sprite);
        structureSprites.push({ spec, sprite });
      }
      syncStructureSprites();
    };

    const parseSeverity = (node: HTMLElement): StatusSeverity => {
      if (isStatusSeverity(node.dataset.detailSeverity)) return node.dataset.detailSeverity;
      if (node.classList.contains("severity-danger")) return "danger";
      if (node.classList.contains("severity-warning")) return "warning";
      if (node.classList.contains("severity-info")) return "info";
      return "normal";
    };

    const archetypeFor = (node: HTMLElement): ArchetypeKey =>
      ARCHETYPE_BY_PROFILE[node.dataset.archetypeKey || ""] || "coffee_addict";

    const pointFor = (node: HTMLElement): Point => {
      if (node.dataset.presenceKind === "configured_standby") {
        const standbyIndex = Number(node.dataset.sceneStandbyIndex || "") - 1;
        return STANDBY_LOUNGE_POINTS[standbyIndex] || STANDBY_LOUNGE_POINTS[0];
      }
      const seatIndex = Number(node.dataset.sceneSeatIndex || "") - 1;
      return SLOT_SEATS[seatIndex] || SLOT_SEATS[0];
    };

    const drawStaticMarker = (
      marker: PixiGraphics,
      visualState: VisualState,
      color: number,
      presenceKind: PresenceKind
    ): void => {
      const scale = clamp(boardVisualScale(), 0.58, 1.08);
      const emphasis = presenceKind === "configured_standby" ? 1.2 : 1;
      const width = AGENT_MARKER_WIDTH * scale * emphasis;
      const height = AGENT_MARKER_HEIGHT * scale * emphasis;
      marker.clear();
      marker.lineStyle(2, color, presenceKind === "configured_standby" ? 1 : 0.9);
      if (visualState === "blocked") {
        marker.moveTo(-width * 0.7, -height);
        marker.lineTo(width * 0.7, height);
        marker.moveTo(width * 0.7, -height);
        marker.lineTo(-width * 0.7, height);
        return;
      }
      if (visualState === "delivering") {
        marker.beginFill(color, 0.22);
        marker.drawPolygon([-width, -height, width * 0.35, -height, width, 0, width * 0.35, height, -width, height]);
        marker.endFill();
        return;
      }
      if (presenceKind === "configured_standby") marker.beginFill(color, 0.14);
      marker.drawEllipse(0, 0, width, height);
      if (presenceKind === "configured_standby") marker.endFill();
      if (visualState === "awaiting_review") marker.drawEllipse(0, 0, width * 0.66, height * 0.66);
      if (visualState === "working") {
        marker.beginFill(color, 0.16);
        marker.drawEllipse(0, 0, width * 0.72, height * 0.72);
        marker.endFill();
      }
    };

    const resolveAgentTexture = (
      archetype: ArchetypeKey,
      visualState: VisualState,
      pose: StaticPose
    ): {
      texture: PixiTexture | null;
      resolvedAtlasFrameIndex: number | null;
      poseFallback: boolean;
    } => {
      const atlasFrameIndex = STATIC_POSE_MANIFEST[archetype][pose];
      if (atlasFrameIndex !== null) {
        const texture = makeAtlasFrameByIndex(textures.agentAtlas, atlasFrameIndex);
        if (texture) {
          return { texture, resolvedAtlasFrameIndex: atlasFrameIndex, poseFallback: false };
        }
      }
      const neutralFrame = NEUTRAL_FRAME_MANIFEST[visualState];
      const frames = agentFrameSets[archetype]?.[neutralFrame.facing] || [];
      return {
        texture: frames[neutralFrame.frameIndex] || frames[0] || null,
        resolvedAtlasFrameIndex: null,
        poseFallback: pose !== "neutral",
      };
    };

    const makeAgentUnit = (node: HTMLElement): AgentUnit | null => {
      const visualState = node.dataset.visualState;
      if (!isVisualState(visualState)) return null;
      const pose = isStaticPose(node.dataset.staticPose) ? node.dataset.staticPose : "neutral";
      const presenceKind = isPresenceKind(node.dataset.presenceKind)
        ? node.dataset.presenceKind
        : "active";
      const characterId = node.dataset.characterId || node.dataset.actorId || "";
      const actorId = node.dataset.actorId || "";
      const agentId = node.dataset.agentId || "";
      const slotId = node.dataset.slotId || "";
      if (!characterId || !agentId) return null;
      if (presenceKind === "active" && (!actorId || !slotId)) return null;
      const standbyRuntimeIdentityKeys = [
        "actorId",
        "taskId",
        "slotId",
        "sessionKey",
        "ownerAgentId",
        "ownerSessionKey",
        "leaseGeneration",
        "branchName",
        "queueItemId",
      ] as const;
      if (
        presenceKind === "configured_standby" &&
        standbyRuntimeIdentityKeys.some((key) => Boolean(node.dataset[key]))
      ) {
        return null;
      }

      const severity = parseSeverity(node);
      const color = statusPalette[severity] || statusPalette.normal;
      const archetype = archetypeFor(node);
      const { texture, resolvedAtlasFrameIndex, poseFallback } = resolveAgentTexture(
        archetype,
        visualState,
        pose
      );
      const group = new PIXI.Container();
      const shadow = new PIXI.Graphics();
      shadow.beginFill(0x000000, presenceKind === "configured_standby" ? 0.36 : 0.3);
      shadow.drawEllipse(0, 0, AGENT_SHADOW_WIDTH, AGENT_SHADOW_HEIGHT);
      shadow.endFill();
      const marker = new PIXI.Graphics();
      drawStaticMarker(marker, visualState, color, presenceKind);
      const sprite = texture ? new PIXI.Sprite(texture) : null;
      if (sprite) {
        sprite.anchor.set(0.5, 1);
        sprite.scale.set(agentSpriteScale());
        group.addChild(shadow, marker, sprite);
      } else {
        group.addChild(shadow, marker);
      }
      group.alpha = 1;
      agentLayer.addChild(group);

      const point = pointFor(node);
      const unit: AgentUnit = {
        characterId,
        presenceKind,
        actorId,
        agentId,
        slotId,
        visualState,
        pose,
        node,
        group,
        sprite,
        marker,
        homePoint: point,
        point,
        motionPhase: [...characterId].reduce((sum, character) => sum + character.charCodeAt(0), 0) % 17,
        archetype,
        resolvedAtlasFrameIndex,
        poseFallback,
      };
      if (presenceKind === "active") {
        node.addEventListener("pointerenter", () => {
          group.scale.set(1.08);
          requestSceneRender();
        });
        node.addEventListener("pointerleave", () => {
          group.scale.set(1);
          requestSceneRender();
        });
      }
      return unit;
    };

    const syncAgentUnits = (): void => {
      for (const unit of agentUnits) {
        unit.homePoint = pointFor(unit.node);
        unit.point = unit.homePoint;
        const point = designToBoardPoint(unit.point);
        unit.group.x = point.x;
        unit.group.y = point.y;
        unit.group.zIndex = point.y;
        if (unit.sprite) unit.sprite.scale.set(agentSpriteScale());
        drawStaticMarker(
          unit.marker,
          unit.visualState,
          statusPalette[parseSeverity(unit.node)] || statusPalette.normal,
          unit.presenceKind
        );
      }
    };

    const rebuildSignalPackets = (): void => {
      for (const child of packetLayer.removeChildren()) child.destroy({ children: true });
      signalPackets = agentUnits
        .filter(
          (unit) =>
            unit.presenceKind === "active"
            && ["awaiting_review", "delivering", "cleanup"].includes(unit.visualState)
        )
        .map((unit, index) => {
          const color = statusPalette[parseSeverity(unit.node)] || statusPalette.info;
          const graphic = new PIXI.Graphics();
          graphic.beginFill(color, 0.9);
          graphic.drawPolygon([0, -5, 7, 0, 0, 5, -7, 0]);
          graphic.endFill();
          graphic.zIndex = 9_000 + index;
          packetLayer.addChild(graphic);
          return {
            unit,
            graphic,
            target: motionTargetFor(unit),
            phase: index / Math.max(agentUnits.length, 1),
          };
        });
    };

    const animateScene = (timestamp: number): void => {
      animationRequestId = window.requestAnimationFrame(animateScene);
      if (!ready || reducedMotion || timestamp - lastAnimationTime < 66) return;
      lastAnimationTime = timestamp;
      const movingUnits = agentUnits.filter((unit) => unit.visualState !== "idle");
      if (movingUnits.length === 0 && signalPackets.length === 0) return;
      const elapsedSeconds = timestamp / 1000;
      for (const unit of movingUnits) {
        const target = motionTargetFor(unit);
        const progress = semanticProgress(unit, elapsedSeconds);
        const designPoint = pointBetween(unit.homePoint, target, progress);
        const workingBob = unit.visualState === "working"
          ? Math.sin(elapsedSeconds * 5 + unit.motionPhase) * 2.4
          : 0;
        const blockedJitter = unit.visualState === "blocked"
          ? Math.sin(elapsedSeconds * 10 + unit.motionPhase) * 1.8
          : 0;
        unit.point = {
          x: designPoint.x + blockedJitter,
          y: designPoint.y + workingBob,
        };
        const boardPoint = designToBoardPoint(unit.point);
        unit.group.x = boardPoint.x;
        unit.group.y = boardPoint.y;
        unit.group.zIndex = boardPoint.y;
        if (unit.sprite && unit.pose === "neutral" && unit.visualState === "working") {
          const frames = agentFrameSets[unit.archetype]?.down || [];
          const frameIndex = Math.floor(elapsedSeconds * 4 + unit.motionPhase) % Math.max(frames.length, 1);
          if (frames[frameIndex]) unit.sprite.texture = frames[frameIndex];
        }
      }
      for (const packet of signalPackets) {
        const travel = (elapsedSeconds * 0.34 + packet.phase) % 1;
        const designPoint = pointBetween(packet.unit.homePoint, packet.target, travel);
        const boardPoint = designToBoardPoint(designPoint);
        packet.graphic.x = boardPoint.x;
        packet.graphic.y = boardPoint.y;
        packet.graphic.alpha = 0.35 + Math.sin(travel * Math.PI) * 0.65;
        packet.graphic.scale.set(clamp(boardVisualScale(), 0.62, 1.1));
      }
      requestSceneRender();
    };

    const syncLayout = (force = false): void => {
      const { width, height } = boardSize();
      const layoutWidth = Math.round(width);
      const layoutHeight = Math.round(height);
      if (layoutWidth <= 0 || layoutHeight <= 0) return;
      const sizeChanged = layoutWidth !== lastLayoutWidth || layoutHeight !== lastLayoutHeight;
      if (!force && !sizeChanged) return;
      if (sizeChanged) app.renderer.resize(layoutWidth, layoutHeight);
      lastLayoutWidth = layoutWidth;
      lastLayoutHeight = layoutHeight;
      syncStructureSprites();
      syncAgentUnits();
      requestSceneRender();
    };

    const rebuildAgentUnits = (): void => {
      for (const child of agentLayer.removeChildren()) child.destroy({ children: true });
      agentUnits = root
        ? [
            ...root.querySelectorAll<HTMLElement>(
              ".desk[data-actor-id], [data-standby-character][data-character-id]"
            ),
          ]
            .map(makeAgentUnit)
            .filter((unit): unit is AgentUnit => unit !== null)
        : [];
      rebuildSignalPackets();
      syncInspectionDataset();
      syncLayout(true);
    };

    const inspectScene = (): SceneInspection => {
      const activeUnits = agentUnits.filter((unit) => unit.presenceKind === "active");
      const standbyUnits = agentUnits.filter(
        (unit) => unit.presenceKind === "configured_standby"
      );
      return {
        ready,
        actorCount: activeUnits.length,
        characterCount: agentUnits.length,
        standbyCount: standbyUnits.length,
        packetCount: signalPackets.length,
        semanticMotionCount: reducedMotion
          ? 0
          : agentUnits.filter((unit) => unit.visualState !== "idle").length,
        renderCount,
        actors: activeUnits.map((unit) => ({
          actorId: unit.actorId,
          agentId: unit.agentId,
          slotId: unit.slotId,
          visualState: unit.visualState,
          pose: unit.pose,
          resolvedAtlasFrameIndex: unit.resolvedAtlasFrameIndex,
          poseFallback: unit.poseFallback,
          displayWidth: Math.round(unit.sprite?.width || 0),
          displayHeight: Math.round(unit.sprite?.height || 0),
          opacity: unit.group.alpha,
          boardX: Math.round(unit.group.x),
          boardY: Math.round(unit.group.y),
          x: Math.round(unit.point.x),
          y: Math.round(unit.point.y),
        })),
        standbyCharacters: standbyUnits.map((unit) => ({
          characterId: unit.characterId,
          presenceKind: "configured_standby" as const,
          agentId: unit.agentId,
          visualState: unit.visualState,
          pose: unit.pose,
          locationIndex: Number(unit.node.dataset.sceneStandbyIndex || ""),
          resolvedAtlasFrameIndex: unit.resolvedAtlasFrameIndex,
          poseFallback: unit.poseFallback,
          displayWidth: Math.round(unit.sprite?.width || 0),
          displayHeight: Math.round(unit.sprite?.height || 0),
          opacity: unit.group.alpha,
          boardX: Math.round(unit.group.x),
          boardY: Math.round(unit.group.y),
          x: Math.round(unit.point.x),
          y: Math.round(unit.point.y),
        })),
      };
    };

    const loadTextures = async (): Promise<void> => {
      textures = {};
      const entries = Object.entries(assets) as [AssetKey, string][];
      const results = await Promise.allSettled(entries.map(([, url]) => PIXI.Assets.load(url)));
      results.forEach((result, index) => {
        const [key] = entries[index];
        if (result.status === "fulfilled") {
          textures[key] = result.value;
        } else {
          console.warn(`Failed to load sprite: ${key}`, result.reason);
        }
      });
      agentFrameSets = buildAgentFrameSets(textures.agentAtlas);
    };

    const onResize = (): void => syncLayout();

    const destroy = (): void => {
      window.removeEventListener("akra:scene-rendered", rebuildAgentUnits);
      window.removeEventListener("resize", onResize);
      resizeObserver?.disconnect();
      if (renderRequestId) window.cancelAnimationFrame(renderRequestId);
      if (animationRequestId) window.cancelAnimationFrame(animationRequestId);
      app.destroy(true, { children: true, texture: false, baseTexture: false });
      delete container.dataset.akraDioramaMounted;
      if (activeHandle?.app === app) activeHandle = null;
    };

    loadTextures().then(() => {
      buildStructureSprites();
      rebuildAgentUnits();
      ready = true;
      syncInspectionDataset();
      requestSceneRender();
      animationRequestId = window.requestAnimationFrame(animateScene);
      window.addEventListener("akra:scene-rendered", rebuildAgentUnits);
      window.addEventListener("resize", onResize);
      if (typeof ResizeObserver !== "undefined") {
        resizeObserver = new ResizeObserver(() => syncLayout());
        resizeObserver.observe(boardEl);
      }
    });

    const handle = { app, destroy, rebuildAgentUnits, syncLayout, inspectScene };
    activeHandle = handle;
    return handle;
  };

  window.AkraAdminGame = {
    ...(window.AkraAdminGame || {}),
    mountDiorama,
    inspectScene: (): SceneInspection | null => activeHandle?.inspectScene() || null,
  };

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", mountDiorama, { once: true });
  } else {
    mountDiorama();
  }
})();

export {};
