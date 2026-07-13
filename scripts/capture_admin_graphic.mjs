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
const token = process.env.AKRA_ADMIN_VISUAL_TOKEN;

if (!browserPath || !targetUrl || !screenshotPath || !mobileScreenshotPath || !token) {
  throw new Error(
    "browser, url, screenshot, mobile-screenshot, and AKRA_ADMIN_VISUAL_TOKEN are required",
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

    const response = await page.goto(targetUrl, { waitUntil: "networkidle" });
    if (!response?.ok() || page.url().includes("/admin/login")) {
      throw new Error(
        `${label} authenticated admin page failed to load: ${response?.status() ?? "no response"}`,
      );
    }

    const fontState = await page.evaluate(async () => {
      await document.fonts.ready;
      return {
        regular: document.fonts.check("12px Galmuri11", "게임발전국"),
        bold: document.fonts.check("700 12px Galmuri11", "운영 알림"),
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
      const png = await canvas.screenshot({ type: "png" });
      const imageUrl = `data:image/png;base64,${png.toString("base64")}`;
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
    if (firstFrame.checksum !== secondFrame.checksum) {
      throw new Error(`${label} static WebGL canvas changed without a typed transition`);
    }
    for (const scene of [firstScene, secondScene]) {
      if (!scene || scene.packetCount !== 0 || scene.semanticMotionCount !== 0) {
        throw new Error(`${label} scene reported unowned motion: ${JSON.stringify(scene)}`);
      }
    }
    if (firstScene.renderCount !== secondScene.renderCount) {
      throw new Error(`${label} static scene rendered continuously without a state change`);
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
    if (JSON.stringify(actorParity.dom) !== JSON.stringify(actorParity.canvasActors.map(({ x, y, ...actor }) => actor))) {
      throw new Error(`${label} DOM/canvas actor identity mismatch: ${JSON.stringify(actorParity)}`);
    }
    await analysisPage.close();

    await page.locator(".office-board").scrollIntoViewIfNeeded();
    const layout = await page.evaluate(() => {
      const root = document.documentElement;
      const body = document.body;
      const canvas = document.querySelector("#pixi-diorama canvas");
      const board = document.querySelector(".office-board");
      const hud = document.querySelector(".stage-hud");
      const refresh = document.querySelector(".stage-hud [data-refresh-dashboard]");
      const canvasRect = canvas?.getBoundingClientRect();
      const boardRect = board?.getBoundingClientRect();
      const hudRect = hud?.getBoundingClientRect();
      const refreshRect = refresh?.getBoundingClientRect();
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
        hudVisible: isVisible(hud, hudRect),
        hudInsideBoard: isInside(hudRect, boardRect),
        hudHitTestable: centerHitIsInside(hud, hudRect),
        refreshVisible: isVisible(refresh, refreshRect),
        refreshInsideBoard: isInside(refreshRect, boardRect),
        refreshHitTestable:
          centerHitIsInside(refresh, refreshRect) &&
          window.getComputedStyle(refresh).pointerEvents !== "none" &&
          !refresh.disabled,
        refreshWidth: refreshRect?.width ?? 0,
        refreshHeight: refreshRect?.height ?? 0,
        boardRect: rectValue(boardRect),
        hudRect: rectValue(hudRect),
        refreshRect: rectValue(refreshRect),
      };
    });
    if (layout.documentWidth > layout.viewportWidth + 1 || layout.bodyWidth > layout.viewportWidth + 1) {
      throw new Error(`${label} layout has global horizontal overflow: ${JSON.stringify(layout)}`);
    }
    if (!layout.canvasInsideBoard) {
      throw new Error(`${label} canvas is not framed inside the office board`);
    }
    if (!layout.hudVisible || !layout.hudInsideBoard || !layout.hudHitTestable) {
      throw new Error(`${label} mission HUD is hidden, clipped, or occluded: ${JSON.stringify(layout)}`);
    }
    if (
      !layout.refreshVisible ||
      !layout.refreshInsideBoard ||
      !layout.refreshHitTestable ||
      layout.refreshWidth < 40 ||
      layout.refreshHeight < 40
    ) {
      throw new Error(
        `${label} refresh control is hidden, clipped, occluded, or too small: ${JSON.stringify(layout)}`,
      );
    }
    await page.screenshot({ path, fullPage: true });
    await page.close();
  };

  await captureViewport({
    width: 1600,
    height: 1000,
    path: screenshotPath,
    label: "desktop",
  });
  await captureViewport({
    width: 390,
    height: 844,
    path: mobileScreenshotPath,
    label: "mobile",
  });

  const navigationPage = await context.newPage();
  const navigationErrors = [];
  navigationPage.on("console", (message) => {
    if (message.type() === "error") navigationErrors.push(`console: ${message.text()}`);
  });
  navigationPage.on("pageerror", (error) => navigationErrors.push(`page: ${error.message}`));
  navigationPage.on("requestfailed", (request) => {
    navigationErrors.push(`request: ${request.url()} ${request.failure()?.errorText ?? "failed"}`);
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
  await navigationPage.setViewportSize({ width: 390, height: 844 });

  const planningResponse = await navigationPage.goto(`${baseUrl}/admin`, {
    waitUntil: "networkidle",
  });
  if (!planningResponse?.ok()) {
    throw new Error(`mobile planning admin returned ${planningResponse?.status() ?? "no response"}`);
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
  await navigationPage.waitForLoadState("networkidle");

  const expectedDashboardLinks = [
    "/admin/akra",
    "/admin/akra/directions",
    "/admin/akra/tasks",
    "/admin/akra/metrics",
    "/admin",
    "/admin/controls",
  ];
  const dashboardLinks = navigationPage.locator(".draft-nav a");
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
          rect.height >= 40,
        insideViewport: rect.left >= -1 && rect.right <= window.innerWidth + 1,
      };
    }),
  );
  if (
    JSON.stringify(dashboardLinkLayout.map((link) => link.href)) !==
      JSON.stringify(expectedDashboardLinks) ||
    dashboardLinkLayout.some((link) => !link.visible || !link.insideViewport)
  ) {
    throw new Error(`mobile dashboard navigation is missing or clipped: ${JSON.stringify(dashboardLinkLayout)}`);
  }

  await navigationPage.locator("body").click({ position: { x: 1, y: 1 } });
  const keyboardTrail = [];
  for (let index = 0; index < expectedDashboardLinks.length + 1; index += 1) {
    await navigationPage.keyboard.press("Tab");
    keyboardTrail.push(
      await navigationPage.evaluate(() => document.activeElement?.getAttribute("href") ?? null),
    );
  }
  if (
    JSON.stringify(keyboardTrail.slice(1)) !== JSON.stringify(expectedDashboardLinks)
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
      throw new Error(`mobile admin navigation failed for ${path}: ${response?.status() ?? "no response"}`);
    }
    const widths = await navigationPage.evaluate(() => ({
      document: document.documentElement.scrollWidth,
      viewport: document.documentElement.clientWidth,
      body: document.body.scrollWidth,
    }));
    if (widths.document > widths.viewport + 1 || widths.body > widths.viewport + 1) {
      throw new Error(`mobile admin route overflows at ${path}: ${JSON.stringify(widths)}`);
    }
  }
  if (navigationErrors.length > 0) {
    throw new Error(`admin navigation browser errors:\n${navigationErrors.join("\n")}`);
  }
  await navigationPage.close();
} finally {
  await browser.close();
}
