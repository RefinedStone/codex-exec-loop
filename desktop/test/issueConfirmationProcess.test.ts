import { EventEmitter } from "node:events";
import { afterEach, describe, expect, it, vi } from "vitest";
import { IssueConfirmationQueue, IssueConfirmationService } from "../src/main/issueConfirmationService";
import type { ConfirmationControl, ConfirmationDisplay } from "../src/shared/issueConfirmationProcess";

const REQUEST = { workspace: "/work/repo", decisionId: "decision-1", option: "approve", key: "receipt-1" };
const DISPLAY = { message: "현재 서버 질문", detail: "공유본과 선택 효과", confirm_label: "승인" };
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}
class Child extends EventEmitter {
  sent: ConfirmationControl[] = [];
  autoExit = true;
  kill = vi.fn(() => { this.emit("exit"); return true; });
  postMessage(message: ConfirmationControl) {
    this.sent.push(structuredClone(message));
    if (message.type === "dispose" && this.autoExit) queueMicrotask(() => this.emit("exit"));
  }
  event(message: unknown) { this.emit("message", message); }
}
function harness(queue = new IssueConfirmationQueue()) {
  const child = new Child();
  const native = deferred<boolean>();
  const spawn = vi.fn(() => child);
  let alive = true;
  let signal: AbortSignal | undefined;
  const confirm = vi.fn((_display: ConfirmationDisplay, value: AbortSignal) => {
    signal = value;
    return native.promise;
  });
  const service = new IssueConfirmationService({
    process: spawn, endpoint: () => "/private/gateway", executable: () => "/app/akra-gateway",
    confirm, ownerAlive: () => alive,
  }, queue);
  return {
    child, native, spawn, confirm, service, signal: () => signal,
    close: () => { alive = false; service.invalidate(); },
    async ready() {
      await vi.waitFor(() => expect(spawn).toHaveBeenCalledOnce());
      child.event({ type: "ready" });
    },
    async prompt() {
      await this.ready();
      child.event({ type: "prompt", display: DISPLAY });
      expect(confirm).toHaveBeenCalledOnce();
    },
    async approve() {
      native.resolve(true);
      await vi.waitFor(() => expect(child.sent).toContainEqual({ type: "answer", approved: true }));
    },
  };
}
afterEach(() => vi.useRealTimers());

describe("native confirmation and private utility process", () => {
  it("passes only input, native display and sanitized result across the parent channel", async () => {
    const h = harness();
    const response = h.service.confirmDecision(REQUEST);
    await h.prompt();
    expect(h.child.sent).toEqual([{ type: "begin", kind: "decision", input: REQUEST, endpoint: "/private/gateway", executable: "/app/akra-gateway" }]);
    expect(h.confirm.mock.calls[0]?.[0]).toEqual(DISPLAY);
    await h.approve();
    h.child.event({ type: "will-submit" });
    expect(h.child.sent).toContainEqual({ type: "submit", allowed: true });
    h.child.event({ type: "result", response: { cancelled: false, response: { processed: true } } });
    expect(await response).toEqual({ cancelled: false, response: { processed: true } });
    expect(h.signal()?.aborted).toBe(true);
    expect(h.child.sent.at(-1)).toEqual({ type: "dispose" });
  });

  it("a declined native question never allows submission", async () => {
    const h = harness();
    const response = h.service.confirmMode({ workspace: "/work/repo", mode: "observe", key: "mode-1" });
    await h.prompt();
    h.native.resolve(false);
    await vi.waitFor(() => expect(h.child.sent).toContainEqual({ type: "answer", approved: false }));
    h.child.event({ type: "result", response: { cancelled: true } });
    expect(await response).toEqual({ cancelled: true });
    expect(h.child.sent.some((message) => message.type === "submit")).toBe(false);
  });

  it.each(["owner-close", "generation-change"] as const)("%s cancels the dialog and ignores its late approval", async (action) => {
    const h = harness();
    const response = h.service.confirmDecision(REQUEST);
    await h.prompt();
    if (action === "owner-close") h.close();
    else h.service.invalidate();
    expect(h.signal()?.aborted).toBe(true);
    expect(h.child.sent).toContainEqual({ type: "cancel" });
    h.native.resolve(true);
    await Promise.resolve();
    h.child.event({ type: "result", response: { cancelled: true } });
    expect(await response).toEqual({ cancelled: true });
    expect(h.child.sent.some((message) => message.type === "answer" || message.type === "submit")).toBe(false);
  });

  it("rechecks owner lifetime after approval and before permitting any submit", async () => {
    const h = harness();
    const response = h.service.confirmDecision(REQUEST);
    await h.prompt();
    await h.approve();
    h.service.invalidate();
    h.child.event({ type: "will-submit" });
    expect(await response).toEqual({ cancelled: true });
    expect(h.child.sent.some((message) => message.type === "submit")).toBe(false);
  });

  it("a crashed child aborts an unapproved native question without replay", async () => {
    const h = harness();
    const response = h.service.confirmDecision(REQUEST);
    const rejected = expect(response).rejects.toThrow(/^확인 요청을 완료하지 못했습니다/);
    await h.prompt();
    h.child.emit("exit");
    await rejected;
    expect(h.signal()?.aborted).toBe(true);
    h.native.resolve(true);
    h.child.event({ type: "result", response: { cancelled: false, response: { processed: true } } });
    await Promise.resolve();
    expect(h.spawn).toHaveBeenCalledOnce();
    expect(h.child.sent.some((message) => message.type === "answer" || message.type === "submit")).toBe(false);
  });

  it.each(["exit", "invalidate"] as const)("reports unknown after submit permission on %s instead of cancellation or resubmission", async (action) => {
    const h = harness();
    const response = h.service.confirmDecision(REQUEST);
    await h.prompt();
    await h.approve();
    h.child.event({ type: "will-submit" });
    if (action === "exit") h.child.emit("exit");
    else {
      h.service.invalidate();
      h.child.event({ type: "result", response: { cancelled: true } });
    }
    expect(await response).toEqual({ cancelled: false, outcome: "unknown" });
    expect(h.spawn).toHaveBeenCalledOnce();
    expect(h.child.sent.filter((message) => message.type === "submit")).toHaveLength(1);
  });

  it("keeps native questions globally ordered and skips invalidated queued windows", async () => {
    const queue = new IssueConfirmationQueue();
    const first = harness(queue);
    const second = harness(queue);
    const a = first.service.confirmDecision(REQUEST);
    const b = second.service.confirmDecision(REQUEST);
    await first.prompt();
    expect(second.spawn).not.toHaveBeenCalled();
    second.service.invalidate();
    first.native.resolve(false);
    first.child.event({ type: "result", response: { cancelled: true } });
    expect(await a).toEqual({ cancelled: true });
    expect(await b).toEqual({ cancelled: true });
    expect(second.spawn).not.toHaveBeenCalled();
  });

  it("fails a child that never becomes ready and bounds its shutdown", async () => {
    vi.useFakeTimers();
    const h = harness();
    h.child.autoExit = false;
    const response = h.service.confirmDecision(REQUEST);
    const rejected = expect(response).rejects.toThrow(/^확인 요청을 완료하지 못했습니다/);
    await Promise.resolve();
    await vi.advanceTimersByTimeAsync(10_000);
    await rejected;
    expect(h.child.sent).toEqual([{ type: "dispose" }]);
    await vi.advanceTimersByTimeAsync(6_000);
    expect(h.child.kill).toHaveBeenCalledOnce();
    expect(h.spawn).toHaveBeenCalledOnce();
  });

  it("rejects raw protocol frames and early results at the private UI boundary", async () => {
    for (const event of [
      { jsonrpc: "2.0", id: 1, result: { proof_id: "PRIVATE-PROOF" } },
      { type: "result", response: { cancelled: false, response: { processed: true } } },
      { type: "prompt", display: { ...DISPLAY, proof_id: "PRIVATE-PROOF" } },
    ]) {
      const h = harness();
      const response = h.service.confirmDecision(REQUEST);
      const rejected = expect(response).rejects.toThrow(/^확인 요청을 완료하지 못했습니다/);
      await h.ready();
      h.child.event(event);
      await rejected;
      expect(h.confirm).not.toHaveBeenCalled();
      expect(h.child.sent.some((message) => message.type === "submit")).toBe(false);
    }
  });
});


