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
let writes = 0;
const gateway = await fake.startFakeGateway(endpoint, [fake.APPROVAL_THREAD_LIST_RECORDING, fake.INTERRUPTED_TURN_RECORDING], {
  kernel: {kernel, queries: {}, onRequest(method, params) {
    if (method === "akra/kernel/extensions/list") return {result:[{id:"akra.tech-blog",version:1,scope:"repository",enabled}]};
    if (method !== "akra/kernel/extensions/call") return;
    assert.equal(params.kernel,kernel);
    assert.equal(params.extension,"akra.tech-blog");
    const data = params.input;
    if (params.action === "catalog") return {result:{enabled}};
    if (params.action === "configure") { enabled = data.enabled; return {result:{enabled}}; }
    assert.ok(enabled);
    if (params.action === "list") return {result:article ? [{id:article.id,topic:article.topic}] : []};
    if (params.action === "create") {
      article = {id:"one",topic:data.topic,revision:1,commit:"a".repeat(40),sources:[],draft:"",figures:[],messages:[],job:null};
    } else if (params.action === "write" || params.action === "review") {
      assert.equal(data.revision,article.revision);
      const phase = params.action;
      article.job = {id:"job",action:phase};
      article.revision++;
      setTimeout(() => {
        if (phase === "write") {
          writes++;
          article.proposal = {draft:`AI 원고 ${writes}`,reply:data.instruction};
          article.messages.push({role:"assistant",text:data.instruction});
        } else article.review = {state:"needs_evidence",findings:[{code:"figure_required"}]};
        article.job = null; article.revision++;
        gateway.sendExtensionChange(kernel);
      },150);
    } else if (params.action === "accept") {
      article.draft = article.proposal.draft; article.proposal = null; article.revision++;
    } else if (params.action === "save") {
      article.draft = data.draft; article.revision++;
    }
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
  await panel.getByLabel("글 주제").fill("리뷰를 글로 옮기기");
  await panel.getByRole("button",{name:"글 만들기"}).click();
  await panel.getByLabel("모델 ID").fill("test-model");
  await panel.getByRole("button",{name:"AI에 초안 요청"}).click();
  await panel.getByText("AI 원고 1",{exact:true}).waitFor();
  await panel.getByRole("button",{name:"제안을 원고에 적용"}).click();
  await panel.getByLabel("원고",{exact:true}).fill("내가 수정한 문단");
  await panel.getByRole("button",{name:"수정 저장"}).click();
  await panel.getByLabel("편집 요청").fill("사례를 추가해 주세요.");
  await panel.getByRole("button",{name:"AI에 수정 요청"}).click();
  await panel.getByText("AI 원고 2",{exact:true}).waitFor();
  assert.equal(await panel.getByLabel("원고",{exact:true}).inputValue(),"내가 수정한 문단");
  await panel.getByRole("button",{name:"독립 검수"}).click();
  await panel.getByText("자료 보완",{exact:true}).waitFor();
  await page.getByTestId("tech-blog-toggle").click();
  await page.getByTestId("tech-blog-toggle").click();
  await panel.getByLabel("글 선택").selectOption("one");
  await panel.getByText("AI 원고 2",{exact:true}).waitFor();
  assert.equal(await panel.getByLabel("원고",{exact:true}).inputValue(),"내가 수정한 문단");
  console.log("PASS repository plugin enable, draft, edit, feedback, review and reopen");
} finally {
  if (app) await app.close();
  await gateway.close();
  await rm(userData,{recursive:true,force:true});
}
