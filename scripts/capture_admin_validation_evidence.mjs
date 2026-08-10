import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";

import { chromium } from "@playwright/test";

const options = Object.fromEntries(
  process.argv.slice(2).map((argument) => {
    const [key, ...value] = argument.split("=");
    return [key.replace(/^--/, ""), value.join("=")];
  }),
);

const browserPath = options.browser;
const targetUrl = options.url;
const wideScreenshotPath = options["wide-screenshot"];
const narrowScreenshotPath = options["narrow-screenshot"];
const metricsPath = options.metrics;
const commit = options.commit;
const observationMillis = Number.parseInt(options["observation-ms"] ?? "15000", 10);
const token = process.env.AKRA_ADMIN_VISUAL_TOKEN;

if (
  !browserPath ||
  !targetUrl ||
  !wideScreenshotPath ||
  !narrowScreenshotPath ||
  !metricsPath ||
  !commit ||
  !token ||
  !Number.isInteger(observationMillis) ||
  observationMillis < 1000 ||
  observationMillis > 60000
) {
  throw new Error(
    "browser, url, wide-screenshot, narrow-screenshot, metrics, commit, " +
      "AKRA_ADMIN_VISUAL_TOKEN, and an observation-ms value from 1000 to 60000 are required",
  );
}

const target = new URL(targetUrl);
const baseUrl = `${target.protocol}//${target.host}`;
const outputPaths = [wideScreenshotPath, narrowScreenshotPath, metricsPath];
await Promise.all(outputPaths.map((outputPath) => mkdir(path.dirname(outputPath), { recursive: true })));

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

const metricValue = (metrics, name) =>
  metrics.metrics.find((entry) => entry.name === name)?.value ?? null;

const metricDelta = (before, after, name, scale = 1) => {
  const beforeValue = metricValue(before, name);
  const afterValue = metricValue(after, name);
  return beforeValue === null || afterValue === null
    ? null
    : Number(((afterValue - beforeValue) * scale).toFixed(3));
};

const browserErrorsFor = (page) => {
  const errors = [];
  page.on("console", (message) => {
    if (message.type() === "error" || message.type() === "warning") {
      errors.push(`${message.type()}: ${message.text()}`);
    }
  });
  page.on("pageerror", (error) => errors.push(`page: ${error.message}`));
  page.on("response", (response) => {
    if (response.status() >= 400) errors.push(`response: ${response.status()} ${response.url()}`);
  });
  return errors;
};

const login = async (context) => {
  const page = await context.newPage();
  const response = await page.goto(`${baseUrl}/admin/login`, { waitUntil: "networkidle" });
  if (!response?.ok()) {
    throw new Error(`admin login page returned ${response?.status() ?? "no response"}`);
  }
  await page.locator('input[name="token"]').fill(token);
  const submitted = page.waitForResponse(
    (candidate) =>
      candidate.url() === `${baseUrl}/admin/login` && candidate.request().method() === "POST",
  );
  await page.locator('button[type="submit"]').click();
  const result = await submitted;
  if (result.status() !== 303) throw new Error(`admin login returned ${result.status()}`);
  await page.waitForURL(`${baseUrl}/admin`);
  await page.close();
};

const waitForEvidence = async (page) => {
  await page.waitForFunction(() => {
    const evidence = document.querySelector("[data-validation-evidence]");
    const state = evidence?.getAttribute("data-validation-evidence-state");
    return Boolean(state && state !== "unavailable");
  });
  await page.locator("[data-validation-evidence]").scrollIntoViewIfNeeded();
};

const openEvidenceDrawer = async (page) => {
  await page.locator("[data-evidence-detail-trigger]").click();
  await page.waitForFunction(() => {
    const drawer = document.querySelector("[data-detail-drawer]");
    return (
      drawer &&
      !drawer.hidden &&
      drawer.getAttribute("data-detail-mode") === "evidence" &&
      drawer.querySelector("[data-evidence-history-row], .evidence-history-row")
    );
  });
};

