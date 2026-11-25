// 화면 시험 (plan 26 U1, #2626): 영역별 확대와 저장, 옛 확대 값 옮기기, 탭을 확대한 뒤 브라우저 패널의 자리, 이동 기록, 대화 안 찾기, 이전 프롬프트 불러오기,
// 창 자리 저장. 실제 codex 와 턴은 쓰지 않는다 (가짜 Gateway). userData 는 임시 폴더를 쓴다.
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { _electron as electron } from "playwright-core";

const desktopDir = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const electronPath = createRequire(import.meta.url)("electron");
const fake = await import(pathToFileURL(join(desktopDir, "test/fake-gateway/fakeGateway.ts")).href);

const userData = mkdtempSync(join(tmpdir(), "akra-e2e-basics-"));
const endpoint = fake.uniqueTestEndpoint();
// 합성 응답에 소문자화하면 길이가 늘어나는 문자를 넣어 실제 DOM Range도 검사한다.
const unicodeRecording = join(userData, "unicode-approval-turn.jsonl");
function addUnicodeAnswer(value) {
  if (!value || typeof value !== "object") return;
  if (value.type === "agentMessage" && typeof value.text === "string" && value.text) value.text += "\nİstanbul";
  for (const child of Object.values(value)) addUnicodeAnswer(child);
}
const recording = readFileSync(fake.APPROVAL_TURN_RECORDING, "utf8").split(/\r?\n/).filter(Boolean).map((line) => {
  const entry = JSON.parse(line);
  const message = JSON.parse(entry.text);
  addUnicodeAnswer(message);
  entry.text = JSON.stringify(message);
  return JSON.stringify(entry);
}).join("\n");
writeFileSync(unicodeRecording, recording);
const gateway = await fake.startFakeGateway(endpoint, [fake.APPROVAL_THREAD_LIST_RECORDING, unicodeRecording]);
const env = { ...process.env, AKRA_GATEWAY_ENDPOINT: endpoint, AKRA_DESKTOP_HIDDEN: "1", AKRA_DESKTOP_USER_DATA: userData };
delete env.ELECTRON_RUN_AS_NODE;

const failures = [];
const check = (name, ok, detail = "") => {
  console.log(`${ok ? "PASS" : "FAIL"} ${name}${!ok && detail ? ` | ${detail}` : ""}`);
  if (!ok) failures.push(name);
};
const savedState = () => JSON.parse(readFileSync(join(userData, "window-state.json"), "utf8"));

