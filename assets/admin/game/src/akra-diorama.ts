import "pixi.js/unsafe-eval";
import { Application, Assets, Texture, type Ticker } from "pixi.js";
import { AgentWorld } from "./agent-world";
import { SceneCameraController } from "./camera-controller";
import type {
  AkraAdminGameBridge,
  DioramaHandle,
  SceneInspection,
  SemanticZoomLevel,
} from "./game-types";
import { MAP_HEIGHT, MAP_WIDTH } from "./scene-config";
import { DashboardSceneStore } from "./scene-store";

const MAP_ASSET_URL = "/admin/assets/graphics/akra-operations-studio-v3.png";
const AGENT_ATLAS_URL = "/admin/assets/graphics/gamebaljeonguk_atlas_128x192.png";

export const ACTIVE_FRAME_INTERVAL_MS = 1000 / 60;
export const RAF_HEALTHY_GAP_MS = 80;

declare global {
  interface Window {
    AkraAdminGame?: AkraAdminGameBridge;
  }
}

const emptyInspection = (): SceneInspection => ({
  ready: false,
  actorCount: 0,
  characterCount: 0,
  standbyCount: 0,
  ambientActivityCount: 0,
  packetCount: 0,
  semanticMotionCount: 0,
  movementSpeedRatio: 0.3,
  renderCount: 0,
  planningRevision: null,
  zoomLevel: "overview",
  cameraZoom: 1,
  actors: [],
  standbyCharacters: [],
});