const readApiEvidence = async (page) =>
  page.evaluate(async () => {
    const readJson = async (url) => {
      const response = await fetch(url, { headers: { Accept: "application/json" } });
      const text = await response.text();
      if (!response.ok) throw new Error(`${url} returned ${response.status}`);
      return {
        bytes: new TextEncoder().encode(text).byteLength,
        value: JSON.parse(text),
      };
    };
    const dashboard = await readJson("/api/admin/akra/dashboard");
    const detail = await readJson("/api/admin/akra/pr-validation/evidence?limit=10");
    return {
      dashboardBytes: dashboard.bytes,
      evidenceDetailBytes: detail.bytes,
      latestStatus: detail.value.latest?.summary?.status ?? null,
      latestGeneratedAt: detail.value.latest?.summary?.generatedAt ?? null,
      historyRows: detail.value.history?.length ?? 0,
      trendMetrics: detail.value.trends?.comparisons?.map((entry) => entry.metric) ?? [],
      warningKinds: detail.value.warnings?.map((entry) => entry.kind) ?? [],
      collection: detail.value.collection ?? null,
      revision: detail.value.revision ?? null,
    };
  });

const readDomMetrics = async (page) =>
  page.evaluate(() => {
    const root = document.documentElement;
    const body = document.body;
    const evidence = document.querySelector("[data-validation-evidence]");
    const drawer = document.querySelector("[data-detail-drawer]");
    const overflowElements = [...document.querySelectorAll("body *")]
      .map((element) => {
        const bounds = element.getBoundingClientRect();
        return {
          tag: element.tagName.toLowerCase(),
          id: element.id || null,
          classes: [...element.classList].slice(0, 4),
          left: Number(bounds.left.toFixed(2)),
          right: Number(bounds.right.toFixed(2)),
          width: Number(bounds.width.toFixed(2)),
        };
      })
      .filter(
        (element) =>
          element.width > 0 &&
          (element.left < -0.5 || element.right > root.clientWidth + 0.5),
      )
      .slice(0, 20);
    return {
      document: {
        innerWidth: window.innerWidth,
        rootClientWidth: root.clientWidth,
        rootOffsetWidth: root.offsetWidth,
        rootScrollWidth: root.scrollWidth,
        bodyClientWidth: body.clientWidth,
        bodyOffsetWidth: body.offsetWidth,
        bodyScrollWidth: body.scrollWidth,
        bodyBounds: {
          left: Number(body.getBoundingClientRect().left.toFixed(2)),
          right: Number(body.getBoundingClientRect().right.toFixed(2)),
          width: Number(body.getBoundingClientRect().width.toFixed(2)),
        },
        bodyOverflowX: window.getComputedStyle(body).overflowX,
        rootOverflowX: window.getComputedStyle(root).overflowX,
      },
      nodes: document.getElementsByTagName("*").length,
      scrollHeight: root.scrollHeight,
      horizontalOverflowPixels: Math.max(0, root.scrollWidth - root.clientWidth),
      evidenceState: evidence?.getAttribute("data-validation-evidence-state") ?? null,
      drawerMode: drawer?.getAttribute("data-detail-mode") ?? null,
      drawerVisible: Boolean(drawer && !drawer.hidden),
      overflowElements,
      historyRows: drawer?.querySelectorAll(".evidence-history-row").length ?? 0,
      metricCards: [...document.querySelectorAll("[data-evidence-metric]")].map((card) => ({
        metric: card.getAttribute("data-evidence-metric"),
        text: card.textContent?.replace(/\s+/g, " ").trim() ?? "",
      })),
      realtime: document.querySelector("[data-realtime-status]")?.textContent?.trim() ?? null,
    };
  });

