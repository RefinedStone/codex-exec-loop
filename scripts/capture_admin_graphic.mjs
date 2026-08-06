import { chromium } from "@playwright/test";

const options = Object.fromEntries(
  process.argv.slice(2).map((argument) => {
    const [key, ...value] = argument.split("=");
    return [key.replace(/^--/, ""), value.join("=")];
  }),
);
const browserPath = options.browser;
const targetUrl = options.url;
const screenshotPath = options.screenshot;
const mobileScreenshotPath = options["mobile-screenshot"];
const compactScreenshotPath = options["compact-screenshot"];
const fullHdScreenshotPath = options["full-hd-screenshot"];
const qhdScreenshotPath = options["qhd-screenshot"];
const token = process.env.AKRA_ADMIN_VISUAL_TOKEN;

if (
  !browserPath ||
  !targetUrl ||
  !screenshotPath ||
  !mobileScreenshotPath ||
  !compactScreenshotPath ||
  !fullHdScreenshotPath ||
  !qhdScreenshotPath ||
  !token
) {
  throw new Error(
    "browser, url, screenshot, mobile-screenshot, compact-screenshot, full-hd-screenshot, qhd-screenshot, and AKRA_ADMIN_VISUAL_TOKEN are required",
  );
}

const target = new URL(targetUrl);
const baseUrl = `${target.protocol}//${target.host}`;
const browser = await chromium.launch({
  executablePath: browserPath,
  headless: true,
  args: [
    "--no-sandbox",
    ...(target.hostname.endsWith(".localhost")
      ? [`--host-resolver-rules=MAP ${target.hostname} 127.0.0.1`]
      : []),
  ],
});

