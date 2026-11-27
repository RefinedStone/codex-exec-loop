// Manual acceptance recorder. Runs the real Windows Electron build against a named-pipe fake.
import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { release, tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import * as fake from "../../../desktop/test/fake-gateway/fakeGateway.ts";

const desktop = fileURLToPath(new URL("../../../desktop", import.meta.url));
const requireDesktop = createRequire(join(desktop, "package.json"));
const { _electron: electron } = requireDesktop("playwright-core");
const scenarios = {
  regular: { width: 1200, height: 800, zoom: 1, panel: false, scale: null },
  compact: { width: 1024, height: 700, zoom: 1.25, panel: true, scale: 1.25 },
};
const [name, destination, sha] = process.argv.slice(2);
assert.equal(process.platform, "win32", "this recorder requires Windows");
assert.ok(Object.hasOwn(scenarios, name) && destination && /^[a-f0-9]{40}$/.test(sha ?? ""),
  "usage: node capture.mjs regular|compact OUTPUT_DIRECTORY BUILT_COMMIT_SHA");
const scenario = scenarios[name];
const output = resolve(destination);
await mkdir(output, { recursive: true });
const root = await mkdtemp(join(tmpdir(), "akra-2965-"));
const requests = [];
const checks = {};
const frames = {};
const stages = {};
const expectedCode = `const horizon = "${"0123456789".repeat(24)}";\nconsole.log(horizon);`;
const expectedText = `Windows acceptance fixture.\n\nSecond paragraph for task provenance.\n\n${expectedCode}`;
const markdown = `${expectedText.slice(0, -(expectedCode.length + 2))}\n\n\`\`\`js\n${expectedCode}\n\`\`\``;
let gateway;
let app;
let display;
try {
  const recording = join(root, "transcript.jsonl");
  const recorded = (await readFile(fake.INTERRUPTED_TURN_RECORDING, "utf8")).trim().split(/\r?\n/);
  let source;
  await writeFile(recording, recorded.map((line) => {
    const frame = JSON.parse(line);
    const message = JSON.parse(frame.text);
    const thread = message.result?.thread;
    if (!thread) return line;
    const turn = thread.turns.at(-1);
    turn.items.find((item) => item.type === "agentMessage").text = markdown;
    source = { thread: thread.id, turn: turn.id };
    frame.text = JSON.stringify(message);
    return JSON.stringify(frame);
  }).join("\n") + "\n");
  assert.ok(source, "synthetic source exists in the recording");
  let attachPending = true;
  let releaseAttach;
  const attachGate = new Promise((resolve) => { releaseAttach = resolve; });
  const endpoint = fake.uniqueTestEndpoint();
  gateway = await fake.startFakeGateway(endpoint, [fake.APPROVAL_THREAD_LIST_RECORDING, recording], {
    skills: () => ({ data: [{ cwd: "/work/example-app", skills: [], errors: [] }] }),
    kernel: {
      kernel: "windows-2965-fixture",
      queries: {
        list_tasks: { type: "list_tasks", available: true, planning_revision: 1, tasks: [] },
        thread_activity: { type: "thread_activity", threads: [] },
        auto_follow: { type: "auto_follow", max_turns: 0, paused: true },
      },
      onRequest(method) {
        if (method !== "akra/kernel/attach" || !attachPending) return undefined;
        return { result: { client_id: "1", protocol_version: 1, kernel: "windows-2965-fixture" }, replyAfter: attachGate };
      },
    },
    onRequest(method, params) { requests.push({ method, command: params.command?.type }); },
  });
  const env = {
    ...process.env, HOME: root, USERPROFILE: root,
    CODEX_HOME: join(root, ".codex"),
    AKRA_GATEWAY_ENDPOINT: endpoint, AKRA_DESKTOP_HIDDEN: "1",
    AKRA_DESKTOP_USER_DATA: join(root, "user-data"),
  };
  delete env.ELECTRON_RUN_AS_NODE;
  const args = [desktop, "--disable-background-timer-throttling",
    ...(scenario.scale ? [`--force-device-scale-factor=${scenario.scale}`] : [])];
  app = await electron.launch({ executablePath: requireDesktop("electron"), args, env, timeout: 20_000 });
  const page = await app.firstWindow();
  page.setDefaultTimeout(12_000);
  display = await app.evaluate(({ BrowserWindow, screen }, { width, height }) => {
    const window = BrowserWindow.getAllWindows()[0];
    window.setContentSize(width, height);
    const monitor = screen.getDisplayMatching(window.getBounds());
    return { electron: process.versions.electron, monitorDips: monitor.size,
      displayScale: monitor.scaleFactor, contentDips: window.getContentSize() };
  }, scenario);
  const conversation = page.getByTestId("conversation");
  const action = page.getByTestId("draft-task-from-selection");
  const menu = page.getByTestId("conversation-menu");
  const editor = page.getByTestId("task-editor");
  const composer = page.getByTestId("composer-input");
  const body = conversation.locator(".agent-message .markdown").filter({ hasText: "Windows acceptance fixture." });
  const snapshot = () => page.evaluate(() => {
    const chat = document.querySelector('[data-testid="conversation"]');
    const panel = document.querySelector('[data-testid="right-panel"]');
    const box = (element) => {
      if (!element) return null;
      const { x, y, width, height } = element.getBoundingClientRect();
      return { x, y, width, height };
    };
    const active = document.activeElement;
    const code = [...chat.querySelectorAll("pre")].find((pre) => pre.textContent.includes("const horizon"));
    return {
      viewport: [innerWidth, innerHeight], dpr: devicePixelRatio,
      zoom: getComputedStyle(document.querySelector(".main-view")).zoom,
      conversation: box(chat), column: box(chat.querySelector(".column")),
      composer: box(document.querySelector(".composer")),
      scrollTop: chat.scrollTop, scrollLeft: chat.scrollLeft,
      verticalExtent: [chat.clientHeight, chat.scrollHeight],
      horizontalExtent: [chat.clientWidth, chat.scrollWidth],
      codeExtent: code ? [code.clientWidth, code.scrollWidth] : null,
      panel: box(panel), tab: panel?.querySelector('[aria-pressed="true"]')?.textContent ?? null,
      draft: document.querySelector('[data-testid="composer-input"]').value,
      focus: active?.getAttribute("data-testid") ?? (active?.id || active?.tagName),
      action: Boolean(document.querySelector('[data-testid="draft-task-from-selection"]')),
      legacyArea: Boolean(document.querySelector(".conversation-task-action")),
    };
  });
  const capture = async (stage) => {
    await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    const frame = await app.evaluate(async ({ BrowserWindow }) => {
      const image = await BrowserWindow.getAllWindows()[0].webContents.capturePage(undefined, { stayHidden: true });
      return { png: image.toPNG().toString("base64"), pixels: image.getSize() };
    });
    assert.ok(frame.pixels.width > 0 && frame.pixels.height > 0, "Electron provided a rendered frame");
    const file = `${name}-${stage}.png`;
    await writeFile(join(output, file), Buffer.from(frame.png, "base64"));
    frames[stage] = { file, pixels: frame.pixels };
  };
  const clear = () => page.evaluate(() => window.getSelection().removeAllRanges());
  const select = (selector) => body.evaluate((element, child) => {
    const range = document.createRange();
    range.selectNodeContents(child ? element.querySelector(child) : element);
    const selection = window.getSelection();
    selection.removeAllRanges();
    selection.addRange(range);
  }, selector);
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor();
  await page.getByTestId("thread-entry").first().click();
  await body.waitFor();
  if (scenario.panel) {
    await page.getByTestId("skills-toggle").click();
    await page.getByTestId("skills-panel").waitFor();
    await page.evaluate(() => {
      window.akra.zoom.step("thread", "in");
      window.akra.zoom.step("thread", "in");
    });
    await page.waitForFunction(() => getComputedStyle(document.querySelector(".main-view")).zoom === "1.25");
  }
  await composer.fill("Unsaved Windows acceptance fixture");
  await conversation.focus();
  stages.pending = await snapshot();
  assert.ok(requests.some(({ method }) => method === "akra/kernel/attach"),
    "an attach request was held while measuring pending geometry");
  attachPending = false;
  releaseAttach();
  // Readiness is observed through the actual task action, not a fixture-only store mutation.
  await body.locator("p").first().evaluate((element) => {
    return new Promise((resolve, reject) => {
      const deadline = performance.now() + 5000;
      const probe = () => {
        if (document.querySelector('[data-testid="draft-task-from-selection"]')) return resolve();
        if (performance.now() > deadline) return reject(new Error("Task action never became available after attachment"));
        const range = document.createRange();
        range.selectNodeContents(element);
        window.getSelection().removeAllRanges();
        window.getSelection().addRange(range);
        element.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true }));
        requestAnimationFrame(probe);
      };
      probe();
    });
  });
  await action.waitFor();
  await page.keyboard.press("Escape");
  await menu.waitFor({ state: "hidden" });
  await clear();
  await conversation.focus();
  // Keep the pointer target in view so Playwright does not scroll it into view before a click.
  await body.locator("p").first().scrollIntoViewIfNeeded();
  stages.before = await snapshot();
  await capture("before");
  await select("p");
  stages.selected = await snapshot();
  await select(null);
  stages.changed = await snapshot();
  await clear();
  stages.cleared = await snapshot();
  await body.locator("p").first().click({ button: "right" });
  await menu.waitFor();
  checks.emptyContextMenuHasNoTaskAction = (await action.count()) === 0;
  await page.keyboard.press("Escape");
  await menu.waitFor({ state: "hidden" });
  await conversation.focus();
  await select("p");
  await page.keyboard.press("Shift+F10");
  await action.waitFor();
  checks.keyboardActionFocused = await action.evaluate((element) => document.activeElement === element);
  checks.copyAndQuotePresent = (await menu.getByTestId("conversation-menu-copy-selection").count()) === 1
    && (await menu.getByTestId("conversation-menu-quote").count()) === 1;
  await capture("menu");
  await page.keyboard.press("Escape");
  await menu.waitFor({ state: "hidden" });
  stages.menuEscape = await snapshot();
  await conversation.focus();
  await select(null);
  await body.locator("p").first().click({ button: "right" });
  await action.waitFor();
  await clear();
  await action.click();
  await editor.waitFor();
  checks.dialogOwnsFocus = await editor.evaluate((element) => element.contains(document.activeElement));
  checks.exactMultilineDraft = (await page.locator("#task-editor-description").inputValue()) === expectedText;
  await editor.locator("summary").click();
  const provenance = await editor.locator(".task-editor-source .muted").innerText();
  checks.sourceThreadAndTurn = provenance.includes(source.thread) && provenance.includes(source.turn);
  await capture("editor");
  await page.keyboard.press("Escape");
  await editor.waitFor({ state: "hidden" });
  stages.after = await snapshot();
  await capture("after");
  const comparable = ({ focus, ...geometry }) => geometry;
  checks.geometryUnchanged = ["selected", "changed", "cleared", "menuEscape", "after"].every((stage) =>
    JSON.stringify(comparable(stages[stage])) === JSON.stringify(comparable(stages.before)));
  checks.noPersistentAction = Object.values(stages).every((stage) => !stage.action && !stage.legacyArea);
  checks.pendingAndAttachedAlign = JSON.stringify(comparable(stages.pending)) === JSON.stringify(comparable(stages.before));
  checks.overflowStaysInCode = stages.before.horizontalExtent[0] === stages.before.horizontalExtent[1]
    && stages.before.codeExtent?.[1] > stages.before.codeExtent?.[0];
  checks.menuEscapeRestoresFocus = stages.menuEscape.focus === "conversation";
  checks.editorEscapeRestoresFocus = stages.after.focus === "conversation";
  checks.panelAndDraftPreserved = ["panel", "tab", "draft"].every((key) =>
    JSON.stringify(stages.before[key]) === JSON.stringify(stages.after[key]));
  checks.noTaskMutationOrModelRequests = !requests.some(({ method, command }) =>
    /^turn\/(start|steer)$/.test(method) || ["create_task", "update_task"].includes(command));
} finally {
  try {
    await app?.close();
  } finally {
    try {
      await gateway?.close();
    } finally {
      await rm(root, { recursive: true, force: true });
      await writeFile(join(output, `${name}.json`), JSON.stringify({
        sha, platform: process.platform, release: release(), node: process.version, scenario,
        display, checks, stages, frames,
      }, null, 2) + "\n");
    }
  }
}
console.log(`${name}: ${JSON.stringify(checks)}`);