try {
  const context = await browser.newContext();
  await login(context);

  const widePage = await context.newPage();
  const wideErrors = browserErrorsFor(widePage);
  await widePage.setViewportSize({ width: 1600, height: 1000 });
  const response = await widePage.goto(targetUrl, { waitUntil: "domcontentloaded" });
  if (!response?.ok() || widePage.url().includes("/admin/login")) {
    throw new Error(`authenticated Admin page returned ${response?.status() ?? "no response"}`);
  }
  await waitForEvidence(widePage);
  await openEvidenceDrawer(widePage);

  const cdp = await context.newCDPSession(widePage);
  await cdp.send("Performance.enable");
  const api = await readApiEvidence(widePage);
  const beforePerformance = await cdp.send("Performance.getMetrics");
  const beforeDom = await readDomMetrics(widePage);
  await widePage.waitForTimeout(observationMillis);
  const afterPerformance = await cdp.send("Performance.getMetrics");
  const afterDom = await readDomMetrics(widePage);
  if (api.latestStatus !== "ready") {
    throw new Error(`approval evidence is not ready: ${api.latestStatus ?? "missing"}`);
  }
  if (
    beforeDom.horizontalOverflowPixels !== 0 ||
    afterDom.horizontalOverflowPixels !== 0 ||
    beforeDom.nodes !== afterDom.nodes ||
    beforeDom.scrollHeight !== afterDom.scrollHeight
  ) {
    throw new Error(
      `wide evidence surface is not stable: ${JSON.stringify({ beforeDom, afterDom })}`,
    );
  }
  if (wideErrors.length > 0) throw new Error(`wide browser errors:\n${wideErrors.join("\n")}`);
  await widePage.screenshot({ path: wideScreenshotPath, fullPage: true });
  await widePage.close();

  const narrowPage = await context.newPage();
  const narrowErrors = browserErrorsFor(narrowPage);
  await narrowPage.setViewportSize({ width: 390, height: 844 });
  const narrowResponse = await narrowPage.goto(targetUrl, { waitUntil: "domcontentloaded" });
  if (!narrowResponse?.ok() || narrowPage.url().includes("/admin/login")) {
    throw new Error(`narrow Admin page returned ${narrowResponse?.status() ?? "no response"}`);
  }
  await waitForEvidence(narrowPage);
  const narrowBeforeDrawer = await readDomMetrics(narrowPage);
  await openEvidenceDrawer(narrowPage);
  const narrowDom = await readDomMetrics(narrowPage);
  if (
    narrowBeforeDrawer.horizontalOverflowPixels !== 0 ||
    narrowDom.horizontalOverflowPixels !== 0
  ) {
    throw new Error(
      `narrow evidence surface has horizontal overflow: ${JSON.stringify({ narrowBeforeDrawer, narrowDom })}`,
    );
  }
  if (narrowErrors.length > 0) throw new Error(`narrow browser errors:\n${narrowErrors.join("\n")}`);
  await narrowPage.screenshot({ path: narrowScreenshotPath, fullPage: true });
  await narrowPage.close();

  const metrics = {
    schemaVersion: 1,
    capturedAt: new Date().toISOString(),
    commit,
    target: "/admin/akra",
    composition: "production",
    observationMillis,
    api,
    wide: {
      viewport: { width: 1600, height: 1000 },
      before: beforeDom,
      after: afterDom,
      nodeDelta: afterDom.nodes - beforeDom.nodes,
      scrollHeightDelta: afterDom.scrollHeight - beforeDom.scrollHeight,
      taskDurationDeltaMillis: metricDelta(
        beforePerformance,
        afterPerformance,
        "TaskDuration",
        1000,
      ),
      scriptDurationDeltaMillis: metricDelta(
        beforePerformance,
        afterPerformance,
        "ScriptDuration",
        1000,
      ),
      jsHeapUsedBeforeBytes: metricValue(beforePerformance, "JSHeapUsedSize"),
      jsHeapUsedAfterBytes: metricValue(afterPerformance, "JSHeapUsedSize"),
      browserWarningOrErrorCount: wideErrors.length,
    },
    narrow: {
      viewport: { width: 390, height: 844 },
      beforeDrawer: narrowBeforeDrawer,
      dom: narrowDom,
      browserWarningOrErrorCount: narrowErrors.length,
    },
    screenshots: {
      wide: path.basename(wideScreenshotPath),
      narrow: path.basename(narrowScreenshotPath),
    },
  };
  await writeFile(metricsPath, `${JSON.stringify(metrics, null, 2)}\n`, "utf8");
  process.stdout.write(`${JSON.stringify(metrics, null, 2)}\n`);
  await context.close();
} finally {
  await browser.close();
}
