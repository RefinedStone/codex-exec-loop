type StatusSeverity = "normal" | "success" | "warning" | "danger" | "info" | "muted";
type TargetKind = "distributor" | "events";
type AssetKey = "floor" | "desk" | "server" | "whiteboard" | "sofa" | "plant" | "agentAtlas";

interface Point {
  x: number;
  y: number;
}

interface BoardSize {
  width: number;
  height: number;
}

interface PixiScale {
  set: (...values: number[]) => void;
}

interface PixiDisplayObject {
  x: number;
  y: number;
  alpha: number;
  rotation: number;
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
  addChild: (...children: PixiDisplayObject[]) => void;
  removeChildren: () => PixiDisplayObject[];
}

interface PixiSprite extends PixiDisplayObject {
  anchor: PixiScale;
}

interface PixiTexture {
  baseTexture?: unknown;
}

interface PixiApplication {
  view: HTMLCanvasElement;
  stage: PixiContainer;
  renderer: {
    resize: (width: number, height: number) => void;
  };
  ticker: {
    add: (handler: (delta: number) => void) => void;
  };
  destroy: (
    removeView?: boolean,
    options?: { children?: boolean; texture?: boolean; baseTexture?: boolean }
  ) => void;
}

interface AgentUnit {
  node: HTMLElement;
  index: number;
  color: number;
  group: PixiContainer;
  ring: PixiGraphics;
  sprite: PixiSprite | null;
  packet: PixiGraphics;
  point: Point;
  phase: number;
  speed: number;
  targetKind: TargetKind;
}

interface DioramaHandle {
  app: PixiApplication;
  destroy: () => void;
  rebuildAgentUnits: () => void;
  syncLayout: () => void;
}

interface AkraAdminGameBridge {
  mountDiorama?: () => DioramaHandle | null;
  [key: string]: unknown;
}

declare const PIXI: {
  Application: new (options: Record<string, unknown>) => PixiApplication;
  BaseTexture: { defaultOptions: { scaleMode?: unknown } };
  SCALE_MODES: { NEAREST: unknown };
  Graphics: new () => PixiGraphics;
  Container: new () => PixiContainer;
  Sprite: new (texture: PixiTexture) => PixiSprite;
  Texture: new (baseTexture: unknown, frame: unknown) => PixiTexture;
  Rectangle: new (x: number, y: number, width: number, height: number) => unknown;
  Assets: {
    load: (url: string) => Promise<PixiTexture>;
  };
};

declare global {
  interface Window {
    AkraAdminGame?: AkraAdminGameBridge;
  }
}

