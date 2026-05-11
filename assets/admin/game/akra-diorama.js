(() => {
  const mountDiorama = () => {
    const container = document.getElementById("pixi-diorama");
    if (!container || typeof PIXI === "undefined") return null;

    const boardEl = container.closest(".office-board");
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
    const assets = {
      floor: basePath + "sprite_floor_tile.png",
      desk: basePath + "sprite_desk_workstation.png",
      server: basePath + "sprite_server_rack.png",
      whiteboard: basePath + "sprite_whiteboard.png",
      sofa: basePath + "sprite_sofa.png",
      plant: basePath + "sprite_potted_plant.png",
      agentAtlas: basePath + "gamebaljeonguk_atlas_64x96.png",
    };

    const root = boardEl.closest("[data-admin-graphic]");
    const pathLayer = new PIXI.Graphics();
    const packetLayer = new PIXI.Container();
    const agentLayer = new PIXI.Container();
    app.stage.addChild(pathLayer, packetLayer, agentLayer);

    const statusPalette = {
      normal: 0x35d07f,
      success: 0x35d07f,
      warning: 0xf5c84b,
      danger: 0xff6b6b,
      info: 0x5da9ff,
      muted: 0x98abc4,
    };

    let textures = {};
    let agentFrames = [];
    let agentUnits = [];
    let stageBurst = 0;
    let elapsed = 0;
    let resizeObserver = null;

    const boardSize = () => ({
      width: boardEl.offsetWidth || initialWidth,
      height: boardEl.offsetHeight || initialHeight,
    });

    const fallbackPoints = () => {
      const { width, height } = boardSize();
      return [
        { x: width * 0.35, y: height * 0.50 },
        { x: width * 0.50, y: height * 0.65 },
        { x: width * 0.28, y: height * 0.72 },
        { x: width * 0.60, y: height * 0.52 },
        { x: width * 0.43, y: height * 0.82 },
      ];
    };

    let targets = {
      distributor: { x: initialWidth * 0.80, y: initialHeight * 0.52 },
      events: { x: initialWidth * 0.82, y: initialHeight * 0.76 },
    };

    const parseSeverity = (node) => {
      if (node?.dataset?.detailSeverity) return node.dataset.detailSeverity;
      if (node?.classList?.contains("severity-danger")) return "danger";
      if (node?.classList?.contains("severity-warning")) return "warning";
      if (node?.classList?.contains("severity-info")) return "info";
      return "normal";
    };

    const colorFor = (severity) => statusPalette[severity] || statusPalette.normal;

    const makeAtlasFrame = (texture, col, row = 0) => {
      const baseTexture = texture?.baseTexture;
      if (!baseTexture || typeof PIXI.Rectangle === "undefined") return null;
      return new PIXI.Texture(baseTexture, new PIXI.Rectangle(col * 64, row * 96, 64, 96));
    };

    const resolvePoint = (node, fallback, xBias = 0.5, yBias = 0.76) => {
      if (!node) return fallback;
      const boardRect = boardEl.getBoundingClientRect();
      const rect = node.getBoundingClientRect();
      if (rect.width <= 0 && rect.height <= 0) return fallback;
      return {
        x: rect.left - boardRect.left + rect.width * xBias,
        y: rect.top - boardRect.top + rect.height * yBias,
      };
    };

    const makePacket = (color) => {
      const packet = new PIXI.Graphics();
      packet.beginFill(color, 0.92);
      packet.lineStyle(1, 0xffffff, 0.42);
      packet.drawPolygon([0, -6, 6, 0, 0, 6, -6, 0]);
      packet.endFill();
      packet.alpha = 0.84;
      packetLayer.addChild(packet);
      return packet;
    };

    const makeAgentUnit = (node, index) => {
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

    const syncLayout = () => {
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

    const rebuildAgentUnits = () => {
      for (const child of packetLayer.removeChildren()) child.destroy({ children: true });
      for (const child of agentLayer.removeChildren()) child.destroy({ children: true });
      agentUnits = root
        ? [...root.querySelectorAll(".desk[data-agent-id]")].map(makeAgentUnit)
        : [];
      syncLayout();
    };

    const lerp = (a, b, t) => a + (b - a) * t;

    const renderTick = (delta) => {
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

    const loadTextures = async () => {
      textures = {};
      for (const [key, url] of Object.entries(assets)) {
        try {
          textures[key] = await PIXI.Assets.load(url);
        } catch (error) {
          console.warn(`Failed to load sprite: ${key}`, error);
        }
      }
      agentFrames = Array.from({ length: 8 }, (_, index) =>
        makeAtlasFrame(textures.agentAtlas, index)
      ).filter(Boolean);
    };

    const onMissionPulse = (event) => {
      const count = Number(event.detail?.count || 1);
      stageBurst = Math.min(stageBurst + 0.45 + count * 0.06, 1.6);
    };
    const onResize = () => syncLayout();

    const destroy = () => {
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