(() => {
  let activeHandle: DioramaHandle | null = null;
  let pendingDashboard: unknown = null;

  const mountDiorama = (): DioramaHandle | null => {
    const container = document.getElementById("pixi-diorama");
    if (!container) return null;
    const boardEl = container.closest<HTMLElement>(".office-board");
    const root = boardEl?.closest<HTMLElement>("[data-admin-graphic]");
    if (!boardEl || !root || container.dataset.akraDioramaMounted === "true") return null;

    container.dataset.akraDioramaMounted = "true";
    const app = new Application();
    const store = new DashboardSceneStore();
    let world: AgentWorld | null = null;
    let camera: SceneCameraController | null = null;
    let resizeObserver: ResizeObserver | null = null;
    let unsubscribeStore: (() => void) | null = null;
    let initialized = false;
    let ready = false;
    let destroyed = false;
    let lastInspectionSync = 0;
    let lastTickerAt = 0;
    let lastWorldUpdateAt = 0;
    let tickerHealthy = false;
    let frameDriver = "booting";
    let lastLayoutWidth = 0;
    let lastLayoutHeight = 0;
    let currentZoomLevel: SemanticZoomLevel = "overview";
    const cleanupCallbacks: Array<() => void> = [];

    const syncInspectionDataset = (): void => {
      const inspection = world?.inspectScene(ready) ?? emptyInspection();
      container.dataset.sceneReady = String(inspection.ready);
      container.dataset.sceneActorCount = String(inspection.actorCount);
      container.dataset.sceneCharacterCount = String(inspection.characterCount);
      container.dataset.sceneStandbyCount = String(inspection.standbyCount);
      container.dataset.sceneAmbientActivityCount = String(inspection.ambientActivityCount);
      container.dataset.scenePacketCount = String(inspection.packetCount);
      container.dataset.sceneSemanticMotionCount = String(inspection.semanticMotionCount);
      container.dataset.sceneMovementSpeedRatio = String(inspection.movementSpeedRatio);
      container.dataset.sceneRenderCount = String(inspection.renderCount);
      container.dataset.scenePlanningRevision = String(inspection.planningRevision ?? "");
      container.dataset.sceneZoomLevel = inspection.zoomLevel;
      container.dataset.sceneCameraZoom = inspection.cameraZoom.toFixed(3);
      container.dataset.sceneFrameDriver = frameDriver;
      container.dataset.sceneActorSignature = JSON.stringify(
        inspection.actors.map((actor) => ({
          actorId: actor.actorId,
          agentId: actor.agentId,
          slotId: actor.slotId,
          visualState: actor.visualState,
          pose: actor.pose,
          animationKind: actor.animationKind,
          animationFrameIndex: actor.animationFrameIndex,
          animationBlend: actor.animationBlend,
          gaitOffsetX: actor.gaitOffsetX,
          gaitOffsetY: actor.gaitOffsetY,
          x: actor.x,
          y: actor.y,
        }))
      );
      container.dataset.sceneStandbySignature = JSON.stringify(
        inspection.standbyCharacters.map((character) => ({
          characterId: character.characterId,
          agentId: character.agentId,
          ambientActivity: character.ambientActivity,
          visualState: character.visualState,
          pose: character.pose,
          animationKind: character.animationKind,
          animationFrameIndex: character.animationFrameIndex,
          animationBlend: character.animationBlend,
          gaitOffsetX: character.gaitOffsetX,
          gaitOffsetY: character.gaitOffsetY,
          resolvedAtlasFrameIndex: character.resolvedAtlasFrameIndex,
          poseFallback: character.poseFallback,
          x: character.x,
          y: character.y,
        }))
      );
    };

    const updateZoomPresentation = (
      zoomLevel: SemanticZoomLevel,
      userZoom: number
    ): void => {
      currentZoomLevel = zoomLevel;
      boardEl.dataset.sceneZoomLevel = zoomLevel;
      boardEl.style.setProperty("--scene-camera-zoom", userZoom.toFixed(3));
      const readout = boardEl.querySelector<HTMLElement>("[data-scene-zoom-readout]");
      if (readout) readout.textContent = `${Math.round(userZoom * 100)}%`;
      world?.setZoomLevel(zoomLevel, userZoom);
    };

    const boardSize = (): { width: number; height: number } => ({
      width: Math.max(1, Math.round(boardEl.clientWidth || boardEl.offsetWidth || 900)),
      height: Math.max(1, Math.round(boardEl.clientHeight || boardEl.offsetHeight || 540)),
    });

    const syncLayout = (): void => {
      if (!initialized || destroyed) return;
      const { width, height } = boardSize();
      if (width === lastLayoutWidth && height === lastLayoutHeight) {
        return;
      }
      lastLayoutWidth = width;
      lastLayoutHeight = height;
      app.renderer.resize(width, height);
      camera?.resize(width, height);
      syncInspectionDataset();
    };

    const requestSceneRender = (): void => {
      if (initialized && !destroyed) app.render();
    };

    const applyDashboard = (dashboard: unknown): boolean => {
      pendingDashboard = dashboard;
      const changed = store.applyDashboard(dashboard);
      if (changed) {
        syncInspectionDataset();
        requestSceneRender();
      }
      return changed;
    };

    const rebuildAgentUnits = (): void => {
      world?.reconcile(store.current());
      syncInspectionDataset();
      requestSceneRender();
    };

    const inspectScene = (): SceneInspection =>
      world?.inspectScene(ready) ?? {
        ...emptyInspection(),
        zoomLevel: currentZoomLevel,
        cameraZoom: camera?.currentZoom() ?? 1,
      };

    const resetCamera = (): void => {
      camera?.reset();
      syncInspectionDataset();
    };

    const zoomBy = (factor: number): void => {
      camera?.zoomBy(factor);
      syncInspectionDataset();
    };

    const bindControl = (selector: string, action: () => void): void => {
      const control = boardEl.querySelector<HTMLElement>(selector);
      if (!control) return;
      const listener = (event: Event): void => {
        event.preventDefault();
        action();
      };
      control.addEventListener("click", listener);
      cleanupCallbacks.push(() => control.removeEventListener("click", listener));
    };

    const onDashboardRendered = (event: Event): void => {
      const detail = (event as CustomEvent<{ dashboard?: unknown }>).detail;
      if (detail?.dashboard) applyDashboard(detail.dashboard);
    };

    const onResize = (): void => syncLayout();

    const destroy = (): void => {
      if (destroyed) return;
      destroyed = true;
      window.removeEventListener("akra:dashboard-rendered", onDashboardRendered);
      window.removeEventListener("resize", onResize);
      resizeObserver?.disconnect();
      unsubscribeStore?.();
      for (const cleanup of cleanupCallbacks) cleanup();
      camera?.destroy();
      world?.destroy();
      if (initialized) app.destroy(true, { children: true });
      boardEl.classList.remove("is-pixi-ready");
      delete container.dataset.akraDioramaMounted;
      if (activeHandle?.app === app) activeHandle = null;
    };

    const handle: DioramaHandle = {
      app,
      destroy,
      applyDashboard,
      rebuildAgentUnits,
      syncLayout,
      inspectScene,
      resetCamera,
      zoomBy,
    };
    activeHandle = handle;

    const bootstrap = async (): Promise<void> => {
      const { width, height } = boardSize();
      await app.init({
        width,
        height,
        backgroundAlpha: 0,
        antialias: false,
        autoDensity: true,
        resolution: Math.min(window.devicePixelRatio || 1, 2),
        preference: "webgl",
        powerPreference: "high-performance",
      });
      if (destroyed) {
        app.destroy(true, { children: true });
        return;
      }
      initialized = true;
      app.ticker.maxFPS = 60;
      app.canvas.className = "akra-world-canvas";
      app.canvas.setAttribute("aria-hidden", "true");
      app.canvas.setAttribute("tabindex", "-1");
      container.replaceChildren(app.canvas);

      const textureResults = await Promise.allSettled([
        Assets.load<Texture>(MAP_ASSET_URL),
        Assets.load<Texture>(AGENT_ATLAS_URL),
        document.fonts?.load("14px Galmuri11") ?? Promise.resolve([]),
      ]);
      if (destroyed) return;
      const mapResult = textureResults[0];
      const atlasResult = textureResults[1];
      if (mapResult.status !== "fulfilled" || atlasResult.status !== "fulfilled") {
        const reasons = textureResults
          .filter((result) => result.status === "rejected")
          .map((result) => String(result.reason))
          .join(" · ");
        throw new Error(`AKRA game assets failed to load${reasons ? `: ${reasons}` : ""}`);
      }

      atlasResult.value.source.scaleMode = "nearest";
      world = new AgentWorld(mapResult.value, atlasResult.value);
      app.stage.addChild(world.root);
      camera = new SceneCameraController({
        canvas: app.canvas,
        world: world.root,
        worldWidth: MAP_WIDTH,
        worldHeight: MAP_HEIGHT,
        onChange: updateZoomPresentation,
      });
      unsubscribeStore = store.subscribe((snapshot) => {
        world?.reconcile(snapshot);
        syncInspectionDataset();
      });

      app.ticker.add((ticker: Ticker) => {
        if (!world || destroyed || document.hidden) return;
        const now = performance.now();
        const tickerGap = lastTickerAt > 0 ? now - lastTickerAt : 0;
        lastTickerAt = now;
        tickerHealthy = tickerGap > 0 && tickerGap <= RAF_HEALTHY_GAP_MS;
        const deltaMilliseconds =
          lastWorldUpdateAt > 0 ? now - lastWorldUpdateAt : ticker.deltaMS;
        lastWorldUpdateAt = now;
        frameDriver = tickerHealthy ? "ticker" : "interval-fallback";
        world.update(deltaMilliseconds, now);
        if (now - lastInspectionSync >= 500) {
          lastInspectionSync = now;
          syncInspectionDataset();
        }
      });
      const fallbackTimer = window.setInterval(() => {
        if (!world || destroyed || document.hidden) return;
        const now = performance.now();
        if (tickerHealthy && now - lastTickerAt <= RAF_HEALTHY_GAP_MS) return;
        const deltaMilliseconds =
          lastWorldUpdateAt > 0
            ? now - lastWorldUpdateAt
            : ACTIVE_FRAME_INTERVAL_MS;
        lastWorldUpdateAt = now;
        frameDriver = "interval-fallback";
        world.update(deltaMilliseconds, now);
        app.render();
        if (now - lastInspectionSync >= 500) {
          lastInspectionSync = now;
          syncInspectionDataset();
        }
      }, ACTIVE_FRAME_INTERVAL_MS);
      cleanupCallbacks.push(() => window.clearInterval(fallbackTimer));
      const syncAnimationVisibility = (): void => {
        if (document.hidden) {
          frameDriver = "paused";
          app.ticker.stop();
        } else {
          lastTickerAt = 0;
          lastWorldUpdateAt = performance.now();
          tickerHealthy = false;
          app.ticker.start();
        }
        syncInspectionDataset();
      };
      document.addEventListener("visibilitychange", syncAnimationVisibility);
      cleanupCallbacks.push(() =>
        document.removeEventListener("visibilitychange", syncAnimationVisibility)
      );
      syncAnimationVisibility();

      bindControl("[data-scene-zoom-out]", () => zoomBy(1 / 1.22));
      bindControl("[data-scene-zoom-reset]", resetCamera);
      bindControl("[data-scene-zoom-in]", () => zoomBy(1.22));
      window.addEventListener("akra:dashboard-rendered", onDashboardRendered);
      window.addEventListener("resize", onResize);
      if (typeof ResizeObserver !== "undefined") {
        resizeObserver = new ResizeObserver(syncLayout);
        resizeObserver.observe(boardEl);
      }

      if (pendingDashboard) store.applyDashboard(pendingDashboard);
      ready = true;
      boardEl.classList.add("is-pixi-ready");
      container.dataset.sceneAsset = "akra-operations-studio-v3.png";
      syncLayout();
      camera.reset();
      syncInspectionDataset();
    };

    bootstrap().catch((error: unknown) => {
      console.error("Failed to initialize the AKRA Pixi scene", error);
      container.dataset.sceneReady = "false";
      container.dataset.sceneError =
        error instanceof Error ? error.message : "unknown scene initialization error";
      boardEl.classList.add("is-pixi-failed");
    });

    return handle;
  };

  window.AkraAdminGame = {
    ...(window.AkraAdminGame || {}),
    mountDiorama,
    applyDashboard: (dashboard: unknown): boolean => {
      pendingDashboard = dashboard;
      return activeHandle?.applyDashboard(dashboard) ?? false;
    },
    inspectScene: (): SceneInspection | null => activeHandle?.inspectScene() ?? null,
    resetCamera: (): void => activeHandle?.resetCamera(),
    zoomBy: (factor: number): void => activeHandle?.zoomBy(factor),
  };

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", mountDiorama, { once: true });
  } else {
    mountDiorama();
  }
})();

export { MAP_HEIGHT, MAP_WIDTH };