(() => {
  const isStatusSeverity = (value: string | undefined): value is StatusSeverity =>
    value === "normal" ||
    value === "success" ||
    value === "warning" ||
    value === "danger" ||
    value === "info" ||
    value === "muted";

  const isPixiTexture = (texture: PixiTexture | null): texture is PixiTexture =>
    texture !== null;

  const mountDiorama = (): DioramaHandle | null => {
    const container = document.getElementById("pixi-diorama");
    if (!container || typeof PIXI === "undefined") return null;

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
      resolution: window.devicePixelRatio || 1,
      autoDensity: true,
    });
    container.appendChild(app.view);

    PIXI.BaseTexture.defaultOptions.scaleMode = PIXI.SCALE_MODES.NEAREST;

    const basePath = "/admin/assets/graphics/";
    const assets: Record<AssetKey, string> = {
      floor: basePath + "sprite_floor_tile.png",
      desk: basePath + "sprite_desk_workstation.png",
      server: basePath + "sprite_server_rack.png",
      whiteboard: basePath + "sprite_whiteboard.png",
      sofa: basePath + "sprite_sofa.png",
      plant: basePath + "sprite_potted_plant.png",
      agentAtlas: basePath + "gamebaljeonguk_atlas_64x96.png",
    };

    const root = boardEl.closest<HTMLElement>("[data-admin-graphic]");
    const pathLayer = new PIXI.Graphics();
    const packetLayer = new PIXI.Container();
    const agentLayer = new PIXI.Container();
    app.stage.addChild(pathLayer, packetLayer, agentLayer);

    const statusPalette: Record<StatusSeverity, number> = {
      normal: 0x35d07f,
      success: 0x35d07f,
      warning: 0xf5c84b,
      danger: 0xff6b6b,
      info: 0x5da9ff,
      muted: 0x98abc4,
    };

    let textures: Partial<Record<AssetKey, PixiTexture>> = {};
    let agentFrames: PixiTexture[] = [];
    let agentUnits: AgentUnit[] = [];
    let stageBurst = 0;
    let elapsed = 0;
    let resizeObserver: ResizeObserver | null = null;

    const boardSize = (): BoardSize => ({
      width: boardEl.offsetWidth || initialWidth,
      height: boardEl.offsetHeight || initialHeight,
    });

    const fallbackPoints = (): Point[] => {
      const { width, height } = boardSize();
      return [
        { x: width * 0.35, y: height * 0.50 },
        { x: width * 0.50, y: height * 0.65 },
        { x: width * 0.28, y: height * 0.72 },
        { x: width * 0.60, y: height * 0.52 },
        { x: width * 0.43, y: height * 0.82 },
      ];
    };

    let targets: Record<TargetKind, Point> = {
      distributor: { x: initialWidth * 0.80, y: initialHeight * 0.52 },
      events: { x: initialWidth * 0.82, y: initialHeight * 0.76 },
    };

    const parseSeverity = (node: HTMLElement): StatusSeverity => {
      if (isStatusSeverity(node.dataset.detailSeverity)) return node.dataset.detailSeverity;
      if (node.classList.contains("severity-danger")) return "danger";
      if (node.classList.contains("severity-warning")) return "warning";
      if (node.classList.contains("severity-info")) return "info";
      return "normal";
    };

    const colorFor = (severity: StatusSeverity): number =>
      statusPalette[severity] || statusPalette.normal;

    const makeAtlasFrame = (
      texture: PixiTexture | undefined,
      col: number,
      row = 0
    ): PixiTexture | null => {
      const baseTexture = texture?.baseTexture;
      if (!baseTexture || typeof PIXI.Rectangle === "undefined") return null;
      return new PIXI.Texture(baseTexture, new PIXI.Rectangle(col * 64, row * 96, 64, 96));
    };

    const resolvePoint = (
      node: Element | null | undefined,
      fallback: Point,
      xBias = 0.5,
      yBias = 0.76
    ): Point => {
      if (!node) return fallback;
      const boardRect = boardEl.getBoundingClientRect();
      const rect = node.getBoundingClientRect();
      if (rect.width <= 0 && rect.height <= 0) return fallback;
      return {
        x: rect.left - boardRect.left + rect.width * xBias,
        y: rect.top - boardRect.top + rect.height * yBias,
      };
    };

    const makePacket = (color: number): PixiGraphics => {
      const packet = new PIXI.Graphics();
      packet.beginFill(color, 0.92);
      packet.lineStyle(1, 0xffffff, 0.42);
      packet.drawPolygon([0, -6, 6, 0, 0, 6, -6, 0]);
      packet.endFill();
      packet.alpha = 0.84;
      packetLayer.addChild(packet);
      return packet;
    };

    const makeAgentUnit = (node: HTMLElement, index: number): AgentUnit => {
      const severity = parseSeverity(node);
      const color = colorFor(severity);
      const group = new PIXI.Container();
      const shadow = new PIXI.Graphics();
      shadow.beginFill(0x000000, 0.26);
      shadow.drawEllipse(0, 0, 30, 8);
      shadow.endFill();

      const ring = new PIXI.Graphics();
      const texture = agentFrames.length ? agentFrames[index % agentFrames.length] : null;
      const sprite = texture ? new PIXI.Sprite(texture) : null;
      if (sprite) {
        sprite.anchor.set(0.5, 1);
        sprite.scale.set(0.72);
        group.addChild(shadow, ring, sprite);
      } else {
        group.addChild(shadow, ring);
      }

      group.alpha = severity === "muted" ? 0.58 : 0.95;
      agentLayer.addChild(group);

      const packet = makePacket(color);
      node.addEventListener("pointerenter", () => {
        group.scale.set(1.08);
        packet.scale.set(1.2);
      });
      node.addEventListener("pointerleave", () => {
        group.scale.set(1);
        packet.scale.set(1);
      });
      node.addEventListener("click", () => {
        stageBurst = Math.min(stageBurst + 0.55, 1.4);
      });

      const points = fallbackPoints();
      return {
        node,
        index,
        color,
        group,
        ring,
        sprite,
        packet,
        point: points[index % points.length],
        phase: index * 0.23,
        speed: 0.16 + index * 0.025,
        targetKind: index % 2 === 0 ? "distributor" : "events",
      };
    };

    const syncLayout = (): void => {
      const { width, height } = boardSize();
      if (width > 0 && height > 0) app.renderer.resize(width, height);
      targets = {
        distributor: resolvePoint(
          root?.querySelector(".distributor-desk"),
          { x: width * 0.80, y: height * 0.52 },
          0.5,
          0.6
        ),
        events: resolvePoint(
          root?.querySelector(".event-board"),
          { x: width * 0.82, y: height * 0.76 },
          0.5,
          0.58
        ),
      };
      const points = fallbackPoints();
      for (const unit of agentUnits) {
        unit.point = resolvePoint(
          unit.node,
          points[unit.index % points.length],
          0.5,
          0.78
        );
        unit.group.x = unit.point.x;
        unit.group.y = unit.point.y;
      }
    };

    const rebuildAgentUnits = (): void => {
      for (const child of packetLayer.removeChildren()) child.destroy({ children: true });
      for (const child of agentLayer.removeChildren()) child.destroy({ children: true });
      agentUnits = root
        ? [...root.querySelectorAll<HTMLElement>(".desk[data-agent-id]")].map(makeAgentUnit)
        : [];
      syncLayout();
    };

    const lerp = (a: number, b: number, t: number): number => a + (b - a) * t;

    const renderTick = (delta: number): void => {
      elapsed += delta / 60;
      stageBurst = Math.max(0, stageBurst - delta * 0.018);
      pathLayer.clear();

      for (const unit of agentUnits) {
        const target = targets[unit.targetKind] || targets.distributor;
        const start = { x: unit.point.x, y: unit.point.y - 24 };
        const end = { x: target.x, y: target.y - 18 };
        const alpha = 0.22 + stageBurst * 0.16;
        pathLayer.lineStyle(2, unit.color, alpha);
        pathLayer.moveTo(start.x, start.y);
        pathLayer.lineTo(end.x, end.y);

        const travel = (elapsed * unit.speed + unit.phase) % 1;
        const arc = Math.sin(travel * Math.PI) * (24 + stageBurst * 10);
        unit.packet.x = lerp(start.x, end.x, travel);
        unit.packet.y = lerp(start.y, end.y, travel) - arc;
        unit.packet.rotation += delta * 0.08;
        unit.packet.alpha = 0.42 + Math.sin(travel * Math.PI) * 0.42 + stageBurst * 0.12;
        unit.packet.scale.set(0.92 + stageBurst * 0.18);

        const bob = Math.sin(elapsed * 3.4 + unit.phase * 8) * 2.2;
        unit.group.y = unit.point.y + bob - stageBurst * 1.5;
        unit.ring.clear();
        unit.ring.lineStyle(2, unit.color, 0.58 + stageBurst * 0.2);
        const ringPulse = 1 + Math.sin(elapsed * 4.2 + unit.phase * 4) * 0.12 + stageBurst * 0.12;
        unit.ring.drawEllipse(0, 0, 25 * ringPulse, 7 * ringPulse);
      }
    };

    const loadTextures = async (): Promise<void> => {
      textures = {};
      for (const [key, url] of Object.entries(assets) as [AssetKey, string][]) {
        try {
          textures[key] = await PIXI.Assets.load(url);
        } catch (error) {
          console.warn(`Failed to load sprite: ${key}`, error);
        }
      }
      agentFrames = Array.from({ length: 8 }, (_, index) =>
        makeAtlasFrame(textures.agentAtlas, index)
      ).filter(isPixiTexture);
    };

    const onMissionPulse = (event: Event): void => {
      const detail = (event as CustomEvent<{ count?: number }>).detail;
      const count = Number(detail?.count || 1);
      stageBurst = Math.min(stageBurst + 0.45 + count * 0.06, 1.6);
    };
    const onResize = (): void => syncLayout();

    const destroy = (): void => {
      window.removeEventListener("akra:mission-pulse", onMissionPulse);
      window.removeEventListener("akra:dashboard-rendered", rebuildAgentUnits);
      window.removeEventListener("resize", onResize);
      resizeObserver?.disconnect();
      app.destroy(true, { children: true, texture: false, baseTexture: false });
      delete container.dataset.akraDioramaMounted;
    };

    loadTextures().then(() => {
      rebuildAgentUnits();
      app.ticker.add(renderTick);
      window.addEventListener("akra:mission-pulse", onMissionPulse);
      window.addEventListener("akra:dashboard-rendered", rebuildAgentUnits);
      window.addEventListener("resize", onResize);
      if (typeof ResizeObserver !== "undefined") {
        resizeObserver = new ResizeObserver(syncLayout);
        resizeObserver.observe(boardEl);
      }
    });

    return { app, destroy, rebuildAgentUnits, syncLayout };
  };

  window.AkraAdminGame = {
    ...(window.AkraAdminGame || {}),
    mountDiorama,
  };

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", mountDiorama, { once: true });
  } else {
    mountDiorama();
  }
})();

export {};
