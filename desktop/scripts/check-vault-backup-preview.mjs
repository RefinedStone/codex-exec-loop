// Explicit invocation with a built local helper; uses in-memory synthetic keys only.
import assert from "node:assert/strict";
import { createCipheriv, createHash, hkdfSync, randomBytes } from "node:crypto";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
const binary = resolve(process.argv[2]);
const root = mkdtempSync(join(tmpdir(), "akra-preview-contract-"));
const part = value => { const bytes = Buffer.from(value); const len=Buffer.alloc(8); len.writeBigUInt64BE(BigInt(bytes.length)); return Buffer.concat([len,bytes]); };
const versionBytes = version => {const bytes=Buffer.alloc(4); bytes.writeUInt32BE(version); return bytes;};
const seal = (key, nonce, aad, text) => {const cipher=createCipheriv("chacha20-poly1305", key,nonce,{authTagLength:16}); cipher.setAAD(aad);return Buffer.concat([cipher.update(text),cipher.final(),cipher.getAuthTag()]).toString("base64");};
function synthetic(version) {
  const salt=randomBytes(16), backupId=salt.toString("hex"), keyId=version===1 ? "fixture-device" : "archive-"+backupId;
  const recordKey=randomBytes(32), recovery=randomBytes(32), nonce=randomBytes(12);
  const alg="chacha20-poly1305", kdf="hkdf-sha256", workspaceDir="synthetic-project";
  const issue={provider:"github",serverId:"github.com",repositoryId:"R_fixture",issueId:"I_fixture",projectScope:{workspaceDir}};
  const digest=createHash("sha256"); const baseVersion="v1-"+"a".repeat(64);
  function record(field, kind, value, plaintext) {
    const scope=Buffer.concat([workspaceDir,"github",issue.serverId,issue.repositoryId,issue.issueId,field,kind,value].map(part));
    const iv=randomBytes(12), aad=Buffer.concat([part("akra-evidence-vault-record"),versionBytes(1),part(alg),part(keyId),scope]);
    const encoded=JSON.stringify({formatVersion:1,algorithm:alg,keyId,nonce:iv.toString("base64"),ciphertext:seal(recordKey,iv,aad,Buffer.from(plaintext))});
    digest.update(part(scope)); digest.update(part(encoded));
    return {issue,field,binding:kind==="current-version" ? kind : {[kind]:value},record:encoded};
  }
  const records=[record("body","current-version","",baseVersion),record("body","confirmed-version",baseVersion,"synthetic fixture only")];
  const aad=Buffer.concat([part("akra-evidence-vault-backup"),versionBytes(version),part(alg),part(kdf),part(backupId),part(keyId),digest.digest()]);
  const wrap=hkdfSync("sha256",recovery,salt,Buffer.from("akra-evidence-vault-backup-wrap-key-v1"),32);
  return {secret:recovery.toString("hex"),archive:{formatVersion:version,algorithm:alg,kdf,backupId,keyId,wrapNonce:nonce.toString("base64"),wrappedKey:seal(wrap,nonce,aad,recordKey),records}};
}
function preview(key, archive) {
  const run=spawnSync(binary,["desktop-backup-preview"],{cwd:root,env:{...process.env,AKRA_HOME:join(root,"vault"),CODEX_HOME:join(root,"codex")},input:key+"\n"+JSON.stringify(archive),encoding:"utf8",timeout:15000});
  assert.equal(run.status,0); assert.equal(run.stderr,"");return JSON.parse(run.stdout);
}
try {
  for (const version of [1,2]) {
    const {secret,archive}=synthetic(version);
    assert.deepEqual(preview(secret,archive),{status:"verified",version,projects:["synthetic-project"]});
    assert.deepEqual(preview("00".repeat(32),archive),{status:"rejected"});
    assert.deepEqual(preview(secret,{...archive,keyId:"altered"}),{status:"rejected"});
  }
  assert.deepEqual(readdirSync(root),[]);console.log("PASS actual preview helper: authenticated legacy and scoped projects, rejected wrong key/tamper, no storage");
} finally {rmSync(root,{recursive:true,force:true});}