// 옛 판이 남긴 파일이다: 앱 전체 175%. 새 판은 본문 보기로만 옮긴다.
writeFileSync(join(userData, "window-state.json"), JSON.stringify({ zoom: 1.75, bounds: null, maximized: false }));
const app = await electron.launch({ executablePath: electronPath, args: [desktopDir, ...(process.env.CI ? ["--no-sandbox"] : [])], env });
try {
  const page = await app.firstWindow();
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
  // 메뉴 항목을 이 창에서 누른 것처럼 부른다. main 은 메뉴의 확대를 누른 창에 맡긴다.
  const clickMenu = (id) => app.evaluate(({ BrowserWindow, Menu }, itemId) => Menu.getApplicationMenu()?.getMenuItemById(itemId)?.click(undefined, BrowserWindow.getAllWindows()[0]), id);
  const zoomOf = (scope) => page.evaluate(async (key) => (await window.akra.zoom.get())[key] ?? 1, scope);
  const zoomIs = (scope, factor) => page.waitForFunction(async ([key, expected]) => Math.abs(((await window.akra.zoom.get())[key] ?? 1) - expected) < 0.001, [scope, factor], { timeout: 5_000 }).then(() => true, () => false);
  const box = (testId) => page.getByTestId(testId).evaluate((element) => {
    const rect = element.getBoundingClientRect();
    return { x: rect.left, y: rect.top, width: rect.width, height: rect.height };
  });
  const cssZoom = (selector) => page.locator(selector).first().evaluate((element) => getComputedStyle(element).zoom);
  const sidebarBefore = await box("folders");
  const topBarBefore = await box("gateway-info");
  const pixelRatio = await page.evaluate(() => window.devicePixelRatio);

  // 옛 판의 앱 전체 175% 는 본문 보기로만 옮겨 왔다. 사이드바와 위 막대는 100% 이다 (#2627).
  check("an old single zoom moves to the body views", (await zoomOf("home")) === 1.75 && (await zoomOf("thread")) === 1.75 && (await zoomOf("sidebar")) === 1, JSON.stringify(savedState()));
  check("the body view is zoomed with CSS", (await cssZoom(".main-view")) === "1.75");
  check("the page itself stays at 100%", (await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0]?.webContents.getZoomFactor())) === 1);

  // 메뉴는 포커스가 없으면 지금 본문 보기를 바꾼다. 사이드바와 위 막대, 창의 픽셀 비율은 그대로이다 (#2628).
  await clickMenu("zoom-in");
  check("the view menu zooms the body view one step", await zoomIs("home", 2));
  check("the window shows the scope and the new factor for a moment", (await page.getByTestId("zoom-notice").textContent()) === "홈 200%");
  check("the body factor is saved in the userData folder", savedState().zooms?.home === 2 && savedState().zoom === undefined, JSON.stringify(savedState()));
  const sidebarAfter = await box("folders");
  const topBarAfter = await box("gateway-info");
  check("the sidebar keeps its size while the body zooms", sidebarAfter.width === sidebarBefore.width, JSON.stringify({ sidebarBefore, sidebarAfter }));
  check("the top bar keeps its size while the body zooms", topBarAfter.height === topBarBefore.height, JSON.stringify({ topBarBefore, topBarAfter }));
  check("the device pixel ratio does not change", (await page.evaluate(() => window.devicePixelRatio)) === pixelRatio);
  for (const scope of ["home", "fleet", "thread", "remote"]) await page.evaluate((key) => window.akra.zoom.step(key, "reset"), scope);
  check("each body view resets on its own", await zoomIs("remote", 1) && (await cssZoom(".main-view")) === "1");

  // ⌘ + 휠은 포인터 아래의 영역을 바꾼다. 사이드바 위에서는 바꾸지 않는다.
  await page.getByTestId("folders").hover();
  await page.keyboard.down("Control");
  await page.mouse.wheel(0, -120);
  await page.keyboard.up("Control");
  await page.waitForTimeout(200);
  check("a wheel over the sidebar does not zoom it", (await zoomOf("sidebar")) === 1);

  // 브라우저 패널의 내용을 125% 로 확대해도 웹 페이지는 빈 자리와 같은 곳에 있다 (#2629).
  await page.getByTestId("browser-toggle").click();
  await page.getByTestId("browser-slot").waitFor({ timeout: 10_000 });
  await page.evaluate(() => {
    window.akra.zoom.step("panel:browser", "in");
    window.akra.zoom.step("panel:browser", "in");
  });
  check("the browser tab zooms on its own", await zoomIs("panel:browser", 1.25) && (await cssZoom(".panel-body")) === "1.25");
  const placed = async () => {
    const slot = await box("browser-slot");
    const view = await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0]?.contentView.children.at(-1)?.getBounds() ?? null);
    const near = (key) => view !== null && Math.abs(view[key] - slot[key]) <= 2;
    return { ok: near("x") && near("y") && near("width") && near("height"), slot, view };
  };
  let placement = await placed();
  for (let attempt = 0; attempt < 20 && !placement.ok; attempt += 1) {
    await page.waitForTimeout(100);
    placement = await placed();
  }
  check("the browser page sits on its slot while the tab is zoomed", placement.ok, JSON.stringify(placement));
  // 웹 페이지 자체의 배율은 탭의 배율과 따로이다. 배율은 읽은 페이지에만 걸리므로 먼저 페이지를 읽는다.
  const loadPage = (html) => app.evaluate(({ BrowserWindow }, url) => BrowserWindow.getAllWindows()[0]?.contentView.children.at(-1)?.webContents.loadURL(url), `data:text/html,${html}`);
  const pageFactor = () => app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0]?.contentView.children.at(-1)?.webContents.getZoomFactor() ?? null);
  await loadPage("<p>one</p>");
  await page.evaluate(() => window.akra.zoom.step("browser-page", "in"));
  await zoomIs("browser-page", 1.1);
  check("the web page has its own factor", Math.abs((await pageFactor()) - 1.1) < 0.001, String(await pageFactor()));
  await loadPage("<p>two</p>");
  check("the web page keeps its factor after it loads another page", Math.abs((await pageFactor()) - 1.1) < 0.001, String(await pageFactor()));
  await page.evaluate(() => {
    window.akra.zoom.step("browser-page", "reset");
    window.akra.zoom.step("panel:browser", "reset");
  });
  check("the browser tab resets", await zoomIs("panel:browser", 1));
  await page.getByTestId("browser-toggle").click();

  // 이동 기록: 새 스레드 화면 → 스레드 → 뒤로 → 앞으로.
  check("back is off before the first move", await page.getByTestId("nav-back").isDisabled());
  await page.getByTestId("thread-entry").first().click();
  await page.getByTestId("approval").waitFor({ timeout: 15_000 });
  await page.getByTestId("nav-back").click();
  await page.getByTestId("home").waitFor({ timeout: 5_000 });
  check("back returns to the new-thread view", (await page.getByTestId("conversation").count()) === 0);
  await page.keyboard.press("ControlOrMeta+BracketRight");
  await page.getByTestId("conversation").waitFor({ timeout: 5_000 });
  check("the forward shortcut reopens the thread", await page.getByTestId("nav-forward").isDisabled());

  await page.getByTestId("approval-accept").click();
  await page.locator('[data-testid="turn"][data-status="completed"]').last().waitFor({ timeout: 15_000 });
  check("each message has a copy button", (await page.locator('[data-testid="item-agent"] .message-copy').count()) > 0);

  // 대화 안 찾기: 에이전트 답의 첫 낱말을 찾는다.
  const word = ((await page.getByTestId("item-agent").first().locator(".markdown").textContent()) ?? "").trim().split(/\s+/)[0] ?? "";
  await page.keyboard.press("ControlOrMeta+KeyF");
  await page.getByTestId("find-input").fill(word.toUpperCase());
  await page.getByTestId("find-count").filter({ hasText: "턴 1/" }).waitFor({ timeout: 5_000 });
  const marked = await page.evaluate(() => CSS.highlights.get("akra-find-current")?.size ?? 0);
  check("find marks the query in the conversation", word.length > 0 && marked > 0, `word=${word} marked=${marked}`);
  await page.getByTestId("find-input").fill("bul");
  await page.waitForFunction(() => [...(CSS.highlights.get("akra-find-current") ?? [])].some((range) => range.toString() === "bul"), null, { timeout: 5_000 });
  check("Unicode case expansion highlights the original substring without a DOM range error", true);
  await page.getByTestId("find-input").fill("akra-no-such-text");
  check("find reports no match", (await page.getByTestId("find-count").textContent()) === "없음");
  await page.keyboard.press("Escape");
  check("escape closes the find bar", (await page.getByTestId("find-bar").count()) === 0);

  // 이전 프롬프트: 빈 입력창에서 ↑ 는 가장 새 글, ↓ 는 다시 빈 입력창.
  await page.evaluate(() => window.localStorage.setItem("akra.promptHistory", JSON.stringify(["첫 프롬프트", "둘째 프롬프트"])));
  const input = page.getByTestId("composer-input");
  await input.focus();
  await input.press("ArrowUp");
  const newest = await input.inputValue();
  await input.press("ArrowUp");
  const older = await input.inputValue();
  await input.press("ArrowDown");
  await input.press("ArrowDown");
  check("arrow keys recall the sent prompts in an empty composer", newest === "둘째 프롬프트" && older === "첫 프롬프트" && (await input.inputValue()) === "", `${newest} / ${older}`);

  // 창 크기를 바꾸면 그 크기를 저장한다.
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0]?.setSize(1000, 700));
  let bounds = null;
  for (let attempt = 0; attempt < 30 && bounds?.width !== 1000; attempt += 1) {
    await page.waitForTimeout(100);
    bounds = savedState().bounds;
  }
  check("resizing the window saves its size and position", bounds?.width === 1000 && bounds?.height === 700, JSON.stringify(bounds));
} catch (error) {
  check(`flow finished without error (${String(error).split("\n")[0]})`, false);
} finally {
  const exited = new Promise((resolve) => app.process().once("exit", resolve));
  for (const window of app.windows()) await window.close();
  await Promise.race([exited, new Promise((resolve) => setTimeout(resolve, 10_000))]);
  await gateway.close();
  rmSync(userData, { recursive: true, force: true, maxRetries: 5 });
}
process.exit(failures.length === 0 ? 0 : 1);
