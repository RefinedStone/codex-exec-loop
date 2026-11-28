import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import electronPath from "electron";
import { _electron } from "playwright-core";
import { startFakeGateway, uniqueTestEndpoint, APPROVAL_THREAD_LIST_RECORDING, INTERRUPTED_TURN_RECORDING } from "../fake-gateway/fakeGateway.ts";

const desktop = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const profile = await mkdtemp(join(tmpdir(), "akra-attention-badge-"));
const endpoint = uniqueTestEndpoint();
const thread = "01a1abfb-8f0a-7581-a8e4-c2953347f470";
const item = {id:"thread:first", revision:"1", subject:{kind:"thread",id:thread}, cause:"turn_failed", summary:"Thread needs a response", next:[{step:"ask_user"}]};
let answer = {result:{type:"attention",items:[]}};
let refreshOnOverview = false;
const kernel = {kernel:"badge-repo", queries:{}, onRequest(method, params) {
  if (method === "akra/kernel/query" && params.query.type === "parallel" && refreshOnOverview) {
    refreshOnOverview = false;
    return {result:{type:"parallel",enabled:false},events:[{type:"thread_activity",threads:[]}]};
  }
  if (method === "akra/kernel/query" && params.query.type === "attention") return answer;
}};
const gateway = await startFakeGateway(endpoint, [APPROVAL_THREAD_LIST_RECORDING, INTERRUPTED_TURN_RECORDING], {kernel});
const env = {...process.env, AKRA_GATEWAY_ENDPOINT:endpoint, AKRA_DESKTOP_USER_DATA:profile, AKRA_DESKTOP_HIDDEN:"1"};
delete env.ELECTRON_RUN_AS_NODE;
const app = await _electron.launch({executablePath:electronPath,args:[desktop],env,timeout:30_000});
try {
  const first = await app.firstWindow();
  const opening = app.waitForEvent("window");
  await app.evaluate(({Menu}) => Menu.getApplicationMenu().getMenuItemById("new-window").click());
  const second = await opening;
  // Observe calls across the native boundary, but delegate every call to Electron itself.
  await app.evaluate(({app,BrowserWindow}) => {
    globalThis.badgeReports = [];
    if (process.platform === "win32") for (const window of BrowserWindow.getAllWindows()) {
      const original = window.setOverlayIcon.bind(window);
      window.setOverlayIcon = (icon, label) => { original(icon,label); globalThis.badgeReports.push([window.id,label,!!icon && !icon.isEmpty()]); app.emit("test-badge-applied"); };
    } else {
      const original = app.setBadgeCount.bind(app);
      app.setBadgeCount = n => { const applied = original(n); globalThis.badgeReports.push(n); app.emit("test-badge-applied"); return applied; };
    }
  });
  const waitBadge = count => app.evaluate(({app}, count) => new Promise((resolve, reject) => {
    const timer = setTimeout(() => { app.off("test-badge-applied", check); reject(new Error(`Expected badge ${count}; last native call: ${JSON.stringify(globalThis.badgeReports.at(-1))}`)); }, 5000);
    function check() {
      const last = globalThis.badgeReports.at(-1);
      if (process.platform === "win32" ? last?.[1] !== `${count} responses required` || !last[2] : last !== count) return;
      clearTimeout(timer); app.off("test-badge-applied", check); resolve();
    }
    app.on("test-badge-applied", check); check();
  }), count);
  const enterFleet = async page => {
    page.setDefaultTimeout(15_000);
    await page.getByTestId("thread-entry").first().click();
    await page.getByRole("button",{name:/현황판/}).click();
  };
  const other = {...item,id:"task:second",subject:{kind:"task",id:"second"},cause:"task_proposed",summary:"Task needs an answer"};
  answer = {result:{type:"attention",items:[item]}};
  await enterFleet(first);
  await waitBadge(1);
  answer = {result:{type:"attention",items:[item,other]}};
  await enterFleet(second);
  await waitBadge(2);
  // Only A opens the overview and receives this event. Its new snapshot still has one demand.
  answer = {result:{type:"attention",items:[{...item,id:"thread:revised",revision:"2",summary:"A refreshed demand"}]}};
  refreshOnOverview = true;
  await first.getByTestId("kernel-toggle").click();
  await first.getByTestId("kernel-attention").getByText("A refreshed demand").waitFor();
  await second.getByTestId("kernel-attention").getByText(other.summary).waitFor();
  await waitBadge(1);
  // Reload withdraws A's report without closing B; the previous valid count must return.
  await first.reload();
  await waitBadge(2);
  console.log("PASS same kernel: A=1, B=2, refreshed A=1, A detaches and B=2 is restored");
  answer = {result:{type:"attention",items:[]}};
  await enterFleet(first);
  gateway.sendKernelEvent({type:"resync"});
  for (const page of [first,second]) await page.getByTestId("kernel-attention").getByText("없음",{exact:true}).waitFor();
  const owner = await app.browserWindow(second);
  await owner.evaluate(window => { window.show(); window.focus(); });
  await second.waitForFunction(() => document.hasFocus());
  answer = {result:{type:"attention",items:[item]}};
  gateway.sendKernelEvent({type:"resync"});
  await second.getByTestId("kernel-attention").getByText(item.summary).waitFor();
  answer = {result:{type:"attention",items:[item,other]}};
  gateway.sendKernelEvent({type:"resync"});
  await second.getByText("새 응답 필요 항목 2개가 있습니다.").waitFor();
  await first.getByTestId("kernel-attention").getByText(item.summary).waitFor();
  assert.equal(await first.getByText("새 응답 필요 항목 2개가 있습니다.").count(),0);
  const reports = await app.evaluate(() => globalThis.badgeReports);
  if (process.platform === "win32") {
    assert.equal(new Set(reports.filter(r=>r[1]==="2 responses required" && r[2]).map(r=>r[0])).size,2);
    assert(!reports.some(r=>r[1]==="4 responses required"));
  } else assert.equal(reports.at(-1),2);
  await second.keyboard.press(process.platform === "darwin" ? "Meta+y" : "Control+y");
  await second.getByTestId("kernel-attention").waitFor({state:"detached"});
  await second.getByRole("button",{name:/현황판/}).click();
  answer = {error:{code:-32013,message:"synthetic read failure"}};
  gateway.sendKernelEvent({type:"resync"});
  for (const page of [first,second]) await page.getByTestId("kernel-attention").getByText("커널 확인 항목을 아직 읽지 못했습니다.").waitFor();
  await second.keyboard.press(process.platform === "darwin" ? "Meta+y" : "Control+y");
  await second.getByText("현재 저장소의 응답 필요 항목을 읽지 못했습니다.").waitFor();
  answer = {result:{type:"attention",items:[]}};
  gateway.sendKernelEvent({type:"resync"});
  await second.getByTestId("kernel-attention").getByText("없음",{exact:true}).waitFor();
  await second.keyboard.press(process.platform === "darwin" ? "Meta+y" : "Control+y");
  await second.getByText("응답이 필요한 스레드가 없습니다.").waitFor();
  const cleared = await app.evaluate(() => globalThis.badgeReports.at(-1));
  if (process.platform === "win32") assert.deepEqual(cleared.slice(1), ["",false]); else assert.equal(cleared,0);
  console.log("PASS native badge + focus owner across two windows, no double counting, next/empty/unavailable command");
} finally {
  await app.close();
  await gateway.close();
  await rm(profile,{recursive:true,force:true,maxRetries:5});
}
