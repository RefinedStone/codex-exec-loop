// A profile, not a window or a session, owns the personal view of a demand revision.
import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import electronPath from "electron";
import { _electron } from "playwright-core";
import { startFakeGateway, uniqueTestEndpoint, APPROVAL_THREAD_LIST_RECORDING, INTERRUPTED_TURN_RECORDING } from "../fake-gateway/fakeGateway.ts";

const desktop = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const profile = await mkdtemp(join(tmpdir(), "akra-attention-profile-"));
const endpoint = uniqueTestEndpoint();
const item = {id:"task:t:r1", revision:"r1", subject:{kind:"task", id:"t"}, cause:"task_proposed", summary:"Profile demand", next:[{step:"ask_user"}]};
let available = true;
const kernel = {kernel:"profile-repo", queries:{}, onRequest(method, params) {
  if (method === "akra/kernel/query" && params.query.type === "attention") return available
    ? {result:{type:"attention", items:[item]}} : {error:{code:-32013, message:"synthetic disconnection"}};
}};
const gateway = await startFakeGateway(endpoint, [APPROVAL_THREAD_LIST_RECORDING, INTERRUPTED_TURN_RECORDING], {kernel});
const env = {...process.env, AKRA_GATEWAY_ENDPOINT:endpoint, AKRA_DESKTOP_USER_DATA:profile, AKRA_DESKTOP_HIDDEN:"1"};
delete env.ELECTRON_RUN_AS_NODE;
const launch = () => _electron.launch({executablePath:electronPath, args:[desktop], env, timeout:30_000});
async function fleet(page) {
  page.setDefaultTimeout(20_000);
  await page.getByTestId("connection-phase").filter({hasText:"연결됨"}).waitFor();
  await page.getByTestId("thread-entry").first().click();
  await page.getByRole("button",{name:/현황판/}).click();
  return page.getByTestId("kernel-attention");
}
async function choices(view, labels) {
  for (const name of labels) await view.getByRole("button", {name, exact:true}).waitFor();
  assert.equal(await view.getByRole("button",{name:/Profile demand/}).count(), 1);
}
let app;
try {
  app = await launch();
  const first = await fleet(await app.firstWindow());
  const opening = app.waitForEvent("window");
  await app.evaluate(({ Menu }) => Menu.getApplicationMenu().getMenuItemById("new-window").click());
  const second = await fleet(await opening);
  await choices(second, ["읽음","고정","1시간 미루기"]);
  await first.getByRole("button", {name:"읽음",exact:true}).click();
  await second.getByRole("button", {name:"고정",exact:true}).click();
  await first.getByRole("button", {name:"1시간 미루기",exact:true}).click();
  const marked = ["안 읽음","고정 해제","미루기 해제"];
  await choices(first, marked);
  await choices(second, marked);
  await app.close();
  app = await launch();
  const restarted = await fleet(await app.firstWindow());
  await choices(restarted, marked);
  available = false;
  gateway.sendKernelEvent({type:"resync"});
  await restarted.getByText("커널 확인 항목을 아직 읽지 못했습니다.").waitFor();
  available = true;
  gateway.sendKernelEvent({type:"resync"});
  await choices(restarted, marked);
  item.id = "task:t:r2"; item.revision = "r2";
  gateway.sendKernelEvent({type:"resync"});
  await choices(restarted, ["읽음","고정","1시간 미루기"]);
  assert.equal(gateway.receivedMethods.includes("akra/kernel/command"), false);
  console.log("PASS separate windows, process restart, outage/recovery, new revision, no execution commands");
} finally {
  await app?.close();
  await gateway.close();
  await rm(profile,{recursive:true,force:true,maxRetries:5});
}
