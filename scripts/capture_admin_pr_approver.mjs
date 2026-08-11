/**
 * Browser contract for the Admin PR approver diorama.
 *
 * Prerequisites:
 *   1. Build the game bundle (`npm --prefix assets/admin/game run build`).
 *   2. Start an isolated Admin server with the deterministic harness enabled, for example:
 *      `AKRA_ADMIN_TOKEN=visual-token cargo run --bin akra-admin -- --debug-harness --port 18448`
 *   3. Point `--url` at that server's `/admin/akra` page and provide the same token through
 *      `AKRA_ADMIN_VISUAL_TOKEN`. The script mutates only the process-local debug harness.
 *
 * Usage:
 *   AKRA_ADMIN_VISUAL_TOKEN=visual-token node scripts/capture_admin_pr_approver.mjs \
 *     --browser=/path/to/chromium \
 *     --url=http://127.0.0.1:18448/admin/akra \
 *     --output-dir=docs/validation/artifacts/admin-pr-approver
 *
 * Outputs:
 *   reviewing.png, failure.png, success.png, and evidence.json in `--output-dir`.
 */

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
const outputDirOption = options["output-dir"];
const token = process.env.AKRA_ADMIN_VISUAL_TOKEN;
const timeoutMs = Number(options["timeout-ms"] || "15000");

if (!browserPath || !targetUrl || !outputDirOption || !token) {
  throw new Error(
    "browser, url, output-dir, and AKRA_ADMIN_VISUAL_TOKEN are required",
  );
}
if (!Number.isFinite(timeoutMs) || timeoutMs < 1_000) {
  throw new Error("timeout-ms must be a finite number of at least 1000");
}

const target = new URL(targetUrl);
const baseUrl = `${target.protocol}//${target.host}`;
const outputDir = path.resolve(outputDirOption);
const screenshotPaths = {
  reviewing: path.join(outputDir, "reviewing.png"),
  failure: path.join(outputDir, "failure.png"),
  success: path.join(outputDir, "success.png"),
};

const POST_MERGE_SUCCESS = "post_merge_success";
const CHECK_FAILURE_RECOVERY = "check_failure_recovery";

const assert = (condition, message, detail) => {
  if (condition) return;
  const suffix = detail === undefined ? "" : `: ${JSON.stringify(detail)}`;
  throw new Error(`${message}${suffix}`);
};

const delay = (milliseconds) =>
  new Promise((resolve) => setTimeout(resolve, milliseconds));

const countSnapshot = (inspection) => ({
  actorCount: inspection.actorCount,
  standbyCount: inspection.standbyCount,
  characterCount: inspection.characterCount,
});

const compactInspection = (inspection) => ({
  capturedAt: new Date().toISOString(),
  ready: inspection.ready,
  renderCount: inspection.renderCount,
  actorCount: inspection.actorCount,
  standbyCount: inspection.standbyCount,
  characterCount: inspection.characterCount,
  validation: {
    recordKey: inspection.validation.recordKey,
    phase: inspection.validation.phase,
    workerLeaseActive: inspection.validation.workerLeaseActive,
    approver: { ...inspection.validation.approver },
  },
});

const attachBrowserErrorCollectors = (page, label, bucket) => {
  page.on("console", (message) => {
    if (message.type() === "error") {
      bucket.push({
        label,
        kind: "console",
        message: message.text(),
        location: message.location(),
      });
    }
  });
  page.on("pageerror", (error) => {
    bucket.push({ label, kind: "page", message: error.message });
  });
  page.on("response", (response) => {
    if (response.status() >= 400) {
      bucket.push({
        label,
        kind: "response",
        message: `${response.status()} ${response.url()}`,
      });
    }
  });
};

