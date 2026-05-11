type StatusSeverity = "normal" | "success" | "warning" | "danger" | "info" | "muted";
type TargetKind = "distributor" | "events";
type AssetKey = "floor" | "desk" | "server" | "whiteboard" | "sofa" | "plant" | "agentAtlas";
type Facing = "down" | "side" | "up";

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

interface PixiText extends PixiDisplayObject {
  anchor: PixiScale;
  text: string;
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
  agentId: string;
  node: HTMLElement;
  index: number;
  color: number;
  group: PixiContainer;
  ring: PixiGraphics;
  sprite: PixiSprite | null;
  speechBubble: AgentSpeechBubble | null;
  packet: PixiGraphics;
  frameSet: AgentFrameSet | null;
  point: Point;
  destination: Point;
  phase: number;
  speed: number;
  walkSpeed: number;
  routeIndex: number;
  waitUntil: number;
  facing: Facing;
  facingSign: 1 | -1;
  isWalking: boolean;
  targetKind: TargetKind;
}

interface AgentFrameSet {
  down: PixiTexture[];
  side: PixiTexture[];
  up: PixiTexture[];
}

interface AgentSpeechBubble {
  group: PixiContainer;
  background: PixiGraphics;
  label: PixiText;
}

interface RoamSnapshot {
  point: Point;
  destination: Point;
  routeIndex: number;
  waitUntil: number;
}

interface DioramaHandle {
  app: PixiApplication;
  destroy: () => void;
  rebuildAgentUnits: () => void;
  setSpeechBubblesEnabled: (enabled: boolean) => void;
  syncLayout: () => void;
}

interface AkraAdminGameBridge {
  mountDiorama?: () => DioramaHandle | null;
  setSpeechBubblesEnabled?: (enabled: boolean) => void;
  [key: string]: unknown;
}

const AGENT_FRAME_WIDTH = 128;
const AGENT_FRAME_HEIGHT = 192;
const AGENT_SPRITE_SCALE = 0.4675;
const AGENT_SHADOW_WIDTH = 26.35;
const AGENT_SHADOW_HEIGHT = 6.8;
const AGENT_RING_WIDTH = 21.25;
const AGENT_RING_HEIGHT = 5.95;
const AGENT_SPEECH_BUBBLES_DEFAULT_ENABLED = true;
const AGENT_SPEECH_BUBBLE_MAX_WIDTH = 116;
const AGENT_SPEECH_BUBBLE_MIN_WIDTH = 54;
const AGENT_SPEECH_BUBBLE_TAIL_HEIGHT = 7;
const AGENT_SPEECH_BUBBLE_HEAD_GAP = 8;

