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
let enabled = false;
let article;
let fail = true;
const commits = [1, 2, 3].map(n => ({sha: String(n).repeat(40), time: Math.floor(Date.now() / 1000) - n * 3600, subject: `feat: 작업 ${n}`}));
const STAGES = ["collect", "write", "review"];
// 단계 하나씩 커널의 변경 알림을 보낸다. 첫 실행은 초안 쓰기에서 실패하고, 이어서 하기는 그 단계부터 다시 한다.
function advance() {
  setTimeout(() => {
    if (!article.job) return;
    if (article.stage === "write" && fail) {
      fail = false;
      Object.assign(article, {state: "stopped", job: null, stopped: {reason: "failed", code: "model_timeout", message: "모델의 답이 10분을 넘었습니다."}});
    } else if (article.stage === "review") {
      Object.assign(article, {state: "reviewed", stage: null, job: null, calls: 2, review: {state: "reviewed", findings: []}});
    } else {
      if (article.stage === "write") Object.assign(article, {title: "리뷰를 글로 옮기기", draft: "# 리뷰를 글로 옮기기\n\n자동으로 쓴 **본문**", calls: 1});
      if (article.stage === "collect") article.evidence = {commits: commits.slice(0, 2).map((c, i) => ({id: `s${i + 1}`, sha: c.sha, subject: c.subject})), files: 4, summarized: 1};
      article.stage = STAGES[STAGES.indexOf(article.stage) + 1];
      advance();
    }
    article.revision++;
    gateway.sendExtensionChange(kernel);
  }, 150);
}
const gateway = await fake.startFakeGateway(endpoint, [fake.APPROVAL_THREAD_LIST_RECORDING, fake.INTERRUPTED_TURN_RECORDING], {
  kernel: {kernel, queries: {}, onRequest(method, params) {
    if (method === "akra/kernel/extensions/list") return {result:[{id:"akra.tech-blog",version:2,scope:"repository",enabled}]};
    if (method !== "akra/kernel/extensions/call") return;
    assert.equal(params.kernel,kernel);
    assert.equal(params.extension,"akra.tech-blog");
    assert.equal(params.version,2);
    const data = params.input;
    if (params.action === "catalog") return {result:{enabled}};
    if (params.action === "configure") { enabled = data.enabled; return {result:{enabled}}; }
    assert.ok(enabled);
    if (params.action === "targets") return {result:{commits, fresh: 2, selected: commits.slice(0, 2).map(c => c.sha), first: false, limit: 30, codex: true}};
    if (params.action === "list") return {result:article ? [{id:article.id,title:article.title,state:article.state,stage:article.stage,attempt:0,updated:article.updated}] : []};
    if (params.action === "generate") {
      assert.deepEqual(data.commits, commits.slice(0, 2).map(c => c.sha));
      assert.equal(data.topic, "");
      const now = Math.floor(Date.now() / 1000);
      article = {id:"one",version:2,title:"",topic:"",commits:data.commits,state:"running",stage:"collect",attempt:0,calls:0,
        evidence:null,draft:"",claims:[],figures:[],review:null,stopped:null,messages:[],revision:1,created:now,updated:now,job:{id:"job",started:now}};
      advance();
    } else if (params.action === "resume") {
      assert.equal(data.revision,article.revision);
      Object.assign(article, {state:"running", stopped:null, job:{id:"job2",started:Math.floor(Date.now() / 1000)}});
      article.revision++;
      advance();
    } else if (params.action !== "open") throw new Error(`unexpected action ${params.action}`);
    return {result:article};
  }},
});
const env = {...process.env, AKRA_GATEWAY_ENDPOINT:endpoint, AKRA_DESKTOP_HIDDEN:"1", AKRA_DESKTOP_USER_DATA:userData};
delete env.ELECTRON_RUN_AS_NODE;
let app;
try {
  app = await electron.launch({executablePath:createRequire(import.meta.url)("electron"),args:[root,...(process.env.CI?["--no-sandbox"]:[])],env});
  const page = await app.firstWindow({timeout: 30_000});
  page.setDefaultTimeout(15_000);
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
  await panel.getByRole("button",{name:"← 글 목록"}).click();
  await page.getByTestId("tech-blog-toggle").click();
  await page.getByTestId("tech-blog-toggle").click();
  await panel.getByRole("button",{name:/리뷰를 글로 옮기기/}).click();
  await panel.getByTestId("blog-result").waitFor();
  console.log("PASS one-press blog generation, failure, resume, preview and reopen");
} finally {
  if (app) await app.close();
  await gateway.close();
  await rm(userData,{recursive:true,force:true});
}