const authenticate = async (context) => {
  const loginPage = await context.newPage();
  const loginDocument = await loginPage.goto(`${baseUrl}/admin/login`, {
    waitUntil: "networkidle",
  });
  if (!loginDocument?.ok()) {
    throw new Error(
      `admin browser login page returned ${loginDocument?.status() ?? "no response"}`,
    );
  }

  await loginPage.locator('input[name="token"]').fill(token);
  const loginResponsePromise = loginPage.waitForResponse(
    (response) =>
      response.url() === `${baseUrl}/admin/login`
      && response.request().method() === "POST",
  );
  await loginPage.locator('button[type="submit"]').click();
  const loginResponse = await loginResponsePromise;
  if (loginResponse.status() !== 303) {
    throw new Error(`admin browser login returned ${loginResponse.status()}, expected 303`);
  }
  await loginPage.waitForURL((url) => url.origin === baseUrl && url.pathname === "/admin");
  await loginPage.close();

  const session = (await context.cookies(baseUrl)).find(
    (cookie) => cookie.name === "akra_admin_session",
  );
  if (!session || !session.httpOnly || session.sameSite !== "Strict") {
    throw new Error("admin browser login did not establish the strict HttpOnly session cookie");
  }
};

const waitForDiorama = async (page) => {
  await page.waitForFunction(
    () => {
      const root = document.querySelector("[data-admin-graphic]");
      const container = document.querySelector("#pixi-diorama");
      const canvas = container?.querySelector("canvas");
      return (
        root?.dataset.debugHarnessEnabled === "true"
        && container?.getAttribute("data-akra-diorama-mounted") === "true"
        && canvas instanceof HTMLCanvasElement
        && canvas.width > 64
        && canvas.height > 64
        && window.AkraAdminGame?.inspectScene?.()?.ready === true
      );
    },
    undefined,
    { timeout: timeoutMs },
  );
};

const openDashboard = async (page, reload = false) => {
  const response = reload
    ? await page.reload({ waitUntil: "domcontentloaded" })
    : await page.goto(targetUrl, { waitUntil: "domcontentloaded" });
  if (!response?.ok() || page.url().includes("/admin/login")) {
    throw new Error(
      `authenticated Admin dashboard failed to load: ${response?.status() ?? "no response"}`,
    );
  }
  await waitForDiorama(page);
};

const fetchDashboard = async (page) =>
  page.evaluate(async () => {
    const response = await fetch("/api/admin/akra/dashboard", {
      headers: { Accept: "application/json" },
    });
    if (!response.ok) throw new Error(`dashboard ${response.status}`);
    return response.json();
  });

const applyDashboard = async (page, dashboard) => {
  await page.evaluate((snapshot) => {
    if (typeof window.AkraAdminGame?.applyDashboard !== "function") {
      throw new Error("AkraAdminGame.applyDashboard is unavailable");
    }
    window.AkraAdminGame.applyDashboard(snapshot);
  }, dashboard);

  const expected = dashboard.scene?.validation?.approver;
  await page.waitForFunction(
    ({ state, qualifier, transitionKey }) => {
      const approver = window.AkraAdminGame?.inspectScene?.()?.validation?.approver;
      return (
        approver?.state === state
        && approver?.qualifier === qualifier
        && approver?.transitionKey === transitionKey
      );
    },
    {
      state: expected?.state ?? "idle",
      qualifier: expected?.qualifier ?? "none",
      transitionKey: expected?.transitionKey ?? "idle",
    },
    { timeout: timeoutMs },
  );
};

const syncDashboard = async (page) => {
  const dashboard = await fetchDashboard(page);
  await applyDashboard(page, dashboard);
  return dashboard;
};

const publicDebugCommand = async (page, action, scenario = null) => {
  const result = await page.evaluate(
    async ({ action: requestedAction, scenario: requestedScenario }) => {
      const csrfToken = document.querySelector('meta[name="csrf-token"]')?.content || "";
      const response = await fetch("/api/admin/akra/debug-harness", {
        method: "POST",
        headers: {
          Accept: "application/json",
          "Content-Type": "application/json",
          "X-CSRF-Token": csrfToken,
        },
        body: JSON.stringify({
          action: requestedAction,
          scenario: requestedScenario,
        }),
      });
      const body = await response.json().catch(() => null);
      return { ok: response.ok, status: response.status, body };
    },
    { action, scenario },
  );
  assert(
    result.ok,
    `public debug harness ${action} failed with ${result.status}`,
    result.body,
  );
  return result.body;
};