try {
  const context = await browser.newContext();
  const loginPage = await context.newPage();
  const loginDocument = await loginPage.goto(`${baseUrl}/admin/login`, {
    waitUntil: "networkidle",
  });
  if (!loginDocument?.ok()) {
    throw new Error(`admin browser login page returned ${loginDocument?.status() ?? "no response"}`);
  }
  await loginPage.locator('input[name="token"]').fill(token);
  const loginResponsePromise = loginPage.waitForResponse(
    (response) =>
      response.url() === `${baseUrl}/admin/login` && response.request().method() === "POST",
  );
  await loginPage.locator('button[type="submit"]').click();
  const login = await loginResponsePromise;
  if (login.status() !== 303) {
    throw new Error(`admin browser login returned ${login.status()}, expected 303`);
  }
  await loginPage.waitForURL(`${baseUrl}/admin`);
  await loginPage.close();

  const session = (await context.cookies(baseUrl)).find(
    (cookie) => cookie.name === "akra_admin_session",
  );
  if (!session || !session.httpOnly || session.sameSite !== "Strict") {
    throw new Error("admin browser login did not establish the strict HttpOnly session cookie");
  }

  const captureViewport = async ({ width, height, path, label }) => {
    const page = await context.newPage();
    const browserErrors = [];
    page.on("console", (message) => {
      if (message.type() === "error") {
        browserErrors.push(
          `console: ${message.text()} @ ${JSON.stringify(message.location())}`,
        );
      }
    });
    page.on("pageerror", (error) => browserErrors.push(`page: ${error.message}`));
    page.on("response", (response) => {
      if (response.status() >= 400) {
        browserErrors.push(`response: ${response.status()} ${response.url()}`);
      }
    });
    await page.setViewportSize({ width, height });

    // The game dashboard intentionally keeps an EventSource connection open.
    // Waiting for networkidle turns a healthy realtime stream into a timeout.
    const response = await page.goto(targetUrl, { waitUntil: "domcontentloaded" });
    if (!response?.ok() || page.url().includes("/admin/login")) {
      throw new Error(
        `${label} authenticated admin page failed to load: ${response?.status() ?? "no response"}`,
      );
    }

    const fontState = await page.evaluate(async () => {
      await document.fonts.ready;
      return {
        regular: document.fonts.check("12px Galmuri11", "게임발전국"),
        bold: document.fonts.check("700 12px Galmuri11", "운영 상태"),
        bodyFamily: window.getComputedStyle(document.body).fontFamily,
      };
    });
    if (!fontState.regular || !fontState.bold || !fontState.bodyFamily.includes("Galmuri11")) {
      throw new Error(`${label} bundled Korean font did not load: ${JSON.stringify(fontState)}`);
    }

    await page.waitForFunction(() => {
      const container = document.querySelector("#pixi-diorama");
      const canvas = container?.querySelector("canvas");
      return (
        container?.getAttribute("data-akra-diorama-mounted") === "true" &&
        canvas instanceof HTMLCanvasElement &&
        canvas.width > 64 &&
        canvas.height > 64
      );
    });
    await page.waitForFunction(() => window.AkraAdminGame?.inspectScene?.()?.ready === true);

    const canvas = page.locator("#pixi-diorama canvas");
    const analysisPage = await context.newPage();
    const inspectCanvasFrame = async () => {
      // Reading a WebGL canvas with toDataURL requires preserveDrawingBuffer,
      // which the production renderer deliberately leaves disabled. A locator
      // screenshot captures the compositor output without changing that runtime
      // performance contract.
      const imageBuffer = await canvas.screenshot({ type: "png" });
      const imageUrl = `data:image/png;base64,${imageBuffer.toString("base64")}`;
      return analysisPage.evaluate(
        (source) =>
          new Promise((resolve) => {
            const image = new Image();
            image.onload = () => {
              const sampleWidth = Math.min(image.naturalWidth, 256);
              const sampleHeight = Math.min(image.naturalHeight, 256);
              const analysisCanvas = document.createElement("canvas");
              analysisCanvas.width = sampleWidth;
              analysisCanvas.height = sampleHeight;
              const context = analysisCanvas.getContext("2d", {
                willReadFrequently: true,
              });
              if (!context) {
                resolve({ error: "2D analysis context missing" });
                return;
              }
              context.drawImage(image, 0, 0, sampleWidth, sampleHeight);
              const pixels = context.getImageData(0, 0, sampleWidth, sampleHeight).data;
              let nonTransparent = 0;
              let nonBlack = 0;
              let checksum = 0;
              for (let index = 0; index < pixels.length; index += 4) {
                const red = pixels[index];
                const green = pixels[index + 1];
                const blue = pixels[index + 2];
                const alpha = pixels[index + 3];
                if (alpha !== 0) nonTransparent += 1;
                if (red !== 0 || green !== 0 || blue !== 0) nonBlack += 1;
                checksum = (checksum * 33 + red + green * 3 + blue * 7 + alpha * 11) >>> 0;
              }
              resolve({
                width: image.naturalWidth,
                height: image.naturalHeight,
                nonTransparent,
                nonBlack,
                checksum,
              });
            };
            image.onerror = () => resolve({ error: "captured canvas PNG could not be decoded" });
            image.src = source;
          }),
        imageUrl,
      );
    };

    await page.waitForTimeout(100);
    const firstFrame = await inspectCanvasFrame();
    const firstScene = await page.evaluate(() => window.AkraAdminGame?.inspectScene?.());
    await page.waitForTimeout(300);
    const secondFrame = await inspectCanvasFrame();
    const secondScene = await page.evaluate(() => window.AkraAdminGame?.inspectScene?.());
    if (browserErrors.length > 0) {
      await page.screenshot({ path, fullPage: true });
      throw new Error(`${label} browser errors:\n${browserErrors.join("\n")}`);
    }
    for (const frame of [firstFrame, secondFrame]) {
      if (frame.error) throw new Error(`${label} ${frame.error}`);
      if (frame.nonTransparent < 100 || frame.nonBlack < 100) {
        throw new Error(`${label} WebGL canvas is blank: ${JSON.stringify(frame)}`);
      }
    }
    for (const scene of [firstScene, secondScene]) {
      if (!scene || scene.packetCount < 0 || scene.semanticMotionCount < 0) {
        throw new Error(`${label} scene reported invalid semantic motion: ${JSON.stringify(scene)}`);
      }
      if (scene.movementSpeedRatio !== 0.3) {
        throw new Error(
          `${label} scene reported an unexpected worker movement ratio: ${JSON.stringify(scene.movementSpeedRatio)}`,
        );
      }
    }
    const semanticMotion = Math.max(
      firstScene.semanticMotionCount,
      secondScene.semanticMotionCount,
    );
    if (semanticMotion > 0 && (
      firstFrame.checksum === secondFrame.checksum
      || firstScene.renderCount >= secondScene.renderCount
    )) {
      throw new Error(
        `${label} typed worker motion did not advance: ${JSON.stringify({ firstFrame, secondFrame, firstScene, secondScene })}`,
      );
    }
    const actorParity = await page.evaluate(() => {
      const dom = [...document.querySelectorAll(".desk[data-actor-id]")].map((node) => ({
        actorId: node.dataset.actorId,
        agentId: node.dataset.agentId,
        slotId: node.dataset.slotId,
        visualState: node.dataset.visualState,
        pose: node.dataset.staticPose,
      }));
      const canvasActors = window.AkraAdminGame?.inspectScene?.()?.actors || [];
      return { dom, canvasActors };
    });
    if (
      JSON.stringify(actorParity.dom) !==
      JSON.stringify(
        actorParity.canvasActors.map(
          ({
            x,
            y,
            displayWidth,
            displayHeight,
            opacity,
            boardX,
            boardY,
            resolvedAtlasFrameIndex,
            poseFallback,
            animationKind,
            animationFrameIndex,
            sourceFrameIndex,
            animationBlend,
            gaitOffsetX,
            gaitOffsetY,
            frameScale,
            ...actor
          }) => actor,
        ),
      )
    ) {
      throw new Error(`${label} DOM/canvas actor identity mismatch: ${JSON.stringify(actorParity)}`);
    }
    const standbyParity = await page.evaluate(() => {
      const dom = [...document.querySelectorAll("[data-standby-character]")].map((node) => ({
        characterId: node.dataset.characterId,
        presenceKind: node.dataset.presenceKind,
        agentId: node.dataset.agentId,
        visualState: node.dataset.visualState,
        pose: node.dataset.staticPose,
        locationIndex: Number(node.dataset.sceneStandbyIndex),
      }));
      const scene = window.AkraAdminGame?.inspectScene?.();
      const restArea = document.querySelector("[data-standby-rest-area]");
      const canvas = document.querySelector("#pixi-diorama canvas");
      return {
        dom,
        canvasCharacters: scene?.standbyCharacters || [],
        canvasWidth: canvas?.clientWidth || 0,
        canvasHeight: canvas?.clientHeight || 0,
        actorCount: scene?.actorCount,
        characterCount: scene?.characterCount,
        standbyCount: scene?.standbyCount,
        standbyTotalCount: Number(restArea?.dataset.standbyTotalCount),
      };
    });
    const canvasStandbyIdentity = standbyParity.canvasCharacters.map(
      ({
        x,
        y,
        displayWidth,
        displayHeight,
        opacity,
        boardX,
        boardY,
        ambientActivity,
        resolvedAtlasFrameIndex,
        poseFallback,
        animationKind,
        animationFrameIndex,
        sourceFrameIndex,
        animationBlend,
        gaitOffsetX,
        gaitOffsetY,
        frameScale,
        ...character
      }) => character,
    );
    const expectedStandbyPoses = ["laptop", "sit", "laptop"];
    const standbyCoordinateKeys = new Set(
      standbyParity.canvasCharacters.map((character) => `${character.x}:${character.y}`),
    );
    const standbyWithinLounge = standbyParity.canvasCharacters.every(
      (character) =>
        character.x >= 70 &&
        character.x <= 430 &&
        character.y >= 660 &&
        character.y <= 780,
    );
    const minimumStandbyWidth = width >= 1920 ? 64 : 40;
    const minimumStandbyHeight = width >= 1920 ? 96 : 60;
    const standbyCharactersAreLegible = standbyParity.canvasCharacters.every(
      (character) =>
        character.displayWidth >= minimumStandbyWidth &&
        character.displayHeight >= minimumStandbyHeight &&
        character.opacity >= 0.98 &&
        character.y / 941 <= 0.84,
    );
    const standbyBoundsAreInsideCanvas = standbyParity.canvasCharacters.every(
      (character) =>
        character.boardX - character.displayWidth / 2 >= 0 &&
        character.boardX + character.displayWidth / 2 <= standbyParity.canvasWidth &&
        character.boardY - character.displayHeight >= 0 &&
        character.boardY <= standbyParity.canvasHeight,
    );
    const standbyScreenSpacingIsSafe = standbyParity.canvasCharacters.every(
      (character, index, all) =>
        all.slice(index + 1).every((other) => {
          const centerDistance = Math.hypot(
            character.boardX - other.boardX,
            character.boardY - other.boardY,
          );
          const nonOverlappingDistance =
            (character.displayWidth + other.displayWidth) / 2 + 4;
          return centerDistance >= nonOverlappingDistance;
        }),
    );
    if (
      standbyParity.dom.length === 0 ||
      JSON.stringify(standbyParity.dom) !== JSON.stringify(canvasStandbyIdentity) ||
      standbyParity.standbyCount !== standbyParity.dom.length ||
      !Number.isInteger(standbyParity.standbyTotalCount) ||
      standbyParity.standbyTotalCount < standbyParity.standbyCount ||
      standbyParity.characterCount !== standbyParity.actorCount + standbyParity.standbyCount ||
      standbyCoordinateKeys.size !== standbyParity.canvasCharacters.length ||
      !standbyWithinLounge ||
      !standbyCharactersAreLegible ||
      !standbyBoundsAreInsideCanvas ||
      !standbyScreenSpacingIsSafe ||
      standbyParity.canvasCharacters.some((character) => {
        const index = character.locationIndex - 1;
        return (
          character.pose !== expectedStandbyPoses[index] ||
          !character.ambientActivity ||
          character.animationKind !== "walk" ||
          !Number.isInteger(character.animationFrameIndex) ||
          character.resolvedAtlasFrameIndex !== null ||
          !character.poseFallback ||
          character.frameScale < 0.99
        );
      })
    ) {
      throw new Error(
        `${label} configured standby characters are missing or not visibly walking in the lounge: ${JSON.stringify(standbyParity)}`,
      );
    }
    if (width === 1920) {
      const baselineActorCount = firstScene.actorCount;
      const guardianStandbyWalkSamples = [];
      for (let sampleIndex = 0; sampleIndex < 12; sampleIndex += 1) {
        await page.waitForTimeout(100);
        const sample = await page.evaluate(() =>
          window.AkraAdminGame
            ?.inspectScene?.()
            ?.standbyCharacters.find(
              (character) => character.agentId === "agent-guardian",
            ),
        );
        if (sample) guardianStandbyWalkSamples.push(sample);
      }
      const guardianStandbySourceFrames = new Set(
        guardianStandbyWalkSamples.map((sample) => sample.sourceFrameIndex),
      );
      if (
        guardianStandbySourceFrames.has(2) ||
        !guardianStandbySourceFrames.has(0) ||
        !guardianStandbySourceFrames.has(1) ||
        !guardianStandbySourceFrames.has(3)
      ) {
        throw new Error(
          `${label} Guardian standby walk did not preserve its verified source stride: ${JSON.stringify(guardianStandbyWalkSamples)}`,
        );
      }
      await page.evaluate(async () => {
        const nativeFetch = window.fetch.bind(window);
        const response = await nativeFetch("/api/admin/akra/dashboard", {
          headers: { Accept: "application/json" },
        });
        if (!response.ok) throw new Error(`visual probe dashboard ${response.status}`);
        const dashboard = await response.json();
        dashboard.scene.actors.push({
          actorId: "visual-probe-session",
          agentId: "visual-probe-agent",
          taskId: "visual-probe-task",
          slotId: "visual-probe-slot",
          seatIndex: 1,
          displayName: "Visual Probe",
          archetypeKey: "Artificer",
          roleLabel: "Probe",
          visualState: "working",
          staticPose: "laptop",
          severity: "success",
          statusLabel: "작업 중",
          taskTitle: "Visual probe",
          bubbleLabel: "작업 중",
        });
        window.__akraVisualProbeDashboard = dashboard;
        window.fetch = (input, init) => {
          const requestUrl =
            typeof input === "string" ? input : input?.url ?? String(input);
          const pathname = new URL(requestUrl, window.location.href).pathname;
          if (
            pathname === "/api/admin/akra/dashboard" &&
            window.__akraVisualProbeDashboard
          ) {
            return Promise.resolve(
              new Response(JSON.stringify(window.__akraVisualProbeDashboard), {
                status: 200,
                headers: { "Content-Type": "application/json" },
              }),
            );
          }
          return nativeFetch(input, init);
        };
        window.AkraAdminGame?.applyDashboard?.(dashboard);
      });
      await page.waitForFunction(
        (expectedActorCount) => {
          window.AkraAdminGame?.applyDashboard?.(window.__akraVisualProbeDashboard);
          return window.AkraAdminGame?.inspectScene?.()?.actorCount === expectedActorCount;
        },
        baselineActorCount + 1,
      );
      const activePoseProbe = await page.evaluate(() =>
        window.AkraAdminGame
          ?.inspectScene?.()
          ?.actors.find((actor) => actor.actorId === "visual-probe-session"),
      );
      if (
        !activePoseProbe ||
        activePoseProbe.visualState !== "working" ||
        activePoseProbe.pose !== "laptop" ||
        activePoseProbe.displayWidth < 64 ||
        activePoseProbe.displayHeight < 96 ||
        activePoseProbe.opacity < 0.98 ||
        activePoseProbe.resolvedAtlasFrameIndex !== null ||
        !activePoseProbe.poseFallback
      ) {
        throw new Error(
          `${label} active actor did not use the map-compatible rear-facing workstation pose: ${JSON.stringify(activePoseProbe)}`,
        );
      }
      await page.evaluate(() => {
        const dashboard = window.__akraVisualProbeDashboard;
        const probe = dashboard?.scene?.actors?.find(
          (actor) => actor.actorId === "visual-probe-session",
        );
        if (!probe) throw new Error("blocked pose probe cannot find the active actor");
        probe.archetypeKey = "Guardian";
        probe.visualState = "blocked";
        probe.staticPose = "alert";
        probe.severity = "danger";
        window.AkraAdminGame?.applyDashboard?.(dashboard);
      });
      await page.waitForFunction(() => {
        window.AkraAdminGame?.applyDashboard?.(window.__akraVisualProbeDashboard);
        return window.AkraAdminGame
          ?.inspectScene?.()
          ?.actors.some(
            (actor) =>
              actor.actorId === "visual-probe-session" && actor.visualState === "blocked",
          );
      });
      const blockedPoseProbe = await page.evaluate(() =>
        window.AkraAdminGame
          ?.inspectScene?.()
          ?.actors.find((actor) => actor.actorId === "visual-probe-session"),
      );
      if (
        !blockedPoseProbe ||
        blockedPoseProbe.pose !== "alert" ||
        blockedPoseProbe.resolvedAtlasFrameIndex !== null ||
        !blockedPoseProbe.poseFallback
      ) {
        throw new Error(
          `${label} Guardian alert did not report its explicit neutral fallback: ${JSON.stringify(blockedPoseProbe)}`,
        );
      }
      await page.evaluate(() => {
        const dashboard = window.__akraVisualProbeDashboard;
        const probe = dashboard?.scene?.actors?.find(
          (actor) => actor.actorId === "visual-probe-session",
        );
        if (!probe) throw new Error("Guardian walk probe cannot find the active actor");
        probe.visualState = "delivering";
        probe.staticPose = "neutral";
        window.AkraAdminGame?.applyDashboard?.(dashboard);
      });
      await page.waitForFunction(() => {
        window.AkraAdminGame?.applyDashboard?.(window.__akraVisualProbeDashboard);
        const guardian = window.AkraAdminGame
          ?.inspectScene?.()
          ?.actors.find((actor) => actor.actorId === "visual-probe-session");
        return guardian?.animationKind === "walk" && guardian.frameScale >= 1.09;
      });
      const guardianWalkProbe = await page.evaluate(() =>
        window.AkraAdminGame
          ?.inspectScene?.()
          ?.actors.find((actor) => actor.actorId === "visual-probe-session"),
      );
      const guardianWalkSamples = [];
      for (let sampleIndex = 0; sampleIndex < 12; sampleIndex += 1) {
        await page.waitForTimeout(100);
        const sample = await page.evaluate(() =>
          window.AkraAdminGame
            ?.inspectScene?.()
            ?.actors.find((actor) => actor.actorId === "visual-probe-session"),
        );
        if (sample) guardianWalkSamples.push(sample);
      }
      const guardianSourceFrames = new Set(
        guardianWalkSamples.map((sample) => sample.sourceFrameIndex),
      );
      if (
        !guardianWalkProbe ||
        guardianWalkProbe.visualState !== "delivering" ||
        guardianWalkProbe.animationKind !== "walk" ||
        guardianWalkProbe.frameScale < 1.09 ||
        guardianSourceFrames.has(2) ||
        !guardianSourceFrames.has(0) ||
        !guardianSourceFrames.has(1) ||
        !guardianSourceFrames.has(3)
      ) {
        throw new Error(
          `${label} Guardian side walk did not preserve its verified source stride: ${JSON.stringify({ guardianWalkProbe, guardianWalkSamples })}`,
        );
      }
      await page.evaluate(() => {
        const dashboard = window.__akraVisualProbeDashboard;
        dashboard.scene.actors = dashboard.scene.actors.filter(
          (actor) => actor.actorId !== "visual-probe-session",
        );
        window.AkraAdminGame?.applyDashboard?.(dashboard);
      });
      await page.waitForFunction(
        (expectedActorCount) => {
          window.AkraAdminGame?.applyDashboard?.(window.__akraVisualProbeDashboard);
          return window.AkraAdminGame?.inspectScene?.()?.actorCount === expectedActorCount;
        },
        baselineActorCount,
      );
      const baselineStandbyCount = firstScene.standbyCount;
      await page.evaluate(() => {
        const dashboard = window.__akraVisualProbeDashboard;
        dashboard.scene.standbyCharacters.push({
          characterId: "standby:visual-ranger-probe",
          agentId: "visual-ranger-probe",
          locationIndex: 1,
          displayName: "Ranger Probe",
          archetypeKey: "Ranger",
          roleLabel: "Probe",
          presenceKind: "configured_standby",
          visualState: "idle",
          staticPose: "neutral",
          severity: "muted",
          statusLabel: "대기",
          bubbleLabel: "업무 배정 대기",
        });
        window.AkraAdminGame?.applyDashboard?.(dashboard);
      });
      await page.waitForFunction(
        (expectedStandbyCount) => {
          window.AkraAdminGame?.applyDashboard?.(window.__akraVisualProbeDashboard);
          return window.AkraAdminGame?.inspectScene?.()?.standbyCount === expectedStandbyCount;
        },
        baselineStandbyCount + 1,
      );
      await page.waitForFunction(() => {
        window.AkraAdminGame?.applyDashboard?.(window.__akraVisualProbeDashboard);
        const character = window.AkraAdminGame
          ?.inspectScene?.()
          ?.standbyCharacters.find(
            (candidate) => candidate.characterId === "standby:visual-ranger-probe",
          );
        return character?.animationKind === "walk"
          && Number.isInteger(character.animationFrameIndex);
      });
      const rangerStandbyProbe = await page.evaluate(() =>
        window.AkraAdminGame
          ?.inspectScene?.()
          ?.standbyCharacters.find(
            (character) => character.characterId === "standby:visual-ranger-probe",
          ),
      );
      if (
        !rangerStandbyProbe ||
        rangerStandbyProbe.pose !== "neutral" ||
        !rangerStandbyProbe.ambientActivity ||
        rangerStandbyProbe.animationKind !== "walk" ||
        !Number.isInteger(rangerStandbyProbe.animationFrameIndex) ||
        rangerStandbyProbe.resolvedAtlasFrameIndex !== null ||
        rangerStandbyProbe.poseFallback
      ) {
        throw new Error(
          `${label} Ranger standby did not start its neutral lounge walk: ${JSON.stringify(rangerStandbyProbe)}`,
        );
      }
      const loungeWalkSamples = [];
      for (let sampleIndex = 0; sampleIndex < 4; sampleIndex += 1) {
        loungeWalkSamples.push(
          await page.evaluate(() => {
            window.AkraAdminGame?.applyDashboard?.(window.__akraVisualProbeDashboard);
            return window.AkraAdminGame
              ?.inspectScene?.()
              ?.standbyCharacters.find(
                (character) =>
                  character.characterId === "standby:visual-ranger-probe",
              );
          }),
        );
        await page.waitForTimeout(560);
      }
      const loungeWalkFrames = new Set(
        loungeWalkSamples.map((character) => character?.animationFrameIndex),
      );
      const loungeWalkPoints = new Set(
        loungeWalkSamples.map((character) => `${character?.x}:${character?.y}`),
      );
      if (loungeWalkFrames.size < 2 || loungeWalkPoints.size < 2) {
        throw new Error(
          `${label} Ranger standby did not advance through its lounge walk: ${JSON.stringify(loungeWalkSamples)}`,
        );
      }
      await page.evaluate(() => {
        const dashboard = window.__akraVisualProbeDashboard;
        dashboard.scene.standbyCharacters = dashboard.scene.standbyCharacters.filter(
          (character) => character.characterId !== "standby:visual-ranger-probe",
        );
        window.AkraAdminGame?.applyDashboard?.(dashboard);
      });
      await page.waitForFunction(
        (expectedStandbyCount) => {
          window.AkraAdminGame?.applyDashboard?.(window.__akraVisualProbeDashboard);
          return window.AkraAdminGame?.inspectScene?.()?.standbyCount === expectedStandbyCount;
        },
        baselineStandbyCount,
      );
      await page.waitForTimeout(50);
    }
    await analysisPage.close();

    const layout = await page.evaluate(() => {
      const root = document.documentElement;
      const body = document.body;
      const canvas = document.querySelector("#pixi-diorama canvas");
      const board = document.querySelector(".office-board");
      const refresh = document.querySelector(".command-summary [data-refresh-dashboard]");
      const realtimeStatus = document.querySelector("[data-realtime-status]");
      const sidebar = document.querySelector(".sidebar");
      const attention = document.querySelector(".attention-strip");
      const commandControls = document.querySelector(".command-controls");
      const topbar = document.querySelector(".draft-topbar");
      const mainGrid = document.querySelector(".draft-main-grid");
      const shell = document.querySelector(".shell");
      const graphic = document.querySelector("[data-admin-graphic]");
      const boardRow = document.querySelector(".draft-main-grid > .board-row");
      const leftStack = document.querySelector(".draft-main-grid > .left-stack");
      const rightStack = document.querySelector(".draft-main-grid > .right-stack");
      const bottomGrid = document.querySelector(".bottom-grid");
      const canvasRect = canvas?.getBoundingClientRect();
      const boardRect = board?.getBoundingClientRect();
      const refreshRect = refresh?.getBoundingClientRect();
      const realtimeStatusRect = realtimeStatus?.getBoundingClientRect();
      const sidebarRect = sidebar?.getBoundingClientRect();
      const attentionRect = attention?.getBoundingClientRect();
      const commandControlsRect = commandControls?.getBoundingClientRect();
      const topbarRect = topbar?.getBoundingClientRect();
      const mainGridRect = mainGrid?.getBoundingClientRect();
      const shellRect = shell?.getBoundingClientRect();
      const graphicRect = graphic?.getBoundingClientRect();
      const boardRowRect = boardRow?.getBoundingClientRect();
      const leftStackRect = leftStack?.getBoundingClientRect();
      const rightStackRect = rightStack?.getBoundingClientRect();
      const bottomGridRect = bottomGrid?.getBoundingClientRect();
      const shellStyle = shell ? window.getComputedStyle(shell) : null;
      const shellContentWidth = shellRect
        ? shellRect.width -
          Number.parseFloat(shellStyle?.paddingLeft || "0") -
          Number.parseFloat(shellStyle?.paddingRight || "0")
        : 0;
      const isVisible = (element, rect) => {
        if (!element || !rect || rect.width <= 0 || rect.height <= 0) return false;
        const style = window.getComputedStyle(element);
        return (
          style.display !== "none" &&
          style.visibility !== "hidden" &&
          Number(style.opacity) > 0
        );
      };
      const isInside = (inner, outer) =>
        Boolean(
          inner &&
            outer &&
            inner.left >= outer.left - 1 &&
            inner.right <= outer.right + 1 &&
            inner.top >= outer.top - 1 &&
            inner.bottom <= outer.bottom + 1,
        );
      const centerHitIsInside = (element, rect) => {
        if (!element || !rect) return false;
        const hit = document.elementFromPoint(
          rect.left + rect.width / 2,
          rect.top + rect.height / 2,
        );
        return Boolean(hit && element.contains(hit));
      };
      const rectValue = (rect) =>
        rect
          ? {
              left: rect.left,
              right: rect.right,
              top: rect.top,
              bottom: rect.bottom,
              width: rect.width,
              height: rect.height,
            }
          : null;
      return {
        documentWidth: root.scrollWidth,
        viewportWidth: root.clientWidth,
        bodyWidth: body.scrollWidth,
        canvasInsideBoard: isInside(canvasRect, boardRect),
        topbarVisible: isVisible(topbar, topbarRect),
        realtimeStatusVisible: isVisible(realtimeStatus, realtimeStatusRect),
        refreshVisible: isVisible(refresh, refreshRect),
        refreshInsideTopbar: isInside(refreshRect, topbarRect),
        refreshHitTestable:
          centerHitIsInside(refresh, refreshRect) &&
          window.getComputedStyle(refresh).pointerEvents !== "none" &&
          !refresh.disabled,
        refreshWidth: refreshRect?.width ?? 0,
        refreshHeight: refreshRect?.height ?? 0,
        sidebarVisible: isVisible(sidebar, sidebarRect),
        attentionVisible: isVisible(attention, attentionRect),
        commandControlsVisible: isVisible(commandControls, commandControlsRect),
        topbarHeight: topbarRect?.height ?? 0,
        attentionHeight: attentionRect?.height ?? 0,
        commandControlsHeight: commandControlsRect?.height ?? 0,
        graphicCentered:
          Boolean(graphicRect && shellRect) &&
          Math.abs(
            (graphicRect.left + graphicRect.right) / 2 -
              (shellRect.left + shellRect.right) / 2,
          ) <= 1.5,
        graphicWidth: graphicRect?.width ?? 0,
        shellContentWidth,
        boardWidth: boardRect?.width ?? 0,
        boardAspectRatio:
          boardRect && boardRect.height > 0 ? boardRect.width / boardRect.height : 0,
        leftBoardGap:
          leftStackRect && boardRowRect ? boardRowRect.left - leftStackRect.right : null,
        boardRightGap:
          boardRowRect && rightStackRect ? rightStackRect.left - boardRowRect.right : null,
        bottomStartsInFirstViewport:
          Boolean(bottomGridRect) && bottomGridRect.top <= window.innerHeight + 1,
        mainBottomGap:
          mainGridRect && bottomGridRect ? bottomGridRect.top - mainGridRect.bottom : null,
        boardInFirstViewport:
          Boolean(boardRect) &&
          boardRect.top >= -1 &&
          boardRect.bottom <= window.innerHeight + 1,
        boardRect: rectValue(boardRect),
        sidebarRect: rectValue(sidebarRect),
        attentionRect: rectValue(attentionRect),
        commandControlsRect: rectValue(commandControlsRect),
        topbarRect: rectValue(topbarRect),
        mainGridRect: rectValue(mainGridRect),
        realtimeStatusRect: rectValue(realtimeStatusRect),
        refreshRect: rectValue(refreshRect),
        graphicRect: rectValue(graphicRect),
        boardRowRect: rectValue(boardRowRect),
        leftStackRect: rectValue(leftStackRect),
        rightStackRect: rectValue(rightStackRect),
        bottomGridRect: rectValue(bottomGridRect),
      };
    });
    if (layout.documentWidth > layout.viewportWidth + 1 || layout.bodyWidth > layout.viewportWidth + 1) {
      throw new Error(`${label} layout has global horizontal overflow: ${JSON.stringify(layout)}`);
    }
    if (!layout.canvasInsideBoard) {
      throw new Error(`${label} canvas is not framed inside the office board`);
    }
    if (
      !layout.sidebarVisible ||
      !layout.topbarVisible ||
      !layout.commandControlsVisible ||
      !layout.boardInFirstViewport
    ) {
      throw new Error(
        `${label} PC command layout does not keep navigation, loop controls, and office board in the first viewport: ${JSON.stringify(layout)}`,
      );
    }
    if (!layout.realtimeStatusVisible) {
      throw new Error(`${label} command header does not expose realtime status: ${JSON.stringify(layout)}`);
    }
    if (
      !layout.refreshVisible ||
      !layout.refreshInsideTopbar ||
      !layout.refreshHitTestable ||
      layout.refreshWidth < 36 ||
      layout.refreshHeight < 36
    ) {
      throw new Error(
        `${label} refresh control is hidden, clipped, occluded, or too small: ${JSON.stringify(layout)}`,
      );
    }
    if (width >= 1920) {
      const expectedBoardAspectRatio = 1672 / 941;
      const expectedGraphicWidth = Math.min(1784, layout.shellContentWidth);
      const expectedBoardWidth = Math.min(1280, expectedGraphicWidth - 494);
      const gapIsSupported = (gap) => typeof gap === "number" && gap >= 8 && gap <= 24;
      if (
        !layout.graphicCentered ||
        layout.graphicWidth > 1785 ||
        Math.abs(layout.graphicWidth - expectedGraphicWidth) > 2 ||
        layout.boardWidth > 1281 ||
        Math.abs(layout.boardWidth - expectedBoardWidth) > 2 ||
        Math.abs(layout.boardAspectRatio - expectedBoardAspectRatio) > 0.02 ||
        !gapIsSupported(layout.leftBoardGap) ||
        !gapIsSupported(layout.boardRightGap) ||
        !layout.bottomStartsInFirstViewport ||
        typeof layout.mainBottomGap !== "number" ||
        layout.mainBottomGap < 7 ||
        layout.mainBottomGap > 16 ||
        layout.topbarHeight > 90 ||
        layout.attentionHeight > 120 ||
        layout.commandControlsHeight > 120
      ) {
        throw new Error(
          `${label} widescreen layout is stretched, off-center, or clipped: ${JSON.stringify(layout)}`,
        );
      }
    }
    await page.screenshot({ path, fullPage: true });
    await page.close();
  };

  const captureMobileViewport = async () => {
    const page = await context.newPage();
    const browserErrors = [];
    page.on("console", (message) => {
      if (message.type() === "error") browserErrors.push(`console: ${message.text()}`);
    });
    page.on("pageerror", (error) => browserErrors.push(`page: ${error.message}`));
    page.on("response", (response) => {
      if (response.status() >= 400) {
        browserErrors.push(`response: ${response.status()} ${response.url()}`);
      }
    });
    await page.setViewportSize({ width: 390, height: 844 });
    const response = await page.goto(targetUrl, { waitUntil: "domcontentloaded" });
    if (!response?.ok() || page.url().includes("/admin/login")) {
      throw new Error(
        `mobile authenticated admin page failed to load: ${response?.status() ?? "no response"}`,
      );
    }
    await page.waitForFunction(() => {
      const status = document.querySelector("[data-realtime-status]");
      return status?.textContent?.trim();
    });
    await page.waitForTimeout(200);

    const layout = await page.evaluate(() => {
      const root = document.documentElement;
      const body = document.body;
      const sidebar = document.querySelector(".sidebar");
      const topbar = document.querySelector(".draft-topbar");
      const commandSummary = document.querySelector(".command-summary");
      const refresh = document.querySelector(".command-summary [data-refresh-dashboard]");
      const commandControls = document.querySelector(".command-controls");
      const mainGrid = document.querySelector(".draft-main-grid");
      const leftStack = document.querySelector(".left-stack");
      const rightStack = document.querySelector(".right-stack");
      const board = document.querySelector(".office-board");
      const campaign = document.querySelector(".right-stack #campaign");
      const events = document.querySelector(".right-stack #events");
      const rect = (element) => element?.getBoundingClientRect() ?? null;
      const visible = (element) => {
        const bounds = rect(element);
        if (!element || !bounds || bounds.width <= 0 || bounds.height <= 0) return false;
        const style = window.getComputedStyle(element);
        return style.display !== "none" && style.visibility !== "hidden";
      };
      const gridColumns = (element) =>
        element ? window.getComputedStyle(element).gridTemplateColumns.trim() : "";
      const overflow = (element) =>
        element ? window.getComputedStyle(element).overflowY : "";
      const refreshRect = rect(refresh);
      const topbarRect = rect(topbar);
      const loopControlHeights = [...document.querySelectorAll(".loop-control")].map(
        (control) => rect(control)?.height ?? 0,
      );
      return {
        documentWidth: root.scrollWidth,
        viewportWidth: root.clientWidth,
        bodyWidth: body.scrollWidth,
        sidebarVisible: visible(sidebar),
        topbarVisible: visible(topbar),
        commandSummaryVisible: visible(commandSummary),
        commandControlsVisible: visible(commandControls),
        boardVisible: visible(board),
        refreshVisible: visible(refresh),
        refreshInsideTopbar:
          Boolean(refreshRect && topbarRect) &&
          refreshRect.left >= topbarRect.left - 1 &&
          refreshRect.right <= topbarRect.right + 1 &&
          refreshRect.top >= topbarRect.top - 1 &&
          refreshRect.bottom <= topbarRect.bottom + 1,
        refreshWidth: refreshRect?.width ?? 0,
        refreshHeight: refreshRect?.height ?? 0,
        topbarColumns: gridColumns(topbar),
        mainGridColumns: gridColumns(mainGrid),
        leftStackColumns: gridColumns(leftStack),
        rightStackColumns: gridColumns(rightStack),
        campaignOverflow: overflow(campaign),
        eventsOverflow: overflow(events),
        loopControlHeights,
      };
    });
    if (
      layout.documentWidth > layout.viewportWidth + 1 ||
      layout.bodyWidth > layout.viewportWidth + 1
    ) {
      throw new Error(`mobile layout has global horizontal overflow: ${JSON.stringify(layout)}`);
    }
    if (
      !layout.sidebarVisible ||
      !layout.topbarVisible ||
      !layout.commandSummaryVisible ||
      !layout.commandControlsVisible ||
      !layout.boardVisible ||
      !layout.refreshVisible ||
      !layout.refreshInsideTopbar
    ) {
      throw new Error(`mobile command surfaces are hidden or clipped: ${JSON.stringify(layout)}`);
    }
    if (
      layout.refreshWidth < 36 ||
      layout.refreshHeight < 36 ||
      layout.mainGridColumns.split(" ").length !== 1 ||
      layout.leftStackColumns.split(" ").length !== 1 ||
      layout.rightStackColumns.split(" ").length !== 1 ||
      layout.campaignOverflow !== "visible" ||
      layout.eventsOverflow !== "visible" ||
      layout.loopControlHeights.some((height) => height < 38)
    ) {
      throw new Error(
        `mobile command layout does not collapse into one readable rail: ${JSON.stringify(layout)}`,
      );
    }
    if (browserErrors.length > 0) {
      throw new Error(`mobile browser errors:\n${browserErrors.join("\n")}`);
    }
    await page.screenshot({ path: mobileScreenshotPath, fullPage: true });
    await page.close();
  };

  await captureMobileViewport();
  await captureViewport({
    width: 1280,
    height: 800,
    path: compactScreenshotPath,
    label: "compact desktop",
  });
  await captureViewport({
    width: 1600,
    height: 1000,
    path: screenshotPath,
    label: "wide desktop",
  });
  await captureViewport({
    width: 1920,
    height: 1080,
    path: fullHdScreenshotPath,
    label: "full HD desktop",
  });
  await captureViewport({
    width: 2560,
    height: 1440,
    path: qhdScreenshotPath,
    label: "QHD desktop",
  });

  const navigationPage = await context.newPage();
  const navigationErrors = [];
  navigationPage.on("console", (message) => {
    if (message.type() === "error") navigationErrors.push(`console: ${message.text()}`);
  });
  navigationPage.on("pageerror", (error) => navigationErrors.push(`page: ${error.message}`));
  navigationPage.on("requestfailed", (request) => {
    const failure = request.failure()?.errorText ?? "failed";
    if (failure === "net::ERR_ABORTED" && request.url().startsWith(baseUrl)) return;
    navigationErrors.push(`request: ${request.url()} ${failure}`);
  });
  navigationPage.on("response", (response) => {
    if (response.status() >= 400) {
      navigationErrors.push(`response: ${response.status()} ${response.url()}`);
    }
  });
  navigationPage.on("request", (request) => {
    const url = request.url();
    if (
      !url.startsWith(baseUrl) &&
      !url.startsWith("data:") &&
      !url.startsWith("blob:")
    ) {
      navigationErrors.push(`external request: ${url}`);
    }
  });
  await navigationPage.setViewportSize({ width: 1280, height: 800 });

  const planningResponse = await navigationPage.goto(`${baseUrl}/admin`, {
    waitUntil: "networkidle",
  });
  if (!planningResponse?.ok()) {
    throw new Error(`desktop planning admin returned ${planningResponse?.status() ?? "no response"}`);
  }
  const graphicEntry = navigationPage.getByRole("link", {
    name: "Graphic dashboard",
    exact: true,
  });
  if (!(await graphicEntry.isVisible())) {
    throw new Error("planning admin does not expose a visible graphic dashboard link");
  }
  await graphicEntry.focus();
  if ((await navigationPage.evaluate(() => document.activeElement?.getAttribute("href"))) !== "/admin/akra") {
    throw new Error("graphic dashboard entry link is not keyboard focusable");
  }
  await Promise.all([
    navigationPage.waitForURL(`${baseUrl}/admin/akra`),
    navigationPage.keyboard.press("Enter"),
  ]);
  await navigationPage.waitForLoadState("domcontentloaded");

  const expectedDashboardLinks = [
    "/admin/akra",
    "/admin/akra#pool",
    "/admin/akra#agents",
    "/admin/akra/directions",
    "/admin/akra/tasks",
    "/admin/akra#pipeline",
    "/admin/akra#events",
    "/admin/akra/metrics#metrics",
    "/admin/akra/metrics#system",
    "/admin",
    "/admin/controls",
  ];
  const dashboardLinks = navigationPage.locator(".graphic-nav a");
  const dashboardLinkLayout = await dashboardLinks.evaluateAll((links) =>
    links.map((link) => {
      const rect = link.getBoundingClientRect();
      const style = window.getComputedStyle(link);
      return {
        href: link.getAttribute("href"),
        visible:
          style.display !== "none" &&
          style.visibility !== "hidden" &&
          rect.width > 0 &&
          rect.height >= 36,
        insideViewport: rect.left >= -1 && rect.right <= window.innerWidth + 1,
      };
    }),
  );
  if (
    JSON.stringify(dashboardLinkLayout.map((link) => link.href)) !==
      JSON.stringify(expectedDashboardLinks) ||
    dashboardLinkLayout.some((link) => !link.visible || !link.insideViewport)
  ) {
    throw new Error(`desktop dashboard navigation is missing or clipped: ${JSON.stringify(dashboardLinkLayout)}`);
  }

  await navigationPage.locator("body").click({ position: { x: 1, y: 1 } });
  const keyboardTrail = [];
  for (let index = 0; index < expectedDashboardLinks.length; index += 1) {
    await navigationPage.keyboard.press("Tab");
    keyboardTrail.push(
      await navigationPage.evaluate(() => document.activeElement?.getAttribute("href") ?? null),
    );
  }
  if (
    JSON.stringify(keyboardTrail) !== JSON.stringify(expectedDashboardLinks)
  ) {
    throw new Error(`dashboard keyboard order is incomplete: ${JSON.stringify(keyboardTrail)}`);
  }

  for (const path of [
    "/admin/akra/directions",
    "/admin/akra/tasks",
    "/admin/akra/metrics",
  ]) {
    const response = await navigationPage.goto(`${baseUrl}${path}`, {
      waitUntil: "networkidle",
    });
    if (!response?.ok()) {
      throw new Error(`desktop admin navigation failed for ${path}: ${response?.status() ?? "no response"}`);
    }
    const widths = await navigationPage.evaluate(() => ({
      document: document.documentElement.scrollWidth,
      viewport: document.documentElement.clientWidth,
      body: document.body.scrollWidth,
    }));
    if (widths.document > widths.viewport + 1 || widths.body > widths.viewport + 1) {
      throw new Error(`desktop admin route overflows at ${path}: ${JSON.stringify(widths)}`);
    }
  }
  if (navigationErrors.length > 0) {
    throw new Error(`admin navigation browser errors:\n${navigationErrors.join("\n")}`);
  }
  await navigationPage.close();
} finally {
  await browser.close();
}
