import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import electronPath from "electron";
import { _electron } from "playwright-core";
import { startFakeGateway, uniqueTestEndpoint, APPROVAL_THREAD_LIST_RECORDING, INTERRUPTED_TURN_RECORDING } from "../fake-gateway/fakeGateway.ts";

const root = await mkdtemp(join(tmpdir(), "akra-fleet-"));
const [a,b] = await Promise.all(["first", "second"].map(async name => {
  const path = join(root, name);
  await promisify(execFile)("git", ["init","-q",path], {windowsHide:true,env:{...process.env,GIT_MASTER:"1"}});
  return path;
}));
const records = (await readFile(APPROVAL_THREAD_LIST_RECORDING,"utf8")).trim().split("\n").map(line => JSON.parse(line));
const reply = JSON.parse(records.at(-1).text);
const thread = reply.result.data[0];
reply.result.data = [{...thread,cwd:a},{...thread,cwd:b,id:"other-thread"}];
records.at(-1).text = JSON.stringify(reply);
const listing = join(root,"listing.jsonl");
await writeFile(listing,records.map(record => JSON.stringify(record)).join("\n")+"\n");
const item = path => ({id:"task:shared:r1", revision:"r1", subject:{kind:"task",id:"shared"},cause:"task_proposed",
  summary:path === a ? "First repository demand" : "Second repository demand", next:[{step:"ask_user"}]});
const key = path => path === a ? "kernel-A" : "kernel-B";
let active, failed = false;
const attaches = [];
const endpoint = uniqueTestEndpoint();
const gateway = await startFakeGateway(endpoint,[listing,INTERRUPTED_TURN_RECORDING],{kernel:{kernel:"fleet",queries:{},onRequest(method,params) {
  if (method === "akra/kernel/attach") {
    active = params.workspace; attaches.push(active);
    return {result:{protocol_version:1,client_id:"1",kernel:key(active)}};
  }
  if (method === "akra/kernel/fleet_attention") {
    assert([a,b].includes(params.workspace));
    return failed && params.workspace === b ? {error:{code:-32013,message:"unavailable"}} :
      {result:{kernel:key(params.workspace),items:[item(params.workspace)]}};
  }
  if (params.query?.type === "attention") return {result:{type:"attention",items:[item(active)]}};
  if (params.query?.type === "list_tasks") return {result:{type:"list_tasks",available:true,planning_revision:1,tasks:[
    {task_id:"shared",fields:{title:item(active).summary,status:"proposed",acceptance:[]},updated_at:"now"}]}};
}}});
const desktop = join(dirname(fileURLToPath(import.meta.url)), "..","..");
const env = {...process.env,AKRA_GATEWAY_ENDPOINT:endpoint,AKRA_DESKTOP_HIDDEN:"1",AKRA_DESKTOP_USER_DATA:join(root,"profile")};
delete env.ELECTRON_RUN_AS_NODE;
const app = await _electron.launch({executablePath:electronPath,args:[desktop],env,timeout:30_000});
try {
  const page = await app.firstWindow();
  page.setDefaultTimeout(15_000);
  await page.getByTestId("thread-entry").first().waitFor();
  await page.getByRole("button",{name:/현황판/}).click();
  await page.getByTestId("attention-workspace").selectOption(a);
  const selected = page.getByTestId("kernel-attention");
  const other = page.getByTestId("repository-attention");
  const total = page.getByTestId("fleet-attention-total");
  await selected.getByText(item(a).summary).waitFor();
  await page.getByRole("button",{name:"모든 저장소 확인"}).click();
  await total.getByText("확인된 응답 필요 2개 · 미확인 폴더 0개").waitFor();
  await other.getByRole("button",{name:"읽음",exact:true}).click();
  await selected.getByRole("button",{name:"읽음",exact:true}).waitFor();
  await other.getByRole("button",{name:"안 읽음",exact:true}).waitFor();
  failed = true;
  await page.getByRole("button",{name:"다른 저장소 새로 읽기"}).click();
  await total.getByText("확인된 응답 필요 1개 · 미확인 폴더 1개").waitFor();
  await other.getByText("이 저장소의 응답 필요 항목은 확인되지 않았습니다.").waitFor();
  await selected.getByText(item(a).summary).waitFor();
  assert.deepEqual(attaches,[a],"background reads never attach another kernel");
  failed = false;
  await page.getByRole("button",{name:"다른 저장소 새로 읽기"}).click();
  await other.getByRole("button",{name:/Second repository demand/}).click();
  await page.getByTestId("task-editor-select").waitFor();
  await page.waitForFunction(() => document.querySelector('[data-testid="task-editor-select"]')?.value === "shared");
  assert.equal(await page.getByTestId("task-editor").locator("input").evaluateAll(nodes=>nodes.some(n=>n.value === "Second repository demand")),true);
  assert.deepEqual(attaches,[a,b]);
  await page.getByRole("button",{name:"작업 편집 닫기"}).click();
  await selected.getByRole("button",{name:"안 읽음",exact:true}).waitFor();
  await other.getByRole("button",{name:"읽음",exact:true}).waitFor();
  assert.equal(gateway.receivedMethods.some(method=>["akra/kernel/command","turn/start","turn/steer"].includes(method)),false);
  console.log("PASS same item/subject/revision in two disposable repos: independent counts and preferences; partial failure, recovery and correct scoped task navigation");
} catch (error) {
  console.error("Synthetic fleet:", await app.windows()[0]?.locator("body").innerText({timeout:3000}));
  throw error;
} finally {
  await app.close();
  await gateway.close();
  await rm(root,{recursive:true,force:true,maxRetries:5});
}