const selectScenario = async (page, scenario) => {
  const harness = await publicDebugCommand(page, "scenario", scenario);
  assert(harness?.enabled === true, "Admin server did not enable the debug harness", harness);
  assert(harness.scenarioKey === scenario, "debug scenario selection did not stick", harness);
  assert(harness.stageKey === "ready", "selected scenario did not reset to ready", harness);
  await openDashboard(page, true);
  const dashboard = await syncDashboard(page);
  assert(dashboard.debugHarness?.scenarioKey === scenario, "dashboard scenario mismatch", dashboard.debugHarness);
  assert(dashboard.debugHarness?.stageKey === "ready", "dashboard did not reload at ready", dashboard.debugHarness);
  return { harness, dashboard };
};

const stepScenarioThroughUi = async (page, expectedStage) => {
  const stepButton = page.locator('[data-debug-command="step"]');
  await stepButton.waitFor({ state: "visible", timeout: timeoutMs });
  await page.waitForFunction(
    () => !document.querySelector('[data-debug-command="step"]')?.disabled,
    undefined,
    { timeout: timeoutMs },
  );
  const responsePromise = page.waitForResponse(
    (response) =>
      new URL(response.url()).pathname === "/api/admin/akra/debug-harness"
      && response.request().method() === "POST",
    { timeout: timeoutMs },
  );
  await stepButton.click();
  const response = await responsePromise;
  assert(response.ok(), `debug step UI returned ${response.status()}`);
  const harness = await response.json();
  assert(harness.stageKey === expectedStage, `debug step did not reach ${expectedStage}`, harness);

  await page.waitForFunction(
    ({ stage, revision }) => {
      const root = document.querySelector("[data-admin-graphic]");
      const panel = document.querySelector("[data-debug-harness]");
      return (
        panel?.dataset.debugStage === stage
        && Number(root?.dataset.debugHarnessRevision || 0) >= revision
      );
    },
    { stage: expectedStage, revision: Number(harness.revision || 0) },
    { timeout: timeoutMs },
  );

  const dashboard = await syncDashboard(page);
  assert(dashboard.debugHarness?.stageKey === expectedStage, "dashboard stage lagged UI command", {
    expectedStage,
    actual: dashboard.debugHarness,
  });
  return { harness, dashboard };
};

const inspect = async (page) => {
  const inspection = await page.evaluate(() => window.AkraAdminGame?.inspectScene?.());
  assert(inspection?.ready === true, "diorama inspection is not ready", inspection);
  return inspection;
};

const assertWorkerParity = (inspection, dashboard, label) => {
  const expectedActorCount = dashboard.scene?.actors?.length ?? 0;
  const expectedStandbyCount = dashboard.scene?.standbyCharacters?.length ?? 0;
  assert(inspection.actorCount === expectedActorCount, `${label} actor count drift`, {
    inspection: inspection.actorCount,
    dashboard: expectedActorCount,
  });
  assert(inspection.standbyCount === expectedStandbyCount, `${label} standby count drift`, {
    inspection: inspection.standbyCount,
    dashboard: expectedStandbyCount,
  });
  assert(
    inspection.characterCount === inspection.actorCount + inspection.standbyCount,
    `${label} approver was incorrectly counted as a worker character`,
    countSnapshot(inspection),
  );
};

const assertApprover = (
  inspection,
  dashboard,
  { state, qualifier, label, reducedMotion = false },
) => {
  const approver = inspection.validation.approver;
  assert(approver.state === state, `${label} approver state mismatch`, approver);
  if (qualifier !== undefined) {
    assert(approver.qualifier === qualifier, `${label} approver qualifier mismatch`, approver);
  }
  assert(approver.visible === true, `${label} approver is not visible`, approver);
  assert(approver.displayWidth > 24 && approver.displayHeight > 48, `${label} approver is not legible`, approver);
  assert(approver.reducedMotion === reducedMotion, `${label} reduced-motion projection mismatch`, approver);
  assertWorkerParity(inspection, dashboard, label);

  const projected = dashboard.scene.validation.approver;
  assert(approver.recordKey === projected.recordKey, `${label} scene/inspection recordKey mismatch`, {
    inspection: approver.recordKey,
    projection: projected.recordKey,
  });
  assert(
    inspection.validation.recordKey === dashboard.scene.validation.recordKey,
    `${label} validation station recordKey mismatch`,
    inspection.validation,
  );
  if (approver.recordKey !== null) {
    const record = dashboard.validation?.records?.find(
      (candidate) => candidate.recordKey === approver.recordKey,
    );
    assert(Boolean(record), `${label} approver recordKey does not resolve in validation rail`, {
      recordKey: approver.recordKey,
      available: dashboard.validation?.records?.map((candidate) => candidate.recordKey),
    });
    assert(
      dashboard.scene.validation.recordKey === approver.recordKey,
      `${label} station and approver selected different records`,
      dashboard.scene.validation,
    );
  }
};

