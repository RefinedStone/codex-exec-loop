#!/usr/bin/env node
import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { runNativeContracts } from "./run_native_rust_contracts.mjs";

const directory = mkdtempSync(join(tmpdir(),"akra-contract-targets-"));
const reportFile = resolve("artifacts/ci-contract-targets/fixture.json");
const env = {...process.env, CARGO_TARGET_DIR:join(directory,"target"), CARGO_NET_OFFLINE:"true"};
const put = (path, source) => {
  const absolute = join(directory,path); mkdirSync(resolve(absolute,".."),{recursive:true}); writeFileSync(absolute,source);
};
try {
  put("Cargo.toml",'[package]\nname="akra-ade"\nversion="0.0.0"\nedition="2024"\n[lib]\nname="shared"\n[workspace]\nmembers=["feature"]\nresolver="3"\n');
  put("src/lib.rs",'#[cfg(test)] mod tests { #[test] fn root_contract() {} }\n');
  put("feature/Cargo.toml",'[package]\nname="feature"\nversion="0.0.0"\nedition="2024"\n[lib]\nname="shared"\n');
  const fence=String.fromCharCode(96).repeat(3);
  put("feature/src/lib.rs",[
    "/// "+fence,"/// assert_eq!(shared::two(), 2);","/// "+fence,
    "pub fn two() -> u8 {2}",
    "#[cfg(test)] mod native {",
    "#[test] fn contract() {}",
    "#[test] fn one() {}",
    "#[test] fn one_extra() {}",
    '#[test] #[ignore] fn helper() { panic!("ignored helper must remain ignored"); }',
    "}"
  ].join("\n"));
  put("feature/tests/fence.rs","#[test] fn integration_contract() {}\n");
  const ownership={schemaVersion:1,rootPackage:"akra-ade",packages:[
    {name:"akra-ade",state:"active"},{name:"feature",state:"active"}],
    transfers:[{owner:"feature",oldTestPrefix:"old::native::",newTestPrefix:"native::"}]};
  put("scripts/ci-crate-ownership.json",JSON.stringify(ownership));
  const lock=spawnSync("cargo",["generate-lockfile","--offline"],{cwd:directory,env,encoding:"utf8"});
  assert.equal(lock.status,0,lock.stderr);
  const records=[];const output=[];
  const result=await runNativeContracts(["--include-members","--","root_contract","old::native::contract"],{
    cwd:directory,env,record:(event)=>records.push(event),write:(text)=>output.push(text),
  });
  assert.deepEqual(result,{preparedTargets:3,executedGroups:4,executedTests:5});
  assert.equal(records.filter((event)=>event.contractId==="member-doctest"&&event.status==="passed").length,1);
  assert.ok(!output.join("").includes("ignored helper must remain ignored"));
  const explicit=await runNativeContracts(["--package","feature","--target-kind","test","--target-name","fence","--","integration_contract"],{
    cwd:directory,env,record(){},write(){},
  });
  assert.equal(explicit.executedTests,1);
  const report={schemaVersion:1,capturedAt:new Date().toISOString(),platform:process.platform,
    result,explicit,resultRecords:records,
    checks:["same-name targets in different packages","moved filter inventory","non-overlapping member remainder",
      "ignored helper not enabled","integration target executed","doctest executed","explicit package/integration target"]};
  mkdirSync(resolve(reportFile,".."),{recursive:true});
  writeFileSync(reportFile,JSON.stringify(report,null,2)+"\n");
  console.log(JSON.stringify({result,explicit,reportFile}));
} finally {
  rmSync(directory,{recursive:true,force:true});
}