declare const PIXI: {
  Application: new (options: Record<string, unknown>) => PixiApplication;
  BaseTexture: { defaultOptions: { scaleMode?: unknown } };
  SCALE_MODES: { NEAREST: unknown };
  Graphics: new () => PixiGraphics;
  Container: new () => PixiContainer;
  Sprite: new (texture: PixiTexture) => PixiSprite;
  Text?: new (text: string, style: Record<string, unknown>) => PixiText;
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
  let activeHandle: DioramaHandle | null = null;

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
      agentAtlas: basePath + "gamebaljeonguk_atlas_128x192.png",
    };

    const root = boardEl.closest<HTMLElement>("[data-admin-graphic]");
    const pathLayer = new PIXI.Graphics();
    const packetLayer = new PIXI.Container();
    const agentLayer = new PIXI.Container();
    agentLayer.sortableChildren = true;
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
    let agentFrameSets: AgentFrameSet[] = [];
    let agentUnits: AgentUnit[] = [];
    let roamSnapshots = new Map<string, RoamSnapshot>();
    let stageBurst = 0;
    let elapsed = 0;
    let speechBubblesEnabled = AGENT_SPEECH_BUBBLES_DEFAULT_ENABLED;
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

    const clamp = (value: number, min: number, max: number): number =>
      Math.min(Math.max(value, min), max);

    const lerp = (a: number, b: number, t: number): number => a + (b - a) * t;

    const distanceBetween = (a: Point, b: Point): number =>
      Math.hypot(a.x - b.x, a.y - b.y);

    const seededRatio = (index: number, routeIndex: number, salt: number): number => {
      const raw =
        Math.sin((index + 1) * 12.9898 + (routeIndex + 1) * 78.233 + salt * 37.719) *
        43758.5453;
      return raw - Math.floor(raw);
    };

    const roamBounds = () => {
      const { width, height } = boardSize();
      const horizontalInset = Math.min(width * 0.5, Math.max(56, width * 0.08));
      const topInset = Math.min(height * 0.5, Math.max(168, height * 0.24));
      const bottomInset = Math.max(58, height * 0.08);
      return {
        left: horizontalInset,
        right: Math.max(horizontalInset, width - horizontalInset),
        top: topInset,
        bottom: Math.max(topInset, height - bottomInset),
      };
    };

    const clampRoamPoint = (point: Point): Point => {
      const bounds = roamBounds();
      return {
        x: clamp(point.x, bounds.left, bounds.right),
        y: clamp(point.y, bounds.top, bounds.bottom),
      };
    };

    const chooseRoamPoint = (index: number, routeIndex: number): Point => {
      const bounds = roamBounds();
      return {
        x: lerp(bounds.left, bounds.right, seededRatio(index, routeIndex, 1)),
        y: lerp(bounds.top, bounds.bottom, seededRatio(index, routeIndex, 2)),
      };
    };

    const makeAtlasFrame = (
      texture: PixiTexture | undefined,
      col: number,
      row = 0
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

    const buildAgentFrameSets = (texture: PixiTexture | undefined): AgentFrameSet[] => {
      const planner = {
        down: makeFrameRow(texture, 0, 0),
        side: makeFrameRow(texture, 1, 0),
        up: makeFrameRow(texture, 2, 0),
      };
      const coffeeAddict = {
        down: makeFrameRow(texture, 0, 4),
        side: makeFrameRow(texture, 1, 4),
        up: makeFrameRow(texture, 2, 4),
      };
      const aiResearcher = {
        down: makeFrameRow(texture, 3, 0),
        side: makeFrameRow(texture, 4, 0),
        up: makeFrameRow(texture, 4, 0),
      };
      const designer = {
        down: makeFrameRow(texture, 3, 4),
        side: makeFrameRow(texture, 4, 4),
        up: makeFrameRow(texture, 4, 4),
      };
      return [planner, coffeeAddict, aiResearcher, designer].filter(
        (set) => set.down.length > 0 && set.side.length > 0 && set.up.length > 0
      );
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

    const speechNodeFor = (node: HTMLElement): HTMLElement | null =>
      node.querySelector<HTMLElement>(".speech");

    const speechLabelFor = (node: HTMLElement): string =>
      speechNodeFor(node)?.textContent?.trim() || node.dataset.detailState?.trim() || "작업중";

    const speechTextStyleFor = (node: HTMLElement): Record<string, unknown> => {
      const speechNode = speechNodeFor(node);
      const speechStyle = speechNode ? window.getComputedStyle(speechNode) : null;
      const fontSize = speechStyle ? parseFloat(speechStyle.fontSize) || 12 : 12;
      const lineHeight =
        speechStyle?.lineHeight && speechStyle.lineHeight !== "normal"
          ? parseFloat(speechStyle.lineHeight) || Math.round(fontSize * 1.25)
          : Math.round(fontSize * 1.25);
      return {
        align: "center",
        fill: speechStyle?.color || "#102015",
        fontFamily: speechStyle?.fontFamily || "'DungGeunMo', monospace",
        fontSize,
        fontWeight: speechStyle?.fontWeight || "800",
        lineHeight,
        wordWrap: true,
        wordWrapWidth: AGENT_SPEECH_BUBBLE_MAX_WIDTH - 18,
      };
    };

    const drawSpeechBubbleBackground = (
      background: PixiGraphics,
      width: number,
      height: number,
    ): void => {
      const halfWidth = width / 2;
      const bottom = -AGENT_SPEECH_BUBBLE_TAIL_HEIGHT;
      const top = bottom - height;
      const bubbleShape = [
        -halfWidth,
        top,
        halfWidth,
        top,
        halfWidth,
        bottom,
        7,
        bottom,
        0,
        0,
        -7,
        bottom,
        -halfWidth,
        bottom,
      ];
      const shadowShape = bubbleShape.map((value, index) => value + (index % 2 === 0 ? 2 : 3));

      background.clear();
      background.beginFill(0x000000, 0.28);
      background.drawPolygon(shadowShape);
      background.endFill();
      background.lineStyle(2, 0x18452a, 0.4);
      background.beginFill(0xf2fff4, 0.98);
      background.drawPolygon(bubbleShape);
      background.endFill();
      background.lineStyle(1, 0xffffff, 0.38);
      background.moveTo(-halfWidth + 4, top + 4);
      background.lineTo(halfWidth - 4, top + 4);
    };

    const makeSpeechBubble = (node: HTMLElement): AgentSpeechBubble | null => {
      const TextCtor = PIXI.Text;
      if (!TextCtor) return null;

      const group = new PIXI.Container();
      const background = new PIXI.Graphics();
      const label = new TextCtor(speechLabelFor(node), speechTextStyleFor(node));
      label.anchor.set(0.5, 0.5);

      const width = Math.ceil(
        clamp(label.width + 18, AGENT_SPEECH_BUBBLE_MIN_WIDTH, AGENT_SPEECH_BUBBLE_MAX_WIDTH)
      );
      const height = Math.ceil(Math.max(24, label.height + 10));
      drawSpeechBubbleBackground(background, width, height);
      label.y = -AGENT_SPEECH_BUBBLE_TAIL_HEIGHT - height / 2;
      group.addChild(background, label);
      group.alpha = speechBubblesEnabled ? 0.96 : 0;
      return { group, background, label };
    };

    const setSpeechBubblesEnabled = (enabled: boolean): void => {
      speechBubblesEnabled = enabled;
      for (const unit of agentUnits) {
        if (unit.speechBubble) unit.speechBubble.group.alpha = enabled ? 0.96 : 0;
      }
    };

    const makeAgentUnit = (node: HTMLElement, index: number): AgentUnit => {
      const agentId = node.dataset.agentId || `agent-${index}`;
      const severity = parseSeverity(node);
      const color = colorFor(severity);
      const group = new PIXI.Container();
      const shadow = new PIXI.Graphics();
      shadow.beginFill(0x000000, 0.26);
      shadow.drawEllipse(0, 0, AGENT_SHADOW_WIDTH, AGENT_SHADOW_HEIGHT);
      shadow.endFill();

      const ring = new PIXI.Graphics();
      const frameSet = agentFrameSets.length ? agentFrameSets[index % agentFrameSets.length] : null;
      const texture = frameSet?.down[0] || null;
      const sprite = texture ? new PIXI.Sprite(texture) : null;
      const speechBubble = makeSpeechBubble(node);
      if (sprite) {
        sprite.anchor.set(0.5, 1);
        sprite.scale.set(AGENT_SPRITE_SCALE);
        group.addChild(shadow, ring, sprite);
      } else {
        group.addChild(shadow, ring);
      }
      if (speechBubble) group.addChild(speechBubble.group);

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
      const fallbackPoint = resolvePoint(node, points[index % points.length], 0.5, 0.78);
      const snapshot = roamSnapshots.get(agentId);
      const routeIndex = snapshot?.routeIndex ?? index * 5;
      const point = clampRoamPoint(snapshot?.point || fallbackPoint);
      let destination = clampRoamPoint(snapshot?.destination || chooseRoamPoint(index, routeIndex));
      if (distanceBetween(point, destination) < 54) {
        destination = chooseRoamPoint(index, routeIndex + 1);
      }
      return {
        agentId,
        node,
        index,
        color,
        group,
        ring,
        sprite,
        speechBubble,
        packet,
        frameSet,
        point,
        destination,
        phase: index * 0.23,
        speed: 0.16 + index * 0.025,
        walkSpeed: 34 + index * 4,
        routeIndex,
        waitUntil: snapshot?.waitUntil ?? 0,
        facing: "down",
        facingSign: 1,
        isWalking: false,
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
        const fallbackPoint = resolvePoint(unit.node, points[unit.index % points.length], 0.5, 0.78);
        unit.point = clampRoamPoint(unit.point || fallbackPoint);
        unit.destination = clampRoamPoint(unit.destination || chooseRoamPoint(unit.index, 0));
        unit.group.x = unit.point.x;
        unit.group.y = unit.point.y;
      }
    };

    const rememberRoamSnapshots = (): void => {
      roamSnapshots = new Map(
        agentUnits.map((unit) => [
          unit.agentId,
          {
            point: { ...unit.point },
            destination: { ...unit.destination },
            routeIndex: unit.routeIndex,
            waitUntil: unit.waitUntil,
          },
        ])
      );
    };

    const rebuildAgentUnits = (): void => {
      rememberRoamSnapshots();
      for (const child of packetLayer.removeChildren()) child.destroy({ children: true });
      for (const child of agentLayer.removeChildren()) child.destroy({ children: true });
      agentUnits = root
        ? [...root.querySelectorAll<HTMLElement>(".desk[data-agent-id]")].map(makeAgentUnit)
        : [];
      syncLayout();
    };

    const setNextRoamTarget = (unit: AgentUnit): void => {
      unit.routeIndex += 1;
      unit.destination = chooseRoamPoint(unit.index, unit.routeIndex);
      if (distanceBetween(unit.point, unit.destination) < 72) {
        unit.routeIndex += 1;
        unit.destination = chooseRoamPoint(unit.index, unit.routeIndex);
      }
      unit.waitUntil = 0;
    };

    const updateFacing = (unit: AgentUnit, dx: number, dy: number): void => {
      if (Math.abs(dx) > Math.abs(dy) * 0.72) {
        unit.facing = "side";
        unit.facingSign = dx < 0 ? 1 : -1;
        return;
      }
      unit.facing = dy < 0 ? "up" : "down";
    };

    const updateRoamMotion = (unit: AgentUnit, delta: number): void => {
      const dt = Math.min(delta / 60, 0.08);
      const dx = unit.destination.x - unit.point.x;
      const dy = unit.destination.y - unit.point.y;
      const distance = Math.hypot(dx, dy);

      if (distance <= 3) {
        unit.isWalking = false;
        if (unit.waitUntil === 0) {
          unit.waitUntil = elapsed + 0.2 + seededRatio(unit.index, unit.routeIndex, 3) * 0.8;
        }
        if (elapsed >= unit.waitUntil) {
          setNextRoamTarget(unit);
        }
        return;
      }

      unit.isWalking = true;
      updateFacing(unit, dx, dy);
      const step = Math.min(distance, unit.walkSpeed * (1 + stageBurst * 0.1) * dt);
      unit.point = clampRoamPoint({
        x: unit.point.x + (dx / distance) * step,
        y: unit.point.y + (dy / distance) * step,
      });
    };

    const applyWalkFrame = (unit: AgentUnit): void => {
      if (!unit.sprite || !unit.frameSet) return;
      const frames = unit.frameSet[unit.facing];
      if (frames.length === 0) return;

      const frameIndex = unit.isWalking
        ? Math.floor((elapsed * 7.5 + unit.phase * 6) % frames.length)
        : 0;
      unit.sprite.texture = frames[frameIndex] || frames[0];
      unit.sprite.scale.set(
        unit.facing === "side" ? unit.facingSign * AGENT_SPRITE_SCALE : AGENT_SPRITE_SCALE,
        AGENT_SPRITE_SCALE
      );
    };

    const applySpeechBubbleFrame = (unit: AgentUnit): void => {
      if (!unit.speechBubble) return;
      const spriteHeight = AGENT_FRAME_HEIGHT * AGENT_SPRITE_SCALE;
      const driftX = Math.sin(elapsed * 1.7 + unit.phase * 11) * 1.5;
      const driftY = Math.sin(elapsed * 2.6 + unit.phase * 9) * 3;
      unit.speechBubble.group.x = driftX;
      unit.speechBubble.group.y =
        -spriteHeight - AGENT_SPEECH_BUBBLE_HEAD_GAP + driftY - stageBurst * 1.5;
      unit.speechBubble.group.alpha = speechBubblesEnabled
        ? 0.9 + Math.sin(elapsed * 2.2 + unit.phase * 5) * 0.06
        : 0;
      unit.speechBubble.group.scale.set(1 + stageBurst * 0.025);
    };

    const renderTick = (delta: number): void => {
      elapsed += delta / 60;
      stageBurst = Math.max(0, stageBurst - delta * 0.018);
      pathLayer.clear();

      for (const unit of agentUnits) {
        updateRoamMotion(unit, delta);
        applyWalkFrame(unit);
        applySpeechBubbleFrame(unit);

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
        unit.group.x = unit.point.x;
        unit.group.y = unit.point.y + bob - stageBurst * 1.5;
        unit.group.zIndex = unit.point.y;
        unit.ring.clear();
        unit.ring.lineStyle(2, unit.color, 0.58 + stageBurst * 0.2);
        const ringPulse = 1 + Math.sin(elapsed * 4.2 + unit.phase * 4) * 0.12 + stageBurst * 0.12;
        unit.ring.drawEllipse(0, 0, AGENT_RING_WIDTH * ringPulse, AGENT_RING_HEIGHT * ringPulse);
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
      agentFrameSets = buildAgentFrameSets(textures.agentAtlas);
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
      if (activeHandle?.app === app) activeHandle = null;
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

    const handle = { app, destroy, rebuildAgentUnits, setSpeechBubblesEnabled, syncLayout };
    activeHandle = handle;
    return handle;
  };

  window.AkraAdminGame = {
    ...(window.AkraAdminGame || {}),
    mountDiorama,
    setSpeechBubblesEnabled: (enabled: boolean): void => {
      activeHandle?.setSpeechBubblesEnabled(enabled);
    },
  };

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", mountDiorama, { once: true });
  } else {
    mountDiorama();
  }
})();

export {};