const assertStableCounts = (samples, label) => {
  const expected = countSnapshot(samples[0]);
  for (const sample of samples.slice(1)) {
    assert(
      JSON.stringify(countSnapshot(sample)) === JSON.stringify(expected),
      `${label} animation changed worker counts`,
      samples.map(countSnapshot),
    );
  }
};

const sampleAnimation = async (page, count, intervalMs) => {
  const samples = [];
  for (let index = 0; index < count; index += 1) {
    samples.push(await inspect(page));
    if (index + 1 < count) await delay(intervalMs);
  }
  return samples;
};

const waitForApproverSettled = async (page, state) => {
  await page.waitForFunction(
    (expectedState) => {
      const approver = window.AkraAdminGame?.inspectScene?.()?.validation?.approver;
      return approver?.state === expectedState && approver?.settled === true;
    },
    state,
    { timeout: timeoutMs },
  );
  return inspect(page);
};

const assertTerminalHold = async (page, dashboard, state, label) => {
  const first = await waitForApproverSettled(page, state);
  await delay(700);
  const second = await inspect(page);
  for (const inspection of [first, second]) {
    assertApprover(inspection, dashboard, { state, label });
    assert(inspection.validation.approver.settled === true, `${label} did not settle`, inspection.validation.approver);
  }
  assert(
    first.validation.approver.sourceFrameIndex === second.validation.approver.sourceFrameIndex
      && first.validation.approver.frameIndex === second.validation.approver.frameIndex
      && first.validation.approver.transitionKey === second.validation.approver.transitionKey,
    `${label} terminal pose did not hold`,
    [first.validation.approver, second.validation.approver],
  );
  assertStableCounts([first, second], label);
  return [first, second];
};

const idleDashboardFrom = (dashboard, suffix) => {
  const idle = JSON.parse(JSON.stringify(dashboard));
  idle.scene.validation = {
    ...idle.scene.validation,
    stationState: "idle",
    severity: "muted",
    label: "QA/CI idle capture",
    recordKey: null,
    phase: null,
    packetKind: null,
    workerLeaseActive: false,
    approver: {
      state: "idle",
      qualifier: "none",
      recordKey: null,
      pullRequestNumber: null,
      evidenceShortSha: null,
      integrationMethod: null,
      requiredChecksSucceeded: 0,
      requiredChecksTotal: 0,
      findingCount: 0,
      remediationCount: 0,
      statusLabel: "Awaiting GitHub rebase-merge validation",
      transitionKey: `capture-idle:${suffix}`,
    },
  };
  return idle;
};

const captureBoard = async (page, filePath) => {
  const board = page.locator(".office-board");
  await board.scrollIntoViewIfNeeded();
  await board.screenshot({ path: filePath, type: "png", animations: "allow" });
};