it("routes autonomy to its own utility confirmation kind and honours native cancellation", async () => {
  const h = harness();
  const request = { workspace: "/work/repo", level: "propose", commandId: "autonomy-1", key: "autonomy-key" };
  const response = h.service.confirmAutonomy(request);
  await h.prompt();
  expect(h.child.sent[0]).toMatchObject({ type: "begin", kind: "autonomy", input: request });
  h.native.resolve(false);
  await vi.waitFor(() => expect(h.child.sent).toContainEqual({ type: "answer", approved: false }));
  h.child.event({ type: "result", response: { cancelled: true } });
  expect(await response).toEqual({ cancelled: true });
  expect(h.child.sent.some((message) => message.type === "submit")).toBe(false);
});

it("routes binding resume to its own native confirmation kind without granting authority on cancellation", async () => {
  const h = harness();
  const request = { workspace: "/work/repo", commandId: "resume-1", bindingId: "binding-1", expectedRevision: 8, key: "resume-key" };
  const response = h.service.confirmBindingResume(request);
  await h.prompt();
  expect(h.child.sent[0]).toMatchObject({ type: "begin", kind: "binding_resume", input: request });
  h.native.resolve(false);
  await vi.waitFor(() => expect(h.child.sent).toContainEqual({ type: "answer", approved: false }));
  h.child.event({ type: "result", response: { cancelled: true } });
  expect(await response).toEqual({ cancelled: true });
  expect(h.child.sent.some((message) => message.type === "submit")).toBe(false);
});

it("routes criterion waiver through the private utility channel and native cancellation", async () => {
  const h = harness();
  const request = { workspace: "/work/repo", taskId: "task-1", criterionId: "criterion with spaces",
    scopeId: "a".repeat(64), expectedAcceptanceRevision: 3, reason: "fixture reason", requestId: "waiver-1", key: "waiver-key" };
  const response = h.service.confirmCriterionWaiver(request);
  await h.prompt();
  expect(h.child.sent[0]).toMatchObject({ type: "begin", kind: "criterion_waiver", input: request });
  h.native.resolve(false);
  await vi.waitFor(() => expect(h.child.sent).toContainEqual({ type: "answer", approved: false }));
  h.child.event({ type: "result", response: { cancelled: true } });
  expect(await response).toEqual({ cancelled: true });
  expect(h.child.sent.some((message) => message.type === "submit")).toBe(false);
});
