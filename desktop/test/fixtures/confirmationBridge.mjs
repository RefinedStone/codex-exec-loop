// Electron E2E 전용 helper. 실제 production 우회 옵션 없이 고정 argv와 stdio 경로를 그대로 시험한다.
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";

export const CONFIRMATION_DISPLAY = {
  message: "서버가 준비한 사람 확인 질문",
  detail: "서버가 확인한 이슈, 선택 항목, 공유 메모",
  confirm_label: "서버 선택으로 승인",
};

let registeredBinding;

export function fixtureIntent(request) {
  return { binding: registeredBinding, target: { kind: "fixture-confirmation", command: structuredClone(request.command) }, key: request.idempotency_key, fingerprint: "0".repeat(32) };
}

export function confirmationReply(method, params) {
  if (method === "akra/kernel/confirmation/register") {
    if (!params.registration_id?.startsWith("fixture-private-registration-") || !params.session_file?.includes("private-session")) {
      return { error: { code: -32602, message: "invalid fixture registration" } };
    }
    registeredBinding = { registration_id: params.registration_id, connection_id: "fixture-connection", attachment_id: "fixture-attachment", client_id: "fixture-client", workspace: "/work/example-app" };
    return { result: { binding: registeredBinding } };
  }
  if (method === "akra/kernel/confirmation/prepare") {
    return { result: { intent: fixtureIntent(params.request), display: CONFIRMATION_DISPLAY } };
  }
  return undefined;
}

export function confirmedRequest(params) {
  if (!params.confirmation?.proof_id?.startsWith("fixture-private-proof-") ||
    !params.confirmation.registration_id?.startsWith("fixture-private-registration-") ||
    JSON.stringify(params.confirmation.intent) !== JSON.stringify(fixtureIntent(params.request))) {
    throw new Error("confirmed command did not preserve the pipe proof and exact server intent");
  }
  return params.request;
}

export async function createConfirmationFixture(parent) {
  const cwd = join(parent, "confirmation-helper");
  await mkdir(cwd, { recursive: true });
  const metadataPath = join(cwd, "metadata.jsonl");
  const sessionFile = join(cwd, "private-session");
  await writeFile(metadataPath, "");
  const source = [
    'const fs = require("node:fs");',
    'const readline = require("node:readline");',
    'const metadata = ' + JSON.stringify(metadataPath) + ';',
    'const sessionFile = ' + JSON.stringify(sessionFile) + ';',
    'const registration = "fixture-private-registration-" + process.pid;',
    'const record = (command) => fs.appendFileSync(metadata, JSON.stringify({command, argv:process.argv.slice(1)}) + "\\n");',
    'const send = (value) => process.stdout.write(JSON.stringify(value) + "\\n");',
    'record("start");',
    'send({type:"ready"});',
    'readline.createInterface({input:process.stdin}).on("line", (line) => {',
    '  const request = JSON.parse(line); record(request.command);',
    '  if (request.command === "prepare") send({id:request.id,result:{registration_id:registration,session_file:sessionFile}});',
    '  else if (request.command === "approve" && request.registration_id === registration) send({id:request.id,result:{proof_id:"fixture-private-proof-" + process.pid}});',
    '  else send({id:request.id,error:{message:"invalid fixture helper request"}});',
    '}).on("close", () => record("close"));',
  ].join("\n");
  await writeFile(join(cwd, "desktop-confirmation-bridge"), source);
  return {
    cwd,
    environment: { AKRA_GATEWAY_BIN: process.execPath },
    async metadata() {
      return (await readFile(metadataPath, "utf8")).trim().split("\n").filter(Boolean).map((line) => JSON.parse(line));
    },
  };
}
