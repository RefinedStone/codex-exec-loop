// Actual Electron renderer, an isolated local profile, and a synthetic kernel.
import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import electronPath from "electron";
import { _electron } from "playwright-core";
import { startFakeGateway, uniqueTestEndpoint, APPROVAL_THREAD_LIST_RECORDING, APPROVAL_TURN_RECORDING } from "../fake-gateway/fakeGateway.ts";

const desktop = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const userData = await mkdtemp(join(tmpdir(), "akra-attention-"));
const endpoint = uniqueTestEndpoint();
const task = { task_id: "task-17", fields: { title: "Investigate frozen source", status: "proposed", acceptance: [] }, updated_at: "now" };
const item = { id: "task:task-17:r1", revision: "r1", subject: {kind: "task", id: task.task_id},
  cause: "task_proposed", summary: "Kernel-supplied reason", next: [{step: "query", name: "planning"}, {step: "ask_user"}] };
let answer = {result: {type: "attention", items: [item]}};
const secondRepo = "/work/attention-other";
const issue = {provider:"github", server_id:"github.com", repository_id:"repo", issue_id:"issue-1", project_scope:{workspace_dir:"/work/example-app"}};
let attached;
const kernel = {kernel: "attention-repository", queries: {}, onRequest(method, params) {
  if (method === "akra/kernel/attach") {
    attached = params.workspace;
    return {result:{client_id:"1", protocol_version:1, kernel: attached === secondRepo ? "second-kernel" : kernel.kernel}};
  }
  if (method !== "akra/kernel/query") return;
  if (params.query.type === "attention") return attached === secondRepo ? {result:{type:"attention",items:[{...item,summary:"Second repository demand"}]}} : answer;
  if (params.query.type === "list_tasks") return {result: {type:"list_tasks", available:true, planning_revision: 3, tasks:[task]}};
}};
const listing = join(userData, "two-repo-list.jsonl");
const lines = (await readFile(APPROVAL_THREAD_LIST_RECORDING, "utf8")).trim().split("\n").map(line => JSON.parse(line));
const listReply = lines.at(-1);
const envelope = JSON.parse(listReply.text);
const firstThread = envelope.result.data[0];
envelope.result.data.push({...firstThread, id:"other-thread", cwd:secondRepo, updatedAt:1});
listReply.text = JSON.stringify(envelope);
await writeFile(listing, lines.map(line => JSON.stringify(line)).join("\n") + "\n");
const gateway = await startFakeGateway(endpoint, [listing, APPROVAL_TURN_RECORDING], {kernel});
const env = {...process.env, AKRA_GATEWAY_ENDPOINT: endpoint, AKRA_DESKTOP_HIDDEN: "1", AKRA_DESKTOP_USER_DATA: userData};
delete env.ELECTRON_RUN_AS_NODE;
const app = await _electron.launch({executablePath: electronPath, args:[desktop], env, timeout:30_000});
try {
  const page = await app.firstWindow();
  page.setDefaultTimeout(20_000);
  await page.getByTestId("connection-phase").filter({hasText:"연결됨"}).waitFor();
  await page.getByTestId("thread-entry").first().click();
  await page.getByTestId("approval").waitFor();
  await page.getByRole("button", {name:/현황판/}).click();
  const fleet = page.getByTestId("kernel-attention");
  const row = fleet.getByRole("button", {name:/Kernel-supplied reason/});
  await row.waitFor();
  assert.match(await row.innerText(), /planning → ask_user/);
  await row.click();
  await page.getByTestId("task-editor").waitFor();
  await page.waitForFunction(id => document.querySelector('[data-testid="task-editor-select"]')?.value === id, task.task_id);
  await page.getByRole("button", {name: "작업 편집 닫기"}).click();
  answer = {error: {code:-32013, message:"synthetic authority disconnected"}};
  gateway.sendKernelEvent({type:"resync"});
  await fleet.getByText("커널 확인 항목을 아직 읽지 못했습니다.").waitFor();
  assert.equal(await row.count(), 0);
  answer = {result: {type:"attention", items:[]}};
  gateway.sendKernelEvent({type:"resync"});
  await fleet.getByText("없음", {exact:true}).waitFor();
  answer = {result:{type:"attention",items:[
    {...item,id:"old-failure",cause:"turn_failed",subject:{kind:"thread",id:firstThread.id},summary:"Earlier kernel turn failed"},
    {...item,id:"decision:1",cause:"issue_decision_pending",subject:{kind:"issue_decision",id:"decision-1",issue},summary:"Choose the next step"},
  ]}};
  gateway.sendKernelEvent({type:"resync"});
  await fleet.getByText("Earlier kernel turn failed").waitFor();
  await page.getByTestId("fleet-card").filter({hasText:firstThread.name}).getByRole("img",{name:"승인 대기"}).waitFor();
  await fleet.getByRole("button",{name:/Choose the next step/}).click();
  await page.getByTestId("attention-workspace").selectOption(secondRepo);
  await fleet.getByText("Second repository demand").waitFor();
  assert.equal(await fleet.getByText("Earlier kernel turn failed").count(),0);
  console.log("PASS kernel cause/next, exact task navigation, unavailable versus confirmed empty");
} catch (error) {
  const page = app.windows()[0];
  if (page) console.error("Synthetic fleet state:", await page.locator("body").innerText({timeout:3000}));
  throw error;
} finally {
  await app.close();
  await gateway.close();
  await rm(userData, {recursive:true, force:true, maxRetries:5});
}
