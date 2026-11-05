// Read-only probe: N concurrent `codex app-server proxy` processes. Each speaks
// WebSocket over the proxy's stdio, then runs initialize -> initialized -> thread/list
// with the SAME request ids. No turn, no model call. Prints status/keys/counts only.
// usage: node docs/plan/17-akra-desktop/scripts/proxy_ws_check.mjs <codex executable> <clients> [socketPath]
// Without socketPath the proxy connects to the Codex managed daemon control socket.
// On Windows the socket directory must be private to the current user.
import { spawn } from "node:child_process";
import { randomBytes } from "node:crypto";

const [codex, clientsArg, sock] = process.argv.slice(2);
const clients = Number(clientsArg || 2);

function encodeTextFrame(text) {
  const payload = Buffer.from(text, "utf8");
  const mask = randomBytes(4);
  const len = payload.length;
  const header = len < 126 ? Buffer.from([0x81, 0x80 | len])
    : len < 65536 ? Buffer.from([0x81, 0x80 | 126, len >> 8, len & 255])
    : (() => { const h = Buffer.alloc(10); h[0] = 0x81; h[1] = 0x80 | 127; h.writeBigUInt64BE(BigInt(len), 2); return h; })();
  const masked = Buffer.alloc(len);
  for (let i = 0; i < len; i++) masked[i] = payload[i] ^ mask[i % 4];
  return Buffer.concat([header, mask, masked]);
}

// Server frames are unmasked. Returns [messages, rest]. Fragmentation ignored (max unfragmented = 16 MiB).
function decodeFrames(buf) {
  const messages = [];
  while (buf.length >= 2) {
    const opcode = buf[0] & 0x0f;
    let len = buf[1] & 0x7f, off = 2;
    if (len === 126) { if (buf.length < 4) break; len = buf.readUInt16BE(2); off = 4; }
    else if (len === 127) { if (buf.length < 10) break; len = Number(buf.readBigUInt64BE(2)); off = 10; }
    if (buf.length < off + len) break;
    if (opcode === 1) messages.push(buf.subarray(off, off + len).toString("utf8"));
    buf = buf.subarray(off + len);
  }
  return [messages, buf];
}

function runClient(index) {
  return new Promise((resolve) => {
    const child = spawn(codex, ["app-server", "proxy", ...(sock ? ["--sock", sock] : [])], { stdio: ["pipe", "pipe", "pipe"] });
    const lines = [], pending = new Map();
    let buf = Buffer.alloc(0), upgraded = false, stderr = "", notifications = 0;
    const t0 = Date.now();
    const timer = setTimeout(() => finish(`timeout; stderr: ${stderr.slice(-300)}`), 20000);
    function finish(extra) {
      clearTimeout(timer); child.kill();
      resolve(`client ${index} (proxy pid ${child.pid}):\n  ${[...lines, `notifications seen: ${notifications}`, extra].filter(Boolean).join("\n  ")}`);
    }
    child.stderr.on("data", (d) => (stderr += d));
    child.on("error", (e) => finish(`spawn error ${e.message}`));
    child.stdout.on("data", (d) => {
      buf = Buffer.concat([buf, d]);
      if (!upgraded) {
        const end = buf.indexOf("\r\n\r\n");
        if (end < 0) return;
        const status = buf.subarray(0, buf.indexOf("\r\n")).toString();
        lines.push(`upgrade: ${status} | ${Date.now() - t0}ms`);
        upgraded = true; buf = buf.subarray(end + 4);
        start();
      }
      const [messages, rest] = decodeFrames(buf); buf = rest;
      for (const m of messages) {
        const msg = JSON.parse(m);
        if (msg.id !== undefined && pending.has(msg.id)) { pending.get(msg.id)(msg); pending.delete(msg.id); }
        else notifications++;
      }
    });
    const send = (obj) => child.stdin.write(encodeTextFrame(JSON.stringify(obj)));
    const request = (id, method, params) => new Promise((ok) => { pending.set(id, ok); send({ jsonrpc: "2.0", id, method, params }); });
    const describe = (r) => r.error ? `error ${r.error.code} ${String(r.error.message).slice(0, 120)}` : "ok";
    async function start() {
      const init = await request(1, "initialize", { clientInfo: { name: `akra-proxy-check-${index}`, version: "0" }, capabilities: { experimentalApi: false } });
      lines.push(`initialize: ${describe(init)} | keys: ${Object.keys(init.result || {}).sort()} | ${Date.now() - t0}ms`);
      send({ jsonrpc: "2.0", method: "initialized", params: {} });
      const list = await request(2, "thread/list", { limit: 3 });
      lines.push(`thread/list: ${describe(list)} | count: ${((list.result || {}).data || []).length} | ${Date.now() - t0}ms`);
      finish();
    }
    child.stdin.write(`GET / HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: ${randomBytes(16).toString("base64")}\r\nSec-WebSocket-Version: 13\r\n\r\n`);
  });
}

console.log((await Promise.all(Array.from({ length: clients }, (_, i) => runClient(i + 1)))).join("\n"));
