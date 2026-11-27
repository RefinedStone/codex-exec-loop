import assert from "node:assert/strict";
import { join } from "node:path";
import test from "node:test";
import {
  compareContractInventory, exactTestExecutable, planContractFilters, remainingMemberTests, resolveContractTargets,
} from "./ci-contract-targets.mjs";
import { runNativeContracts, parseContractArguments } from "./run_native_rust_contracts.mjs";

const id = "opaque:root?id=1";
const featureId = "opaque:feature?id=2";
const workspace = process.cwd();
const target = {name:"shared",kind:["lib"],test:true};
const metadata = {
  workspace_root:workspace, workspace_members:[id,featureId],
  packages:[
    {id,name:"akra-ade",manifest_path:join(workspace,"Cargo.toml"),targets:[target]},
    {id:featureId,name:"feature",manifest_path:join(workspace,"crates/feature/Cargo.toml"),targets:[target]},
  ],
};
const ownership = {packages:[{name:"akra-ade",state:"active"},{name:"feature",state:"active"}],transfers:[
  {owner:"feature",oldTestPrefix:"domain::feature::",newTestPrefix:"feature::"},
]};
const artifact = (packageId, executable, changes={}) => ({
  reason:"compiler-artifact",package_id:packageId,target,profile:{test:true},executable,...changes,
});
const json = (x) => JSON.stringify(x)+"\n";
const request = {packageId:id,package:"akra-ade",target:{kind:"lib",name:"shared"}};

test("artifact matching separates identical target names by opaque package ID", () => {
  assert.equal(exactTestExecutable(json(artifact(featureId,"wrong"))+json(artifact(id,"right")),request),"right");
  assert.throws(()=>exactTestExecutable(json(artifact(featureId,"wrong")),request),/reported 0/);
  assert.throws(()=>exactTestExecutable(json(artifact(id,"one"))+json(artifact(id,"two")),request),/reported 2/);
  assert.throws(()=>exactTestExecutable(json(artifact(id,"bin",{target:{name:"shared",kind:["bin"]}})),request),/reported 0/);
  assert.throws(()=>exactTestExecutable(json(artifact(id,"not-test",{profile:{test:false}})),request),/reported 0/);
  assert.throws(()=>exactTestExecutable(json(artifact(id,"right"))),/Exact package/);
});

test("metadata resolves explicit package and target and rejects uncertain ownership", () => {
  const resolved=resolveContractTargets(metadata,{package:featureId,targetKind:"lib",targetName:"shared"},ownership);
  assert.equal(resolved[0].packageKey,"crates/feature/Cargo.toml::feature");
  assert.equal(resolved[0].packageId,featureId);
  assert.equal(resolveContractTargets(metadata,undefined,ownership).length,2);
  assert.throws(()=>resolveContractTargets(metadata,{package:"missing"},ownership),/exactly one/);
  assert.throws(()=>resolveContractTargets(metadata,{package:"feature",targetName:"other"},ownership),/enabled test target/);
  assert.throws(()=>resolveContractTargets(metadata,undefined,{packages:[]}),/Unregistered/);
  assert.throws(()=>resolveContractTargets({...metadata,packages:[]},undefined,ownership),/incomplete/);
});

const prepared = [
  {...request,inventory:["domain::feature::retained","unrelated"],ignored:[]},
  {packageId:featureId,package:"feature",target:{kind:"lib",name:"shared"},
    inventory:["feature::tests::one","feature::tests::two","feature::tests::helper"],ignored:["feature::tests::helper"]},
];

test("moved filters execute the exact owner inventory without enabling ignored helpers", () => {
  const groups=planContractFilters(prepared,["domain::feature::"],ownership.transfers);
  assert.equal(groups.length,2);
  assert.deepEqual(groups[0].args,["domain::feature::retained","--exact","--nocapture"]);
  assert.deepEqual(groups[1].args,["feature::tests::","--nocapture"]);
  assert.equal(groups.reduce((n,g)=>n+g.names.length,0),3);
  assert.throws(()=>planContractFilters(prepared,["helper"],ownership.transfers),/no non-ignored/);
  assert.throws(()=>planContractFilters(prepared,["absent"],ownership.transfers),/no non-ignored/);
  assert.throws(()=>planContractFilters(prepared,["domain::feature::","retained"],ownership.transfers),/twice/);
});

test("additional split mappings remain explicit and preserve the old namespace", () => {
  const transfer=[{owner:"feature",oldTestPrefix:"domain::feature::",newTestPrefix:"feature::",
    testMappings:[{oldTestPrefix:"core::ace::",newTestPrefix:"windows_ace::"}]}];
  const item={...prepared[1],inventory:["windows_ace::allows"],ignored:[]};
  assert.equal(planContractFilters([item],["core::ace::"],transfer)[0].names[0],"windows_ace::allows");
});

const oldTest={package:"root",target:{kind:"lib",name:"root"},name:"old::test",ignored:false};
const newTest={package:"feature",target:{kind:"lib",name:"feature"},name:"new::test",ignored:false};
test("inventory migration requires a bijection, explicit additions and unchanged ignored states", () => {
  const renames=[{from:oldTest,to:newTest}];
  assert.deepEqual(compareContractInventory([oldTest],[newTest],renames),{before:1,after:1,renamed:1,added:0});
  assert.throws(()=>compareContractInventory([oldTest],[],renames),/Missing/);
  assert.throws(()=>compareContractInventory([oldTest],[{...newTest,ignored:true}],renames),/Ignored state/);
  assert.throws(()=>compareContractInventory([oldTest],[newTest,newTest],renames),/Duplicate/);
  assert.throws(()=>compareContractInventory([oldTest],[newTest,{...newTest,name:"extra"}],renames),/Unregistered/);
  assert.equal(compareContractInventory([oldTest],[newTest,{...newTest,name:"extra"}],renames,[{...newTest,name:"extra"}]).added,1);
});

