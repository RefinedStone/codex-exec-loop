import "@pixi/unsafe-eval";
import * as PIXI_RUNTIME from "pixi.js";

type StatusSeverity = "normal" | "success" | "warning" | "danger" | "info" | "muted";
type VisualState =
  | "starting"
  | "working"
  | "awaiting_review"
  | "blocked"
  | "delivering"
  | "cleanup";
type StaticPose = "neutral" | "laptop" | "callout" | "alert" | "sit";
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
  actorId: string;
  agentId: string;
  slotId: string;
  visualState: VisualState;
  pose: StaticPose;
  node: HTMLElement;
  group: PixiContainer;
  sprite: PixiSprite | null;
  marker: PixiGraphics;
  point: Point;
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
  packetCount: 0;
  semanticMotionCount: 0;
  renderCount: number;
  actors: Array<{
    actorId: string;
    agentId: string;
    slotId: string;
    visualState: VisualState;
    pose: StaticPose;
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
const AGENT_SPRITE_SCALE = 0.4675;
const AGENT_SHADOW_WIDTH = 26.35;
const AGENT_SHADOW_HEIGHT = 6.8;
const AGENT_MARKER_WIDTH = 21.25;
const AGENT_MARKER_HEIGHT = 5.95;
const MAP_WIDTH = 1671;
const MAP_HEIGHT = 941;

const SLOT_SEATS: Point[] = [
  { x: 450, y: 420 },
  { x: 640, y: 345 },
  { x: 500, y: 615 },
  { x: 760, y: 565 },
  { x: 1030, y: 570 },
];

const STRUCTURE_SPECS: StructureSpec[] = [
  { key: "fdDesk1", x: 470, y: 405, scale: 0.62 },
  { key: "fdDesk2", x: 660, y: 330, scale: 0.62 },
  { key: "fdDesk3", x: 505, y: 600, scale: 0.62 },
  { key: "fdDesk4", x: 785, y: 548, scale: 0.6 },
  { key: "fdDesk5", x: 1048, y: 555, scale: 0.58 },
  { key: "fdBossDesk", x: 840, y: 260, scale: 0.58 },
  { key: "fdDistributorDesk", x: 1075, y: 315, scale: 0.62 },
  { key: "fdEventLogTower", x: 1332, y: 520, scale: 0.82 },
  { key: "fdSofa", x: 805, y: 760, scale: 0.62 },
  { key: "fdPlant", x: 615, y: 505, scale: 0.7 },
  { key: "fdPlant", x: 925, y: 485, scale: 0.7 },
  { key: "fdPlant", x: 1185, y: 620, scale: 0.7 },
];

const STATIC_POSE_MANIFEST: Record<VisualState, { facing: Facing; frameIndex: number }> = {
  starting: { facing: "side", frameIndex: 0 },
  working: { facing: "down", frameIndex: 0 },
  awaiting_review: { facing: "up", frameIndex: 0 },
  blocked: { facing: "side", frameIndex: 0 },
  delivering: { facing: "up", frameIndex: 0 },
  cleanup: { facing: "down", frameIndex: 0 },
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
    const agentLayer = new PIXI.Container();
    structureLayer.sortableChildren = true;
    agentLayer.sortableChildren = true;
    app.stage.addChild(structureLayer, agentLayer);

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
    let structureSprites: StructureSprite[] = [];
    let resizeObserver: ResizeObserver | null = null;
    let renderRequestId = 0;
    let renderCount = 0;
    let lastLayoutWidth = 0;
    let lastLayoutHeight = 0;
    let ready = false;

    const syncInspectionDataset = (): void => {
      container.dataset.sceneReady = String(ready);
      container.dataset.sceneActorCount = String(agentUnits.length);
      container.dataset.scenePacketCount = "0";
      container.dataset.sceneSemanticMotionCount = "0";
      container.dataset.sceneRenderCount = String(renderCount);
      container.dataset.sceneActorSignature = JSON.stringify(
        agentUnits.map((unit) => ({
          actorId: unit.actorId,
          agentId: unit.agentId,
          slotId: unit.slotId,
          visualState: unit.visualState,
          pose: unit.pose,
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
      const seatIndex = Number(node.dataset.sceneSeatIndex || "") - 1;
      return SLOT_SEATS[seatIndex] || SLOT_SEATS[0];
    };

    const drawStaticMarker = (
      marker: PixiGraphics,
      visualState: VisualState,
      color: number
    ): void => {
      const scale = boardVisualScale();
      const width = AGENT_MARKER_WIDTH * scale;
      const height = AGENT_MARKER_HEIGHT * scale;
      marker.clear();
      marker.lineStyle(2, color, 0.78);
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
      marker.drawEllipse(0, 0, width, height);
      if (visualState === "awaiting_review") marker.drawEllipse(0, 0, width * 0.66, height * 0.66);
      if (visualState === "working") {
        marker.beginFill(color, 0.16);
        marker.drawEllipse(0, 0, width * 0.72, height * 0.72);
        marker.endFill();
      }
    };

    const makeAgentUnit = (node: HTMLElement): AgentUnit | null => {
      const visualState = node.dataset.visualState;
      if (!isVisualState(visualState)) return null;
      const pose = isStaticPose(node.dataset.staticPose) ? node.dataset.staticPose : "neutral";
      const actorId = node.dataset.actorId || "";
      const agentId = node.dataset.agentId || "";
      const slotId = node.dataset.slotId || "";
      if (!actorId || !agentId || !slotId) return null;

      const severity = parseSeverity(node);
      const color = statusPalette[severity] || statusPalette.normal;
      const frameSet = agentFrameSets[archetypeFor(node)];
      const poseFrame = STATIC_POSE_MANIFEST[visualState];
      const frames = frameSet?.[poseFrame.facing] || [];
      const texture = frames[poseFrame.frameIndex] || frames[0] || null;
      const group = new PIXI.Container();
      const shadow = new PIXI.Graphics();
      shadow.beginFill(0x000000, 0.26);
      shadow.drawEllipse(0, 0, AGENT_SHADOW_WIDTH, AGENT_SHADOW_HEIGHT);
      shadow.endFill();
      const marker = new PIXI.Graphics();
      drawStaticMarker(marker, visualState, color);
      const sprite = texture ? new PIXI.Sprite(texture) : null;
      if (sprite) {
        sprite.anchor.set(0.5, 1);
        sprite.scale.set(agentSpriteScale());
        group.addChild(shadow, marker, sprite);
      } else {
        group.addChild(shadow, marker);
      }
      group.alpha = severity === "muted" ? 0.58 : 0.95;
      agentLayer.addChild(group);

      const point = pointFor(node);
      const unit: AgentUnit = {
        actorId,
        agentId,
        slotId,
        visualState,
        pose,
        node,
        group,
        sprite,
        marker,
        point,
      };
      node.addEventListener("pointerenter", () => {
        group.scale.set(1.08);
        requestSceneRender();
      });
      node.addEventListener("pointerleave", () => {
        group.scale.set(1);
        requestSceneRender();
      });
      return unit;
    };

    const syncAgentUnits = (): void => {
      for (const unit of agentUnits) {
        unit.point = pointFor(unit.node);
        const point = designToBoardPoint(unit.point);
        unit.group.x = point.x;
        unit.group.y = point.y;
        unit.group.zIndex = point.y;
        if (unit.sprite) unit.sprite.scale.set(agentSpriteScale());
        drawStaticMarker(
          unit.marker,
          unit.visualState,
          statusPalette[parseSeverity(unit.node)] || statusPalette.normal
        );
      }
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
        ? [...root.querySelectorAll<HTMLElement>(".desk[data-actor-id]")]
            .map(makeAgentUnit)
            .filter((unit): unit is AgentUnit => unit !== null)
        : [];
      syncInspectionDataset();
      syncLayout(true);
    };

    const inspectScene = (): SceneInspection => ({
      ready,
      actorCount: agentUnits.length,
      packetCount: 0,
      semanticMotionCount: 0,
      renderCount,
      actors: agentUnits.map((unit) => ({
        actorId: unit.actorId,
        agentId: unit.agentId,
        slotId: unit.slotId,
        visualState: unit.visualState,
        pose: unit.pose,
        x: Math.round(unit.point.x),
        y: Math.round(unit.point.y),
      })),
    });

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
