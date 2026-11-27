import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { _electron as electron } from "playwright-core";
import * as fake from "../fake-gateway/fakeGateway.ts";

const root = join(dirname(fileURLToPath(import.meta.url)), "../..");
const userData = await mkdtemp(join(tmpdir(), "akra-blog-e2e-"));
const kernel = "blog-test-repository";
const endpoint = fake.uniqueTestEndpoint();
const workspace = fake.loadReplayTable(fake.INTERRUPTED_TURN_RECORDING).get("thread/resume").result.thread.cwd;
let enabled = false;
let configured = 0;
let article;
let fail = true;
const commits = [1, 2, 3].map(n => ({sha: String(n).repeat(40), time: Math.floor(Date.now() / 1000) - n * 3600, subject: `feat: 작업 ${n}`}));
const STAGES = ["collect", "write", "review"];
let codex = true;
let seen = 0;
const otherWorkspace = "/work/blog-other", otherKernel = "blog-other-repository";
const requests = [];
let originAttachBarrier;
async function until(predicate, label) {
  const deadline = Date.now() + 15_000;
  while (!(await predicate())) {
    if (Date.now() > deadline) throw new Error("Timed out: " + label);
    await new Promise(resolve => setTimeout(resolve, 20));
  }
}
// 단계 하나씩 커널의 변경 알림을 보낸다. 첫 실행은 초안 쓰기에서 실패하고, 이어서 하기는 그 단계부터 다시 한다.
let hold = false;
function advance() {
  if (hold) return;
  setTimeout(() => {
    if (!article.job) return;
    if (article.task !== "generate") {
      if (article.stage === "revise") Object.assign(article, {stage: "review", draft: "# 짧게 고친 글\n\n두 번째 판", calls: article.calls + 1});
      else Object.assign(article, {state: "reviewed", stage: null, job: null, calls: article.calls + 1, review: {state: "reviewed", findings: []}, unread: true});
      if (article.job) advance();
    } else if (article.stage === "write" && fail) {
      fail = false;
      Object.assign(article, {state: "stopped", job: null, stopped: {reason: "failed", code: "model_timeout", message: "모델의 답이 10분을 넘었습니다."}});
    } else if (article.stage === "review") {
      Object.assign(article, {state: "reviewed", stage: null, job: null, calls: 2, review: {state: "reviewed", findings: []}, unread: true});
    } else {
      if (article.stage === "write") Object.assign(article, {title: "리뷰를 글로 옮기기", draft: "# 리뷰를 글로 옮기기\n\n자동으로 쓴 **본문**", calls: 1});
      if (article.stage === "collect") article.evidence = {commits: commits.slice(0, 2).map((c, i) => ({id: `s${i + 1}`, sha: c.sha, subject: c.subject})), files: 4, summarized: 1};
      article.stage = STAGES[STAGES.indexOf(article.stage) + 1];
      advance();
    }
    article.revision++;
    gateway.sendExtensionChange(kernel);
  }, 400);
}
const gateway = await fake.startFakeGateway(endpoint, [fake.APPROVAL_THREAD_LIST_RECORDING, fake.INTERRUPTED_TURN_RECORDING], {
  kernel: {kernel, queries: {}, onRequest(method, params) {
    requests.push({method, params:structuredClone(params)});
    if (method === "akra/kernel/attach") {
      assert.ok([workspace, otherWorkspace].includes(params.workspace));
      return {result:{client_id:"1",protocol_version:1,kernel:params.workspace === workspace ? kernel : otherKernel},
        replyAfter:params.workspace === workspace ? originAttachBarrier : undefined};
    }
    if (method === "akra/kernel/extensions/list") return {result:[{id:"akra.tech-blog",version:3,scope:"repository",enabled,revision:configured}]};
    if (method !== "akra/kernel/extensions/call") return;
    if (params.kernel === otherKernel) {
      if (params.action === "list") return {result:[]};
      if (params.action === "targets") return {result:{commits:[],fresh:0,selected:[],first:true,limit:30,codex}};
      throw new Error("article action reached the other repository: " + params.action);
    }
    assert.equal(params.kernel,kernel);
    assert.equal(params.extension,"akra.tech-blog");
    assert.equal(params.version,3);
    const data = params.input;
    if (params.action === "configure") {
      assert.equal(data.expected_revision, configured);
      enabled = data.enabled; configured++;
      setTimeout(() => gateway.sendExtensionChange(kernel), 0);
      return {result:{id:"akra.tech-blog",version:3,scope:"repository",enabled,revision:configured}};
    }
    assert.ok(enabled);
    if (params.action === "targets") return {result:{commits, fresh: 2, selected: commits.slice(0, 2).map(c => c.sha), first: false, limit: 30, codex}};
    if (params.action === "list") return {result:article ? [{id:article.id,title:article.title,state:article.state,stage:article.stage,task:article.task,updated:article.updated,unread:!!article.unread}] : []};
    if (params.action === "generate") {
      assert.deepEqual(data.commits, commits.slice(0, 2).map(c => c.sha));
      assert.equal(data.topic, "");
      const now = Math.floor(Date.now() / 1000);
      article = {id:"one",version:2,title:"",topic:"",commits:data.commits,state:"running",stage:"collect",attempt:0,calls:0,
        evidence:null,draft:"",claims:[],figures:[],review:null,stopped:null,messages:[],revision:1,created:now,updated:now,job:{id:"job",started:now},
        task:"generate",edition:1,previous:null,exported:null};
      advance();
    } else if (params.action === "resume") {
      assert.equal(data.revision,article.revision);
      Object.assign(article, {state:"running", stopped:null, job:{id:"job2",started:Math.floor(Date.now() / 1000)}});
      article.revision++;
      advance();
    } else if (params.action === "seen") {
      seen++;
      article.unread = false;
    } else if (params.action === "export") {
      assert.equal(data.revision,article.revision);
      article.exported = {path:"blog/2026-11-23-review-post/post.md", edition:article.edition};
      article.revision++;
    } else if (params.action === "revise" || params.action === "review") {
      assert.equal(data.revision,article.revision);
      if (params.action === "revise") {
        assert.equal(data.instruction, "더 짧게");
        article.previous = {draft: article.draft}; article.edition++;
      }
      Object.assign(article, {task:params.action, stage:params.action, state:"running", job:{id:"job3",started:Math.floor(Date.now() / 1000)}});
      article.revision++;
      advance();
    } else if (params.action === "restore") {
      const current = article.draft;
      Object.assign(article, {draft: article.previous.draft, previous: {draft: current}, edition: article.edition + 1});
      article.revision++;
    } else if (params.action === "save") {
      article.previous = {draft: article.draft}; article.edition++;
      Object.assign(article, {draft: data.draft, state: "edited", review: null});
      article.revision++;
    } else if (params.action !== "open") throw new Error(`unexpected action ${params.action}`);
    // 실제 호스트처럼 조회가 아닌 명령 뒤에는 변경 알림을 보낸다.
    if (params.action !== "open") setTimeout(() => gateway.sendExtensionChange(kernel), 0);
    // 버전 3의 모델 작업은 공통 호스트의 receipt를 돌려주고, 맡은 글은 progress로 알린다.
    if (["generate", "resume", "revise", "review"].includes(params.action))
      return {result:{id:`job-${params.action}`, extension:"akra.tech-blog", action:params.action, state:"running", progress:{article_id:article.id, stage:article.stage}}};
    return {result:article};
  }},
});
const env = {...process.env, AKRA_GATEWAY_ENDPOINT:endpoint, AKRA_DESKTOP_HIDDEN:"1", AKRA_DESKTOP_USER_DATA:userData};
delete env.ELECTRON_RUN_AS_NODE;
let app;
try {
  app = await electron.launch({executablePath:createRequire(import.meta.url)("electron"),args:[root,...(process.env.CI?["--no-sandbox"]:[])],env});
  await app.evaluate(({ipcMain}) => {
    globalThis.blogExportOpens = [];
    ipcMain.removeHandler("akra:git-open-file");
    ipcMain.handle("akra:git-open-file", (_event, cwd, path) => { globalThis.blogExportOpens.push({cwd, path}); });
  });
  const page = await app.firstWindow({timeout: 30_000});
  page.setDefaultTimeout(15_000);
  await page.evaluate(async () => {
    window.blogNotifications = [];
    class FixtureNotification {
      static permission = "granted";
      constructor(title, options) { this.title = title; this.options = options; window.blogNotifications.push(this); }
      close() {}
    }
    Object.defineProperty(window, "Notification", {value:FixtureNotification, configurable:true});
    await window.akra.settings.set({notifications:{mode:"always",sound:false}});
  });
  await page.getByTestId("connection-phase").filter({hasText:"연결됨"}).waitFor();
  await page.getByTestId("thread-entry").first().click();
  await app.evaluate(({Menu})=>Menu.getApplicationMenu().getMenuItemById("open-settings").click());
  const row = page.getByTestId("settings-row-akra.tech-blog");
  await row.getByRole("button",{name:"켜기"}).click();
  await row.getByRole("button",{name:"열기"}).click();
  const panel = page.getByRole("region",{name:"기술 블로그"});
  await assert.rejects(panel.getByLabel("모델 ID").waitFor({timeout: 500}));
  await panel.getByText("2개 중 2개 고름").waitFor();
  await panel.getByRole("button",{name:"글 만들기"}).click();
  await panel.getByText("요약만 넣은 파일 1개").waitFor();
  await panel.getByText("모델의 답이 10분을 넘었습니다.").waitFor();
  await panel.getByRole("button",{name:"이어서 하기"}).click();
  await panel.getByTestId("blog-result").waitFor();
  await panel.getByText("검수 통과").waitFor();
  await panel.getByText("모델 호출 2회", {exact: false}).waitFor();
  assert.equal(await panel.locator(".blog-preview strong").textContent(), "본문");
  await panel.getByRole("button",{name:"원문"}).click();
  assert.match(await panel.locator(".blog-preview pre").textContent(), /\*\*본문\*\*/);
  await panel.getByRole("button",{name:"파일로 내보내기"}).click();
  await panel.getByText("blog/2026-11-23-review-post/post.md").waitFor();
  const exportedPath = article.exported.path;
  await panel.getByRole("button",{name:"편집기로 열기"}).click();
  const openDeadline = Date.now() + 15_000;
  while ((await app.evaluate(() => globalThis.blogExportOpens.length)) === 0) {
    if (Date.now() > openDeadline) throw new Error("exported article open did not reach the main process");
    await new Promise(resolve => setTimeout(resolve, 20));
  }
  assert.deepEqual(await app.evaluate(() => globalThis.blogExportOpens), [{cwd:workspace, path:exportedPath}]);
  await panel.getByRole("button",{name:"다시 쓰기…"}).click();
  await panel.getByRole("button",{name:"더 짧게"}).click();
  await panel.getByRole("button",{name:"다시 쓰기",exact:true}).click();
  await panel.getByText("판 2").waitFor();
  await panel.getByText("내보낸 파일보다 새 판입니다.").waitFor();
  assert.equal(await panel.getByRole("button",{name:"편집기로 열기"}).count(), 0, "a stale export does not open as the current edition");
  assert.equal(await panel.locator(".blog-preview h1").textContent(), "짧게 고친 글");
  await panel.getByRole("button",{name:"이전 판으로"}).click();
  await panel.getByText("판 3").waitFor();
  assert.equal(await panel.locator(".blog-preview h1").textContent(), "리뷰를 글로 옮기기");
  await panel.getByRole("button",{name:"직접 고치기"}).click();
  await panel.getByLabel("원고",{exact:true}).fill("# 사람이 고친 글");
  await panel.getByRole("button",{name:"저장"}).click();
  await panel.getByText("검수 안 함").waitFor();
  await panel.getByLabel("더 보기").click();
  await panel.getByRole("button",{name:/검수하기/}).click();
  await panel.getByText("검수 통과").waitFor();
  await panel.getByRole("button",{name:"← 글 목록"}).click();
  await page.getByTestId("tech-blog-toggle").click();
  await page.getByTestId("tech-blog-toggle").click();
  await panel.getByRole("button",{name:/리뷰를 글로 옮기기/}).click();
  await panel.getByTestId("blog-result").waitFor();
  await panel.getByRole("button",{name:"다시 쓰기…"}).click();
  await panel.getByRole("button",{name:"더 짧게"}).click();
  await page.evaluate(() => { window.blogNotifications = []; });
  hold = true;
  await panel.getByRole("button",{name:"다시 쓰기",exact:true}).click();
  await page.getByTestId("tech-blog-toggle").click();
  await page.getByTestId("blog-activity-running").waitFor();
  assert.equal(await page.getByTestId("tech-blog-toggle").getAttribute("title"), "기술 블로그 · 글을 만드는 중");
  hold = false;
  advance();
  await page.getByText("블로그 글이 준비됐습니다.").waitFor();
  await page.getByTestId("blog-activity-unread").waitFor();
  assert.equal(await page.getByTestId("tech-blog-toggle").getAttribute("title"), "기술 블로그 · 새 글 1개");
  const seenBefore = seen;
  await page.getByTestId("tech-blog-toggle").click();
  await panel.getByTestId("blog-result").waitFor();
  await page.getByTestId("blog-activity-unread").waitFor({state: "detached"});
  assert.equal(seen, seenBefore + 1);
  await until(() => page.evaluate(() => window.blogNotifications.some(notice => notice.options.tag === "akra:blog:one")), "the completed article notification");
  await app.evaluate(({ipcMain}, cwd) => {
    ipcMain.removeHandler("akra:choose-folder");
    ipcMain.handle("akra:choose-folder", () => cwd);
  }, otherWorkspace);
  await page.getByTestId("new-folder-thread").click();
  await page.getByTestId("home").locator(".home-pickers button").first().click();
  await page.getByRole("option",{name:"폴더 고르기…",exact:true}).click();
  await until(() => requests.some(request => request.method === "akra/kernel/extensions/list" && request.params.kernel === otherKernel), "the other repository catalog");
  await panel.getByRole("heading",{name:"새 글",exact:true}).waitFor();
  const attachedBefore = requests.filter(request => request.method === "akra/kernel/attach" && request.params.workspace === workspace).length;
  const openedBefore = requests.filter(request => request.method === "akra/kernel/extensions/call" && request.params.kernel === kernel && request.params.action === "open").length;
  let releaseAttach;
  originAttachBarrier = new Promise(resolve => { releaseAttach = resolve; });
  try {
    await page.evaluate(() => window.blogNotifications.find(notice => notice.options.tag === "akra:blog:one").onclick());
    await until(() => requests.filter(request => request.method === "akra/kernel/attach" && request.params.workspace === workspace).length > attachedBefore, "the original repository attach waiting for its reply");
    assert.equal(requests.filter(request => request.method === "akra/kernel/extensions/call" && request.params.kernel === otherKernel && ["open","seen"].includes(request.params.action)).length, 0);
  } finally { originAttachBarrier = undefined; releaseAttach(); }
  try { await panel.getByTestId("blog-result").waitFor(); }
  catch (error) {
    console.error(JSON.stringify({phase:"notification return", requests:requests.slice(-35), panel:await panel.innerText().catch(() => "not mounted")}));
    throw error;
  }
  assert.equal(await panel.locator(".blog-preview h1").textContent(), "짧게 고친 글");
  assert.ok(requests.filter(request => request.method === "akra/kernel/extensions/call" && request.params.kernel === kernel && request.params.action === "open").length > openedBefore);
  assert.equal(requests.filter(request => request.method === "akra/kernel/attach").at(-1).params.workspace, workspace);
  assert.equal(requests.filter(request => request.method === "akra/kernel/extensions/call" && request.params.kernel === otherKernel && ["open","seen"].includes(request.params.action)).length, 0);
  await panel.getByRole("button",{name:"← 글 목록"}).click();
  codex = false;
  await page.getByTestId("tech-blog-toggle").click();
  await page.getByTestId("tech-blog-toggle").click();
  await panel.getByTestId("blog-setup").waitFor();
  await panel.getByRole("button",{name:/리뷰를 글로 옮기기/}).click();
  await panel.getByTestId("blog-result").waitFor();
  assert.equal(await app.evaluate(() => globalThis.blogExportOpens.length), 1, "only an explicit export-open click opens a file");
  assert.ok(!gateway.receivedMethods.includes("turn/start") && !gateway.receivedMethods.includes("turn/steer"));
  console.log("PASS one-press blog generation, failure, resume, export with exact workspace/path IPC, rewrite, restore, edit, review and reopen; saved articles stay open without Codex CLI; closed-panel unread and cross-repository notification focus survive a delayed attach; no model calls");
} finally {
  if (app) await app.close();
  await gateway.close();
  await rm(userData,{recursive:true,force:true});
}