const assertApproverCanvasSelection = async (page, dashboard, label) => {
  const inspection = await inspect(page);
  assertApprover(inspection, dashboard, {
    state: dashboard.scene.validation.approver.state,
    qualifier: dashboard.scene.validation.approver.qualifier,
    label,
  });
  const approver = inspection.validation.approver;
  const expectedRecordKey = approver.recordKey;
  assert(Boolean(expectedRecordKey), `${label} has no validation record to open`, approver);

  const drawer = page.locator("[data-detail-drawer]");
  if (await drawer.evaluate((element) => element.classList.contains("is-open"))) {
    await page.locator("[data-detail-close]").click();
    await page.waitForFunction(
      () => !document.querySelector("[data-detail-drawer]")?.classList.contains("is-open"),
      undefined,
      { timeout: timeoutMs },
    );
  }

  await page.evaluate(() => {
    window.__akraApproverCanvasSelections = [];
    if (window.__akraApproverCanvasSelectionListenerInstalled) return;
    window.__akraApproverCanvasSelectionListenerInstalled = true;
    window.addEventListener("akra:scene-selection-requested", (event) => {
      window.__akraApproverCanvasSelections.push({ ...(event.detail || {}) });
    });
  });

  const canvas = page.locator("#pixi-diorama canvas");
  const canvasBox = await canvas.boundingBox();
  assert(Boolean(canvasBox), `${label} canvas has no browser bounds`);
  const hitPoint = {
    x: approver.boardX,
    y: approver.boardY - approver.displayHeight * 0.5,
  };
  assert(
    hitPoint.x > 0
      && hitPoint.x < canvasBox.width
      && hitPoint.y > 0
      && hitPoint.y < canvasBox.height,
    `${label} computed approver hit point is outside the canvas`,
    { hitPoint, canvasBox, approver },
  );

  // Use a real pointer event on the WebGL canvas. This specifically guards the
  // overlap with DELIVERY's broader POI hit area; dispatching the custom event
  // directly would miss the z-order regression this contract is meant to catch.
  await canvas.click({ position: hitPoint });
  await page.waitForFunction(
    () => document.querySelector("[data-detail-drawer]")?.classList.contains("is-open"),
    undefined,
    { timeout: timeoutMs },
  );

  const opened = await page.evaluate(() => {
    const drawerElement = document.querySelector("[data-detail-drawer]");
    const distributor = document.querySelector(".distributor-desk");
    const selectedValidation = [...document.querySelectorAll(
      "#validation-rail [data-validation-record-key]",
    )].find((node) => node.getAttribute("aria-expanded") === "true");
    return {
      detailMode: drawerElement?.dataset.detailMode ?? null,
      recordKey: drawerElement?.dataset.validationRecordKey ?? null,
      title: document.querySelector("[data-detail-drawer-title]")?.textContent?.trim() ?? "",
      distributorExpanded: distributor?.getAttribute("aria-expanded") ?? null,
      selectedValidationRecordKey: selectedValidation?.dataset.validationRecordKey ?? null,
      selectionEvents: [...(window.__akraApproverCanvasSelections || [])],
    };
  });
  assert(opened.detailMode === "validation", `${label} canvas click opened a non-validation drawer`, opened);
  assert(opened.recordKey === expectedRecordKey, `${label} canvas click opened the wrong validation record`, opened);
  assert(opened.distributorExpanded !== "true", `${label} canvas click selected the distributor underneath`, opened);
  assert(
    opened.selectedValidationRecordKey === expectedRecordKey,
    `${label} validation rail selection does not match the approver record`,
    opened,
  );
  const selectionEvent = opened.selectionEvents.at(-1);
  assert(
    selectionEvent?.kind === "poi"
      && selectionEvent.detailTarget === "validation"
      && selectionEvent.recordKey === expectedRecordKey,
    `${label} canvas click dispatched the wrong scene selection`,
    opened,
  );

  await page.waitForFunction(
    (recordKey) =>
      document.querySelector("[data-validation-detail-record-key]")
        ?.getAttribute("data-validation-detail-record-key") === recordKey,
    expectedRecordKey,
    { timeout: timeoutMs },
  );
  const loadedRecordKey = await page.locator("[data-validation-detail-record-key]")
    .getAttribute("data-validation-detail-record-key");
  assert(loadedRecordKey === expectedRecordKey, `${label} fetched validation detail identity mismatch`, {
    expectedRecordKey,
    loadedRecordKey,
  });

  await page.locator("[data-detail-close]").click();
  await page.waitForFunction(
    () => !document.querySelector("[data-detail-drawer]")?.classList.contains("is-open"),
    undefined,
    { timeout: timeoutMs },
  );
  return {
    hitPoint,
    canvas: { width: canvasBox.width, height: canvasBox.height },
    expectedRecordKey,
    loadedRecordKey,
    opened,
  };
};

const assertNamedCountsUnchanged = (namedInspections) => {
  const entries = Object.entries(namedInspections);
  const baseline = countSnapshot(entries[0][1]);
  for (const [name, inspection] of entries.slice(1)) {
    assert(
      JSON.stringify(countSnapshot(inspection)) === JSON.stringify(baseline),
      `approver state ${name} changed worker/character counts`,
      Object.fromEntries(entries.map(([key, value]) => [key, countSnapshot(value)])),
    );
  }
};

