import assert from "node:assert/strict";
import test from "node:test";
import { validateCrateOwnership } from "./check_crate_boundaries.mjs";

function fixture() {
  const registry = {schemaVersion:1,rootPackage:"app",packages:[
    {name:"app",manifest:"Cargo.toml",state:"active",workspaceDependencies:["rules"]},
    {name:"rules",manifest:"crates/rules/Cargo.toml",state:"active",workspaceDependencies:[]},
  ],contracts:[],transfers:[]};
  const metadata = {workspace_root:"/repo",workspace_members:["opaque-app","opaque-rules"],packages:[
    {id:"opaque-app",name:"app",manifest_path:"/repo/Cargo.toml",dependencies:[{name:"rules",kind:null}]},
    {id:"opaque-rules",name:"rules",manifest_path:"/repo/crates/rules/Cargo.toml",dependencies:[]},
  ]};
  return {registry,metadata};
}

test("independent feature and root consumer preserve opaque package IDs", () => {
  const {registry,metadata}=fixture();
  assert.deepEqual(validateCrateOwnership(registry,metadata),[]);
});
for (const kind of [null,"dev","build"]) {
  test("root back-reference fails even through " + (kind ?? "normal"), () => {
    const {registry,metadata}=fixture();
    metadata.packages[1].dependencies.push({name:"app",rename:"hidden_alias",kind,optional:true});
    assert.match(validateCrateOwnership(registry,metadata).join("\n"),/must not depend on root/);
  });
}
test("new members and undeclared edges cannot bypass the boundary", () => {
  const {registry,metadata}=fixture();
  registry.packages[0].workspaceDependencies=[];
  assert.match(validateCrateOwnership(registry,metadata).join("\n"),/Forbidden workspace edge/);
  metadata.packages[1].name="unregistered";
  assert.match(validateCrateOwnership(registry,metadata).join("\n"),/Unregistered workspace member/);
});
test("planned moves are not mistaken for extracted code", () => {
  const {registry,metadata}=fixture();
  registry.packages[1].state="planned";
  assert.match(validateCrateOwnership(registry,metadata).join("\n"),/not activated/);
});
test("missing members, changed manifests and incomplete metadata fail", () => {
  const {registry,metadata}=fixture();
  metadata.packages.pop();
  assert.match(validateCrateOwnership(registry,metadata).join("\n"),/incomplete|absent/);
  const next=fixture();
  next.metadata.packages[1].manifest_path="/outside/Cargo.toml";
  assert.match(validateCrateOwnership(next.registry,next.metadata).join("\n"),/Manifest owner mismatch/);
});
test("contract ownership and ignored preservation remain explicit", () => {
  const {registry,metadata}=fixture();
  registry.contracts=[{id:"native",owner:"missing",filter:"x",target:{name:"app",kind:"lib"}}];
  registry.transfers=[{source:"old.rs",owner:"rules",ignored:"run-all"}];
  assert.match(validateCrateOwnership(registry,metadata).join("\n"),/Unregistered contract owner/);
  assert.match(validateCrateOwnership(registry,metadata).join("\n"),/preserve ignored/);
});

function splitTransfer() {
  return {source:"legacy.rs",owner:"rules",ignored:"preserve",oldTestPrefix:"legacy::tests::",newTestPrefix:"primitive::tests::",testMappings:[{oldTestPrefix:"legacy::tests::ace_contract",newTestPrefix:"windows_ace::tests::ace_contract"}]};
}
test("one source can explicitly move tests to multiple native modules", () => {
  const {registry,metadata}=fixture();
  registry.transfers=[splitTransfer()];
  assert.deepEqual(validateCrateOwnership(registry,metadata),[]);
});
for (const invalid of [[], null, "legacy::", [{}], [null], [{oldTestPrefix:"",newTestPrefix:"native::"}], [{oldTestPrefix:"old::",newTestPrefix:17}], [{oldTestPrefix:"old::*",newTestPrefix:"native::"}], [{oldTestPrefix:"old::",newTestPrefix:"native::\n"}]]) {
  test("malformed additional test mapping is rejected: " + JSON.stringify(invalid), () => {
    const {registry,metadata}=fixture();
    registry.transfers=[{...splitTransfer(),testMappings:invalid}];
    assert.match(validateCrateOwnership(registry,metadata).join("\n"),/mapping/i);
  });
}
test("duplicate mapping cannot create two owners for the same transfer", () => {
  const {registry,metadata}=fixture();
  const transfer=splitTransfer();
  transfer.testMappings.push({oldTestPrefix:transfer.oldTestPrefix,newTestPrefix:transfer.newTestPrefix});
  registry.transfers=[transfer];
  assert.match(validateCrateOwnership(registry,metadata).join("\n"),/Duplicate test prefix/);
});
