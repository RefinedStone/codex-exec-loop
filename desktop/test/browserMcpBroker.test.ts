import { randomUUID } from "node:crypto";
import { createServer, type Socket } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { describe, expect, it, vi } from "vitest";
import { BrowserBrokerClient, readSession } from "../src/browser-mcp/broker";
import { readFrames } from "../src/browser-mcp/frames";

async function fixture() {
  const endpoint = process.platform === "win32" ? `\\\\.\\pipe\\akra-test-${randomUUID()}` : join(tmpdir(), `akra-${randomUUID()}.sock`);
  const frames: { socket: Socket; frame: Record<string, unknown> }[] = [];
  const next = new Map<string, (socket: Socket) => void>();
  const sockets = new Set<Socket>();
  const server = createServer((socket) => {
    sockets.add(socket);
    socket.on("error", () => {});
    socket.on("close", () => sockets.delete(socket));
    readFrames(socket, (value) => {
      const frame = value as Record<string, unknown>;
      frames.push({ socket, frame });
      if (frame.kind === "watch") socket.write('{"result":{"registrationId":"r"}}\n');
      next.get(frame.kind as string)?.(socket);
    }, () => socket.destroy());
  });
  await new Promise<void>((resolve, reject) => { server.once("error", reject); server.listen(endpoint, resolve); });
  const closed = vi.fn();
  const client = new BrowserBrokerClient({ version: 1, endpoint, registrationId: "r", token: "secret" }, closed);
  return {
    client, frames, closed,
    next: (kind: string) => new Promise<Socket>((resolve) => next.set(kind, resolve)),
    cleanup: async () => {
      client.close();
      for (const socket of sockets) socket.destroy();
      await new Promise<void>((resolve) => server.close(() => resolve()));
    },
  };
}

describe("browser broker socket client", () => {
  it("authenticates a persistent watch, sends separate calls, and closes on revocation", async () => {
    const h = await fixture();
    try {
      await h.client.start();
      expect(h.frames[0]!.frame).toEqual({ kind: "watch", registrationId: "r", token: "secret" });
      const connected = h.next("call");
      const result = h.client.call("thread", "browser_status", {}, new AbortController().signal);
      const socket = await connected;
      expect(h.frames[1]!.frame).toEqual({
        kind: "call", registrationId: "r", token: "secret", threadId: "thread", tool: "browser_status", arguments: {},
      });
      socket.end('{"result":{"content":[{"type":"text","text":"ready"}]}}\n');
      expect(await result).toEqual({ content: [{ type: "text", text: "ready" }] });
      const closed = new Promise<void>((resolve) => h.frames[0]!.socket.once("close", () => resolve()));
      h.frames[0]!.socket.end();
      await closed;
      expect(h.closed).toHaveBeenCalledOnce();
    } finally { await h.cleanup(); }
  });

  it("disconnects the exact call socket on abort and rejects concurrent calls", async () => {
    const h = await fixture();
    try {
      await h.client.start();
      const controller = new AbortController();
      const connected = h.next("call");
      const pending = h.client.call("t", "browser_wait", {}, controller.signal);
      const rejected = expect(pending).rejects.toThrow("Browser call was cancelled.");
      const socket = await connected;
      await expect(h.client.call("t", "browser_status", {}, new AbortController().signal)).rejects.toThrow("unavailable");
      const disconnected = new Promise<void>((resolve) => socket.once("end", resolve));
      controller.abort();
      await rejected;
      await disconnected;
    } finally { await h.cleanup(); }
  });

  it("does not expose a broker error or a sidecar path", async () => {
    const h = await fixture();
    try {
      await h.client.start();
      const connected = h.next("call");
      const pending = h.client.call("t", "browser_status", {}, new AbortController().signal);
      const rejected = expect(pending).rejects.toThrow("Browser call failed.");
      (await connected).end('{"error":{"code":"secret","message":"/private/socket"}}\n');
      await rejected;
      await expect(readSession("/private/missing-sidecar")).rejects.toThrow("Browser session is unavailable.");
    } finally { await h.cleanup(); }
  });

  it("reads a versioned private sidecar and rejects invalid content", async () => {
    const dir = await mkdtemp(join(tmpdir(), "akra-sidecar-"));
    try {
      const path = join(dir, "session.json");
      const session = { version: 1 as const, endpoint: "local", registrationId: "r", token: "secret" };
      await writeFile(path, JSON.stringify(session), { mode: 0o600 });
      expect(await readSession(path)).toEqual(session);
      await writeFile(path, '{"version":9,"token":"secret"}');
      await expect(readSession(path)).rejects.toThrow("Browser session is unavailable.");
    } finally { await rm(dir, { recursive: true, force: true }); }
  });
});