const assertReducedStaticPose = async (
  page,
  dashboard,
  { state, qualifier, sourceFrameIndex, clip, label },
) => {
  const first = await inspect(page);
  await delay(800);
  const second = await inspect(page);
  for (const inspection of [first, second]) {
    assertApprover(inspection, dashboard, {
      state,
      qualifier,
      label,
      reducedMotion: true,
    });
    const approver = inspection.validation.approver;
    assert(approver.settled === true, `${label} reduced-motion pose did not settle`, approver);
    assert(approver.sourceFrameIndex === sourceFrameIndex, `${label} representative frame mismatch`, approver);
    assert(approver.clip === clip, `${label} representative clip mismatch`, approver);
  }
  assert(
    first.validation.approver.sourceFrameIndex === second.validation.approver.sourceFrameIndex
      && first.validation.approver.transitionKey === second.validation.approver.transitionKey,
    `${label} reduced-motion pose advanced`,
    [first.validation.approver, second.validation.approver],
  );
  assertStableCounts([first, second], label);
  return [first, second];
};

await mkdir(outputDir, { recursive: true });

const evidence = {
  schemaVersion: 1,
  status: "running",
  generatedAt: new Date().toISOString(),
  targetUrl,
  outputDir,
  screenshots: {
    reviewing: path.basename(screenshotPaths.reviewing),
    failure: path.basename(screenshotPaths.failure),
    success: path.basename(screenshotPaths.success),
  },
  normalMotion: {},
  reducedMotion: {},
  browserErrors: [],
};

