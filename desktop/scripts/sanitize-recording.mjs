#!/usr/bin/env node
// Gateway 녹화(AKRA_GATEWAY_RECORD=1)를 커밋해도 되는 가짜 Gateway 픽스처로 바꾼다 (P2 §4.8).
// - 계정, 속도 제한, MCP, hook, 경고처럼 화면 시험에 필요 없고 개인 정보가 섞일 수 있는 알림은 버린다.
// - 경로는 /work/example-app 으로 바꾼다.
// - 아래 허용 목록의 구조 키(id, method, type, status 등)만 원문을 남긴다. 나머지 문자열은 모두 지어낸 단어로
//   바꾼다. 토큰처럼 보이는 짧은 값이라도 허용 목록에 없으면 바꾼다 (URL, 토큰, 비공개 식별자가 새지 않게).
// 결과는 반드시 사람이 다시 읽고 커밋한다. 원본 녹화는 커밋하지 않는다.
// usage: node scripts/sanitize-recording.mjs <recording.jsonl> <fixture.jsonl>
import { readFileSync, writeFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

const DROPPED_METHODS =
  /^(account\/|mcpServer\/|hook\/|remoteControl\/|configWarning$|warning$|deprecationNotice$|thread\/tokenUsage\/|thread\/goal\/)/;
// 프로토콜 구조를 담는 키이다. 값이 짧은 토큰이면 원문을 남긴다.
const STRUCTURAL_KEYS = new Set([
  "jsonrpc", "method", "type", "status", "kind", "decision", "role", "source", "phase", "historyMode",
  "approvalPolicy", "approvalsReviewer", "sandbox", "effort", "reasoningEffort", "reasoning_effort",
  "model", "modelProvider", "serviceTier", "cliVersion", "platformFamily", "platformOs", "itemsView",
  "mode", "multiAgentMode", "id", "threadId", "turnId", "itemId", "sessionId", "approvalId", "requestId",
  "environmentId", "at", "direction",
]);
// 열거형, id, method(`item/agentMessage/delta`) 모양이다. 절대 경로는 앞에서 걸러진다.
const TOKEN = /^[A-Za-z0-9_.:+/-]{1,64}$/;
const EXAMPLE_PATH = "/work/example-app";
const PATH_LIKE = /^[A-Za-z]:[\\/]|^\/|\\|^~[\\/]/;

function sampleText(length) {
  const words = [];
  let size = 0;
  for (let index = 0; size < length; index += 1) {
    const word = `word${index % 10}`;
    words.push(word);
    size += word.length + 1;
  }
  return words.join(" ").slice(0, Math.max(length, 1));
}

function sanitizeString(key, value) {
  if (key === "userAgent") {
    return value.split(" ")[0] ?? "codex";
  }
  if (PATH_LIKE.test(value)) {
    return EXAMPLE_PATH;
  }
  if (STRUCTURAL_KEYS.has(key) && TOKEN.test(value)) {
    return value;
  }
  return sampleText(value.length);
}

export { sanitizeValue };

function sanitizeValue(key, value) {
  if (typeof value === "string") {
    return sanitizeString(key, value);
  }
  if (Array.isArray(value)) {
    return value.map((item) => sanitizeValue(key, item));
  }
  if (value && typeof value === "object") {
    return Object.fromEntries(Object.entries(value).map(([childKey, child]) => [childKey, sanitizeValue(childKey, child)]));
  }
  return value;
}

function sanitizeRecording(input, output) {
  const lines = [];
  for (const line of readFileSync(input, "utf8").split(/\r?\n/).filter(Boolean)) {
    const record = JSON.parse(line);
    let message;
    try {
      message = JSON.parse(record.text);
    } catch {
      continue;
    }
    if (message.method && DROPPED_METHODS.test(message.method)) {
      continue;
    }
    lines.push(
      JSON.stringify({ at: record.at, direction: record.direction, text: JSON.stringify(sanitizeValue("", message)) }),
    );
  }
  writeFileSync(output, `${lines.join("\n")}\n`);
  console.log(`wrote ${lines.length} sanitized lines to ${output}`);
}

// 시험에서 sanitizeValue 를 불러올 때는 실행하지 않는다.
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [input, output] = process.argv.slice(2);
  if (!input || !output) {
    throw new Error("usage: node scripts/sanitize-recording.mjs <recording.jsonl> <fixture.jsonl>");
  }
  sanitizeRecording(input, output);
}