test("one build prepares multiple owner executables and preserves feature/profile/target context", async () => {
  const calls=[];const records=[];
  const result=await runNativeContracts(["--all-features","--profile","debugging","--target","host-triple","--","domain::feature::"],{
    metadata,ownership,record:(e)=>records.push(e),write(){},
    run(command,args,settings){
      calls.push({command,args,settings});
      if(command==="cargo")return{status:0,stdout:json(artifact(id,"root-exe"))+json(artifact(featureId,"feature-exe"))};
      if(args.includes("--list"))return{status:0,stdout:args.includes("--ignored")?"":(command==="root-exe"?"domain::feature::retained: test\n":"feature::one: test\nfeature::two: test\n")};
      return{status:0,stdout:"test result: ok. "+(command==="root-exe"?1:2)+" passed; 0 failed;\n"};
    },
  });
  assert.equal(calls.filter(c=>c.command==="cargo").length,1);
  assert.ok(calls[0].args.includes(id)&&calls[0].args.includes(featureId));
  assert.deepEqual(result,{preparedTargets:2,executedGroups:2,executedTests:3});
  assert.ok(records.every(r=>r.profile==="debugging"&&r.features==="all"&&r.targetTriple==="host-triple"));
  assert.equal(new Set(records.map(r=>r.packageId)).size,2);
});

test("registry contract selection validates the required platform and feature condition", async () => {
  const registry={...ownership,contracts:[{id:"named",owner:"feature",target:{kind:"lib",name:"shared"},
    filter:"feature::one",platforms:["windows"],features:"all",isolation:"process-per-filter"}]};
  const settings={metadata,ownership:registry,platform:"win32",write(){},record(){},run(command,args){
    if(command==="cargo")return{status:0,stdout:json(artifact(featureId,"feature-exe"))};
    if(args.includes("--list"))return{status:0,stdout:args.includes("--ignored")?"":"feature::one: test\n"};
    return{status:0,stdout:"test result: ok. 1 passed; 0 failed;\n"};
  }};
  assert.equal((await runNativeContracts(["--contract-id","named","--all-features","--"],settings)).executedTests,1);
  await assert.rejects(()=>runNativeContracts(["--contract-id","named","--"],settings),/all-features/);
  await assert.rejects(()=>runNativeContracts(["--contract-id","named","--all-features","--"],{...settings,platform:"linux"}),/OS/);
  assert.throws(()=>parseContractArguments(["--target-kind","test","--","one"]),/explicit package/);
  assert.throws(()=>parseContractArguments(["--contract-id","named","--","other"]),/supplies its own/);
});

test("member remainder covers only unselected tests and uses exact skip matching", () => {
  const p=[{...prepared[1],inventory:["feature::one","feature::one_extra","feature::helper"],ignored:["feature::helper"]}];
  const selected=[{...p[0],names:["feature::one"]}];
  const rest=remainingMemberTests(p,selected);
  assert.deepEqual(rest[0].names,["feature::one_extra"]);
  assert.deepEqual(rest[0].args,["--exact","--nocapture","--skip","feature::one"]);
  assert.equal(remainingMemberTests([prepared[0]],[]).length,0);
});

test("full member selection includes its integration targets but not root binaries", () => {
  const m=structuredClone(metadata);
  m.packages[0].targets.push({name:"root-bin",kind:["bin"],test:true});
  m.packages[1].targets.push({name:"contract",kind:["test"],test:true});
  m.packages[1].targets.push({name:"optional",kind:["test"],test:true,"required-features":["disabled"]});
  const requests=resolveContractTargets(m,undefined,ownership,true);
  assert.deepEqual(requests.map(r=>[r.package,r.target.kind,r.target.name]),[["akra-ade","lib","shared"],["feature","lib","shared"],["feature","test","contract"]]);
});

test("an explicit complete member may have no contract filters and retains all its targets", async () => {
  assert.equal(parseContractArguments(["--include-members","--package","feature","--"]).includeMembers,true);
  assert.throws(()=>parseContractArguments(["--include-members","--"]));
  assert.throws(()=>parseContractArguments(["--include-members","--package","feature","--target-kind","lib","--","x"]),/complete member/);
  const resolved=resolveContractTargets(metadata,{package:"feature"},ownership,true);
  assert.equal(resolved.length,1);
  const result=await runNativeContracts(["--include-members","--package","feature","--"],{metadata,ownership,write(){},record(){},
    run(command,args){
      if(command==="cargo") return {status:0,stdout:json(artifact(featureId,"feature-exe"))};
      if(args.includes("--list")) return {status:0,stdout:args.includes("--ignored")?"":"feature::one: test\n"};
      return {status:0,stdout:"test result: ok. 1 passed; 0 failed;\n"};
    }
  });
  assert.equal(result.executedTests,1);
});

test("all-tests mode requires exact package/target identity and permits a legitimate empty harness", async () => {
  assert.throws(()=>parseContractArguments(["--all-tests","--"]));
  const result=await runNativeContracts(["--all-tests","--package","feature","--target-kind","lib","--target-name","shared","--"],{
    metadata,ownership,write(){},record(){},run(command,args){
      if(command==="cargo")return {status:0,stdout:json(artifact(featureId,"feature-exe"))};
      if(args.includes("--list"))return {status:0,stdout:""};
      return {status:0,stdout:"test result: ok. 0 passed; 0 failed;\n"};
    }
  });
  assert.deepEqual(result,{preparedTargets:1,executedGroups:1,executedTests:0});
});
