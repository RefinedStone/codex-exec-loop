import assert from "node:assert/strict";
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
