import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { setTimeout as delay } from "node:timers/promises";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { runStreamingCommand } from "./ci-process-progress.mjs";
import { createContractProgressWriter, runNativeContracts } from "./run_native_rust_contracts.mjs";

test("stdout and stderr arrive before the process can complete", async () => {
  const controller = new AbortController();
  let observed = "";
  let errors = "";
  const result = await runStreamingCommand(process.execPath, ["-e",
    "console.error('diagnostic'); console.log('ready'); setInterval(()=>{},1000);"], {
    signal:controller.signal,
    write(chunk) { observed += chunk; if (observed.includes("ready")) controller.abort(); },
    writeError(chunk) { errors += chunk; },
  });
  assert.match(observed,/ready/);
  assert.match(errors,/diagnostic/);
  assert.equal(result.aborted,true);
  assert.notEqual(result.status,0);
});
test("nonzero exits and spawn failures are not promoted to success", async () => {
  const result = await runStreamingCommand(process.execPath, ["-e","console.log('partial'); process.exitCode=7;"], {write(){},writeError(){}});
  assert.equal(result.status,7);
  assert.match(result.stdout,/partial/);
  await assert.rejects(runStreamingCommand("akra-command-that-does-not-exist",[],{write(){},writeError(){}}),/ENOENT/);
});
test("progress is appended before the contract ends and retains partial runs", () => {
  const dir=mkdtempSync(join(tmpdir(),"akra-progress-"));
  try {
    const file=join(dir,"nested","progress.jsonl");
    const output=[];
    const record=createContractProgressWriter({cwd:dir,env:{AKRA_NATIVE_PROGRESS_FILE:file,GITHUB_SHA:"source-sha"},write:(s)=>output.push(s)});
    record({phase:"started",filter:"one",index:1,total:2});
    const partial=readFileSync(file,"utf8").trim().split("\n").map(JSON.parse);
    assert.equal(partial.length,1);
    assert.equal(partial[0].phase,"started");
    assert.equal(partial[0].sourceSha,"source-sha");
    record({phase:"finished",filter:"one",status:"passed",durationMs:12});
    assert.equal(readFileSync(file,"utf8").trim().split("\n").length,2);
    assert.ok(output.every((s)=>s.startsWith("AKRA_NATIVE_PROGRESS ")));
  } finally { rmSync(dir,{recursive:true,force:true}); }
});
test("a cancelled contract records failure and never starts the next filter", async () => {
  const records=[];
  const run=(command,args)=>command==="cargo"
    ? {status:0,stdout:JSON.stringify({reason:"compiler-artifact",target:{name:"akra_ade",kind:["lib"]},profile:{test:true},executable:"fake"})+"\n"}
    : {status:0,stdout:"one::test: test\ntwo::test: test\n"};
  await assert.rejects(runNativeContracts(["--","one::","two::"],{
    run,write(){},record:(e)=>records.push(e),
    stream:async()=>({status:null,signal:"SIGTERM",aborted:true,stdout:"partial"}),
  }),/SIGTERM/);
  assert.deepEqual(records.map((e)=>[e.filter,e.phase,e.status]),[
    ["one::","started",undefined],["one::","finished","cancelled"],
  ]);
});

function fixtureProcessIsAlive(pid) {
  try { process.kill(pid, 0); }
  catch (error) { if (error.code === "ESRCH") return false; throw error; }
  if (process.platform === "linux") {
    try {
      const stat = readFileSync("/proc/" + pid + "/stat", "utf8");
      if (stat.slice(stat.lastIndexOf(")") + 1).trimStart().startsWith("Z ")) return false;
    } catch (error) { if (error.code === "ENOENT") return false; throw error; }
  }
  return true;
}

function killFixtureProcess(pid) {
  if (!pid || !fixtureProcessIsAlive(pid)) return;
  try { process.kill(pid, "SIGKILL"); }
  catch (error) { if (error.code !== "ESRCH") throw error; }
}

test("cancellation terminates the owned child and its pipe-inheriting grandchild", async () => {
  const controller = new AbortController();
  let observed = "";
  let grandchildPid;
  const fixture = [
    "const {spawn}=require('node:child_process');",
    "const grandchild=spawn(process.execPath,['-e',\"process.on('SIGTERM',()=>{});setInterval(()=>{},1000);\"],{stdio:['ignore','inherit','inherit']});",
    "console.log('GRANDCHILD:'+grandchild.pid);setInterval(()=>{},1000);",
  ].join("");
  const fallback = setTimeout(() => controller.abort(), 10000);
  try {
    const result = await runStreamingCommand(process.execPath, ["-e", fixture], {
      signal:controller.signal,
      write(chunk) {
        observed += chunk;
        const match = observed.match(/GRANDCHILD:(\d+)/);
        if (match) { grandchildPid = Number(match[1]); controller.abort(); }
      },
      writeError() {},
    });
    assert.ok(grandchildPid, "the fixture must report its own descendant PID");
    assert.equal(result.aborted, true);
    assert.notEqual(result.status, 0);
    const deadline = Date.now() + 2000;
    while (fixtureProcessIsAlive(grandchildPid) && Date.now() < deadline) await delay(20);
    assert.equal(fixtureProcessIsAlive(grandchildPid), false, "the owned descendant must be terminated");
  } finally {
    clearTimeout(fallback);
    killFixtureProcess(grandchildPid);
  }
});

test("an escaped writer after leader exit fails cleanup within the cancellation bound", async () => {
  const controller = new AbortController();
  let leaderExited = false;
  let observed = "";
  let grandchildPid;
  let abortedAt;
  const abortWhenReady = () => {
    if (leaderExited && grandchildPid && !controller.signal.aborted) {
      abortedAt = Date.now();
      controller.abort();
    }
  };
  const fixture = [
    "const {spawn}=require('node:child_process');",
    "const grandchild=spawn(process.execPath,['-e','setInterval(()=>{},1000);'],{detached:true,stdio:['ignore','inherit','inherit']});",
    "grandchild.unref();process.stdout.write('GRANDCHILD:'+grandchild.pid+'\\n',()=>process.exit(0));",
  ].join("");
  const fallback = setTimeout(() => controller.abort(), 10000);
  try {
    await assert.rejects(runStreamingCommand(process.execPath, ["-e", fixture], {
      signal:controller.signal, terminationGraceMs:100, drainTimeoutMs:100,
      spawnProcess(command, args, options) {
        const child = spawn(command, args, options);
        child.once("exit", () => queueMicrotask(() => { leaderExited = true; abortWhenReady(); }));
        return child;
      },
      write(chunk) {
        observed += chunk;
        const match = observed.match(/GRANDCHILD:(\d+)/);
        if (match) { grandchildPid = Number(match[1]); abortWhenReady(); }
      },
      writeError() {},
    }), /Process tree cleanup failed:/);
    assert.ok(grandchildPid && abortedAt, "the fixture must reach the exited-leader boundary");
    assert.ok(Date.now() - abortedAt < 3000, "an inherited pipe cannot hold the supervisor indefinitely");
  } finally {
    clearTimeout(fallback);
    killFixtureProcess(grandchildPid);
  }
});