let browser;
try {
  browser = await chromium.launch({
    executablePath: browserPath,
    headless: true,
    args: [
      "--no-sandbox",
      ...(target.hostname.endsWith(".localhost")
        ? [`--host-resolver-rules=MAP ${target.hostname} 127.0.0.1`]
        : []),
    ],
  });

  const normalContext = await browser.newContext({
    viewport: { width: 1600, height: 1000 },
  });
  await authenticate(normalContext);
  const authenticatedState = await normalContext.storageState();
  const normalPage = await normalContext.newPage();
  attachBrowserErrorCollectors(normalPage, "normal-motion", evidence.browserErrors);
  await openDashboard(normalPage);

  const successSelection = await selectScenario(normalPage, POST_MERGE_SUCCESS);
  let successDashboard = successSelection.dashboard;
  const reviewingInitial = await inspect(normalPage);
  assertApprover(reviewingInitial, successDashboard, {
    state: "reviewing",
    qualifier: "none",
    label: "post-merge reviewing",
  });

  const idleDashboard = idleDashboardFrom(successDashboard, "normal");
  await applyDashboard(normalPage, idleDashboard);
  const idleSamples = await sampleAnimation(normalPage, 2, 500);
  for (const idleInspection of idleSamples) {
    assertApprover(idleInspection, idleDashboard, {
      state: "idle",
      qualifier: "none",
      label: "idle",
    });
    assert(idleInspection.validation.approver.clip === "idle", "idle clip mismatch", idleInspection.validation.approver);
    assert(idleInspection.validation.approver.sourceFrameIndex === 0, "idle frame mismatch", idleInspection.validation.approver);
    assert(idleInspection.validation.approver.settled === true, "idle pose did not hold", idleInspection.validation.approver);
  }
  assertStableCounts(idleSamples, "idle");

  await applyDashboard(normalPage, successDashboard);
  const reviewingSamples = await sampleAnimation(normalPage, 9, 220);
  for (const reviewingInspection of reviewingSamples) {
    assertApprover(reviewingInspection, successDashboard, {
      state: "reviewing",
      qualifier: "none",
      label: "reviewing animation",
    });
  }
  const reviewingFrames = new Set(
    reviewingSamples.map((sample) => sample.validation.approver.sourceFrameIndex),
  );
  assert(reviewingFrames.size >= 3, "reviewing animation did not advance through document frames", {
    frames: [...reviewingFrames],
  });
  assert(
    reviewingSamples.at(-1).renderCount > reviewingSamples[0].renderCount,
    "reviewing animation render loop did not advance",
    reviewingSamples.map((sample) => sample.renderCount),
  );
  assertStableCounts(reviewingSamples, "reviewing");
  await captureBoard(normalPage, screenshotPaths.reviewing);
  const approverCanvasSelection = await assertApproverCanvasSelection(
    normalPage,
    successDashboard,
    "reviewing approver interaction",
  );

  const postMergeTrace = [];
  for (const expectedStage of ["delivering", "reviewing", "complete"]) {
    const transition = await stepScenarioThroughUi(normalPage, expectedStage);
    successDashboard = transition.dashboard;
    const inspection = await inspect(normalPage);
    assertWorkerParity(inspection, successDashboard, `post-merge ${expectedStage}`);
    postMergeTrace.push({
      stage: expectedStage,
      revision: transition.harness.revision,
      inspection: compactInspection(inspection),
    });
  }
  const successHold = await assertTerminalHold(
    normalPage,
    successDashboard,
    "success",
    "post-merge success",
  );
  assert(successHold[0].validation.approver.sourceFrameIndex === 23, "success terminal frame mismatch", successHold[0].validation.approver);
  await captureBoard(normalPage, screenshotPaths.success);

  const failureSelection = await selectScenario(normalPage, CHECK_FAILURE_RECOVERY);
  let recoveryDashboard = failureSelection.dashboard;
  const recoveryTrace = [];
  for (const expectedStage of ["reviewing", "blocked"]) {
    const transition = await stepScenarioThroughUi(normalPage, expectedStage);
    recoveryDashboard = transition.dashboard;
    recoveryTrace.push({
      stage: expectedStage,
      revision: transition.harness.revision,
      inspection: compactInspection(await inspect(normalPage)),
    });
  }

  const failureSamples = await sampleAnimation(normalPage, 7, 200);
  for (const failureInspection of failureSamples) {
    assertApprover(failureInspection, recoveryDashboard, {
      state: "failure",
      qualifier: "none",
      label: "failure animation",
    });
  }
  const failureFrames = new Set(
    failureSamples.map((sample) => sample.validation.approver.sourceFrameIndex),
  );
  assert(failureFrames.size >= 2, "failure reaction frames did not advance", {
    frames: [...failureFrames],
  });
  assertStableCounts(failureSamples, "failure");
  await captureBoard(normalPage, screenshotPaths.failure);
  const failureHold = await assertTerminalHold(
    normalPage,
    recoveryDashboard,
    "failure",
    "failure",
  );
  assert(failureHold[0].validation.approver.sourceFrameIndex === 17, "failure terminal frame mismatch", failureHold[0].validation.approver);

  const recoveringTransition = await stepScenarioThroughUi(normalPage, "intake");
  recoveryDashboard = recoveringTransition.dashboard;
  const recoveringInspection = await inspect(normalPage);
  assertApprover(recoveringInspection, recoveryDashboard, {
    state: "failure",
    qualifier: "recovering",
    label: "recovering handoff",
  });
  recoveryTrace.push({
    stage: "intake",
    revision: recoveringTransition.harness.revision,
    inspection: compactInspection(recoveringInspection),
  });

  for (const expectedStage of ["dispatching", "working", "reviewing", "complete"]) {
    const transition = await stepScenarioThroughUi(normalPage, expectedStage);
    recoveryDashboard = transition.dashboard;
    const inspection = await inspect(normalPage);
    assertWorkerParity(inspection, recoveryDashboard, `recovery ${expectedStage}`);
    recoveryTrace.push({
      stage: expectedStage,
      revision: transition.harness.revision,
      inspection: compactInspection(inspection),
    });
  }
  const recoverySuccessHold = await assertTerminalHold(
    normalPage,
    recoveryDashboard,
    "success",
    "recovery success",
  );

  assertNamedCountsUnchanged({
    idle: idleSamples[0],
    reviewing: reviewingSamples[0],
    failure: failureSamples[0],
    recovering: recoveringInspection,
    success: recoverySuccessHold[0],
  });

  evidence.normalMotion = {
    idle: idleSamples.map(compactInspection),
    reviewing: {
      uniqueSourceFrames: [...reviewingFrames],
      samples: reviewingSamples.map(compactInspection),
    },
    approverCanvasSelection,
    postMergeTrace,
    successTerminalHold: successHold.map(compactInspection),
    failure: {
      uniqueSourceFrames: [...failureFrames],
      samples: failureSamples.map(compactInspection),
      terminalHold: failureHold.map(compactInspection),
    },
    recoveryTrace,
    recoverySuccessTerminalHold: recoverySuccessHold.map(compactInspection),
  };

  const reducedContext = await browser.newContext({
    storageState: authenticatedState,
    viewport: { width: 1600, height: 1000 },
    reducedMotion: "reduce",
  });
  const reducedPage = await reducedContext.newPage();
  attachBrowserErrorCollectors(reducedPage, "reduced-motion", evidence.browserErrors);
  await openDashboard(reducedPage);

  const reducedSuccessSelection = await selectScenario(reducedPage, POST_MERGE_SUCCESS);
  let reducedDashboard = reducedSuccessSelection.dashboard;
  const reducedReviewing = await assertReducedStaticPose(reducedPage, reducedDashboard, {
    state: "reviewing",
    qualifier: "none",
    sourceFrameIndex: 6,
    clip: "review",
    label: "reduced reviewing",
  });

  const reducedIdleDashboard = idleDashboardFrom(reducedDashboard, "reduced");
  await applyDashboard(reducedPage, reducedIdleDashboard);
  const reducedIdle = await assertReducedStaticPose(reducedPage, reducedIdleDashboard, {
    state: "idle",
    qualifier: "none",
    sourceFrameIndex: 0,
    clip: "idle",
    label: "reduced idle",
  });

  const reducedFailureSelection = await selectScenario(reducedPage, CHECK_FAILURE_RECOVERY);
  reducedDashboard = reducedFailureSelection.dashboard;
  for (const expectedStage of ["reviewing", "blocked"]) {
    reducedDashboard = (await stepScenarioThroughUi(reducedPage, expectedStage)).dashboard;
  }
  const reducedFailure = await assertReducedStaticPose(reducedPage, reducedDashboard, {
    state: "failure",
    qualifier: "none",
    sourceFrameIndex: 17,
    clip: "failure",
    label: "reduced failure",
  });

  reducedDashboard = (await stepScenarioThroughUi(reducedPage, "intake")).dashboard;
  const reducedRecovering = await assertReducedStaticPose(reducedPage, reducedDashboard, {
    state: "failure",
    qualifier: "recovering",
    sourceFrameIndex: 17,
    clip: "failure",
    label: "reduced recovering",
  });

  for (const expectedStage of ["dispatching", "working", "reviewing", "complete"]) {
    reducedDashboard = (await stepScenarioThroughUi(reducedPage, expectedStage)).dashboard;
  }
  const reducedSuccess = await assertReducedStaticPose(reducedPage, reducedDashboard, {
    state: "success",
    qualifier: "none",
    sourceFrameIndex: 23,
    clip: "success",
    label: "reduced success",
  });

  assertNamedCountsUnchanged({
    idle: reducedIdle[0],
    reviewing: reducedReviewing[0],
    failure: reducedFailure[0],
    recovering: reducedRecovering[0],
    success: reducedSuccess[0],
  });

  evidence.reducedMotion = {
    idle: reducedIdle.map(compactInspection),
    reviewing: reducedReviewing.map(compactInspection),
    failure: reducedFailure.map(compactInspection),
    recovering: reducedRecovering.map(compactInspection),
    success: reducedSuccess.map(compactInspection),
  };

  await reducedPage.close();
  await reducedContext.close();
  await normalPage.close();
  await normalContext.close();

  assert(evidence.browserErrors.length === 0, "browser errors were observed", evidence.browserErrors);
  evidence.status = "passed";
} catch (error) {
  evidence.status = "failed";
  evidence.error = error instanceof Error
    ? { message: error.message, stack: error.stack }
    : { message: String(error) };
  throw error;
} finally {
  evidence.completedAt = new Date().toISOString();
  await writeFile(
    path.join(outputDir, "evidence.json"),
    `${JSON.stringify(evidence, null, 2)}\n`,
    "utf8",
  );
  await browser?.close();
}
