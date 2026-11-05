import { describe, expect, it } from "vitest";

import { JsonRpcClient, JsonRpcError } from "../src/renderer/rpc/jsonRpcClient";

function createClient() {
  const sent: Array<Record<string, unknown>> = [];
  const client = new JsonRpcClient((text) => sent.push(JSON.parse(text) as Record<string, unknown>));
  return { client, sent };
}

describe("JsonRpcClient", () => {
  it("matches responses to requests by id", async () => {
    const { client, sent } = createClient();
    const first = client.request<string>("a");
    const second = client.request<string>("b");
    client.handleText(JSON.stringify({ jsonrpc: "2.0", id: sent[1]?.id, result: "B" }));
    client.handleText(JSON.stringify({ jsonrpc: "2.0", id: sent[0]?.id, result: "A" }));
    await expect(first).resolves.toBe("A");
    await expect(second).resolves.toBe("B");
    expect(client.pendingCount).toBe(0);
  });

  it("rejects with JsonRpcError when the server returns an error", async () => {
    const { client, sent } = createClient();
    const pending = client.request("missing");
    client.handleText(
      JSON.stringify({ jsonrpc: "2.0", id: sent[0]?.id, error: { code: -32601, message: "nope" } }),
    );
    await expect(pending).rejects.toBeInstanceOf(JsonRpcError);
  });

  it("delivers notifications to listeners", () => {
    const { client } = createClient();
    const seen: string[] = [];
    const off = client.onNotification((method) => seen.push(method));
    client.handleText(JSON.stringify({ jsonrpc: "2.0", method: "turn/started", params: {} }));
    off();
    client.handleText(JSON.stringify({ jsonrpc: "2.0", method: "turn/completed", params: {} }));
    expect(seen).toEqual(["turn/started"]);
  });

  it("reset fails every pending request", async () => {
    const { client } = createClient();
    const pending = client.request("thread/list");
    client.reset("disconnected");
    await expect(pending).rejects.toThrow("thread/list: disconnected");
    expect(client.pendingCount).toBe(0);
  });

  it("answers server requests with the handler result or method not found", async () => {
    const { client, sent } = createClient();
    client.handleText(JSON.stringify({ jsonrpc: "2.0", id: 9, method: "item/commandExecution/requestApproval" }));
    await Promise.resolve();
    expect(sent.at(-1)).toMatchObject({ id: 9, error: { code: -32601 } });

    client.setServerRequestHandler(async () => ({ decision: "decline" }));
    client.handleText(JSON.stringify({ jsonrpc: "2.0", id: 10, method: "item/commandExecution/requestApproval" }));
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(sent.at(-1)).toMatchObject({ id: 10, result: { decision: "decline" } });
  });

  it("ignores text that is not JSON", () => {
    const { client } = createClient();
    expect(() => client.handleText("not json")).not.toThrow();
  });
});
