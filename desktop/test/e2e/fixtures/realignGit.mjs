// Only the thread catalog is fake. Git RPC envelopes go to the Rust test's real graph and journal services.
import assert from "node:assert/strict";
import {execFileSync} from "node:child_process";
import {readFileSync,writeFileSync} from "node:fs";
import {resolve,join} from "node:path";
import {createRequire} from "node:module";
import {createInterface} from "node:readline";
import {_electron as electron} from "playwright-core";
import * as fake from "../../fake-gateway/fakeGateway.ts";
const root=process.argv[2],repo=join(root,"repo"),desktop=resolve(".");
const git=(...a)=>execFileSync("git",["-c","user.name=QA","-c","user.email=qa@example.invalid",...a],{cwd:repo,windowsHide:true,encoding:"utf8"}).trim();
const commit=(name,body)=>{writeFileSync(join(repo,name),body);git("add",name);git("commit","-qm",body);return git("rev-parse","HEAD");};
git("init","-qb","main");git("config","user.name","QA");git("config","user.email","qa@example.invalid");commit("base","old base");git("switch","-qc","feature");const feature=commit("mine","my patch");git("switch","-q","main");const base=commit("base","replacement base");git("switch","-q","feature");const initial=git("show-ref");
let next=0;const pending=new Map(),plans=[],executes=[];
const input=createInterface({input:process.stdin});input.on("line",line=>{const {id,result,error}=JSON.parse(line);const slot=pending.get(id);if(!slot)return;pending.delete(id);if(error)slot.reject(Error(error.message));else slot.resolve(result);});
const rpc=(name,params)=>new Promise((resolve,reject)=>{const id=++next;pending.set(id,{resolve,reject});process.stdout.write(JSON.stringify({id,method:"akra/git/"+name,params})+"\n");});
const handlers=Object.fromEntries(["graph","refs","commit","plan","execute","operations","undo"].map(name=>["git"+name[0].toUpperCase()+name.slice(1),params=>{if(name==="plan")plans.push(params);if(name==="execute")executes.push(params);return rpc(name,params);} ]));
const recordings=[fake.APPROVAL_THREAD_LIST_RECORDING,fake.INTERRUPTED_TURN_RECORDING].map((source,i)=>{const file=join(root,"recording"+i+".jsonl");writeFileSync(file,readFileSync(source,"utf8").replaceAll("/work/example-app",repo.replaceAll("\\","/")));return file;});
const endpoint=fake.uniqueTestEndpoint();const gateway=await fake.startFakeGateway(endpoint,recordings,handlers);
const env={...process.env,AKRA_GATEWAY_ENDPOINT:endpoint,AKRA_DESKTOP_HIDDEN:"1",AKRA_DESKTOP_USER_DATA:join(root,"profile")};delete env.ELECTRON_RUN_AS_NODE;
let app;try{
 app=await electron.launch({executablePath:createRequire(import.meta.url)("electron"),args:[desktop],env});const p=await app.firstWindow();p.setDefaultTimeout(16000);
 await p.getByTestId("connection-phase").filter({hasText:"연결됨"}).waitFor();await p.getByTestId("thread-entry").first().click();await p.getByTestId("git-graph-toggle").click();
 const row=(sha)=>p.locator('[data-commit="'+sha+'"]');await row(feature).getByRole("checkbox").check();
 const source=row(feature).getByTestId("git-graph-ref").filter({hasText:"feature"});await source.dragTo(row(base));
 const plan=p.getByTestId("git-plan");await plan.filter({hasText:"로컬만 변경"}).waitFor();assert.equal(git("show-ref"),initial);assert.equal(executes.length,0);const drag=plans.at(-1).operation;
 await p.getByTestId("git-plan-cancel").click();const keyboard=p.getByRole("button",{name:"이 커밋을 새 base로 계획"});await keyboard.focus();await p.keyboard.press("Enter");await plan.waitFor();assert.deepEqual(plans.at(-1).operation,drag);assert.deepEqual(drag,{kind:"realign",base,commits:[feature]});assert.equal(executes.length,0);
 await plan.getByRole("button",{name:"확인: 로컬 묶음 실행"}).click();await p.getByTestId("git-operation-undo").waitFor();assert.equal(executes.length,1);assert.equal(git("rev-parse","HEAD^"),base);assert.equal(readFileSync(join(repo,"mine"),"utf8"),"my patch");assert.equal(git("rev-parse","main"),base);
 await p.getByTestId("git-operation-undo").click();await p.getByTestId("git-operation-notice").filter({hasText:"되돌리기"}).waitFor();assert.equal(git("show-ref"),initial);assert(!gateway.receivedMethods.includes("turn/start"));console.error("PASS real Electron drag and keyboard preview, explicit execution, real Git grouped replay and all-ref undo");
}finally{if(app)await app.close();await gateway.close();input.close();process.stdin.unref();}
