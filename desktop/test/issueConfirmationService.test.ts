import { describe, expect, it, vi } from "vitest";
import { IssueConfirmationQueue, IssueConfirmationService, type ConfirmationDisplay } from "../src/confirmation/service";
import type { ConfirmationMethod } from "../src/confirmation/connection";

const REQUEST = { workspace: "/work/repo", decisionId: "decision-1", option: "approve", note: "공유 메모", key: "receipt-1" };
const DISPLAY = { message: "서버의 확인 질문", detail: "서버가 정한 이슈·선택·공유 메모", confirm_label: "서버의 승인 문구" };
const REGISTRATION = { registration_id: "private-registration-canary", session_file: "/private/confirmation-session-canary" };
const BINDING = { registration_id: REGISTRATION.registration_id, connection_id: "connection-1", attachment_id: "attachment-1", client_id: "client-1", workspace: "/work/repo" };
const INTENT = { binding: BINDING, target: { type: "answer_issue_decision", decision_id: "decision-1", option: "approve", basis: "basis-1" }, key: REQUEST.key, fingerprint: "0".repeat(32) };
const PROOF = { proof_id: "private-proof-canary" };

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

function harness(queue = new IssueConfirmationQueue()) {
  let alive = true;
  let rpcClosed = () => {};
  let helperClosed = () => {};
  const native = deferred<boolean>();
  const request = vi.fn(async (method: ConfirmationMethod, _params: unknown): Promise<unknown> => {
    if (method === "akra/kernel/attach") return { client_id: "1", protocol_version: 1, kernel: "repo" };
    if (method === "akra/kernel/confirmation/register") return { binding: BINDING };
    if (method === "akra/kernel/confirmation/prepare") return { intent: { ...INTENT, key: (_params as { request: { idempotency_key: string } }).request.idempotency_key }, display: DISPLAY };
    return { processed: true };
  });
  const rpc = { request, close: vi.fn() };
  const helper = { prepare: vi.fn(async () => REGISTRATION), approve: vi.fn(async () => PROOF), close: vi.fn() };
  const confirm = vi.fn((display: ConfirmationDisplay, signal: AbortSignal) => new Promise<boolean>((resolve) => {
    expect(display).toEqual(DISPLAY);
    native.promise.then(resolve);
    signal.addEventListener("abort", () => resolve(false), { once: true });
  }));
  const createRpc = vi.fn((onClosed: () => void) => { rpcClosed = onClosed; return rpc; });
  const createHelper = vi.fn((onClosed: () => void) => { helperClosed = onClosed; return helper; });
  const beforeSubmit = vi.fn(async (_signal: AbortSignal) => true);
  const service = new IssueConfirmationService({
    rpc: createRpc, helper: createHelper, confirm, ownerAlive: () => alive, beforeSubmit,
  }, queue);
  return {
    service, rpc, helper, confirm, native, createRpc, createHelper, beforeSubmit,
    closeOwner: () => { alive = false; service.invalidate(); },
    disconnect: () => rpcClosed(),
    closeHelper: () => helperClosed(),
    untilDialog: () => vi.waitFor(() => expect(confirm).toHaveBeenCalledTimes(1)),
    submitted: () => request.mock.calls.filter(([method]) => method === "akra/kernel/command_confirmed"),
  };
}

describe("utility issue confirmation", () => {
  it("rejects a server prompt that echoes a private registration before contacting native UI", async () => {
    const h = harness();
    const original = h.rpc.request.getMockImplementation()!;
    h.rpc.request.mockImplementation(async (method, params) => method === "akra/kernel/confirmation/prepare"
      ? { intent: INTENT, display: { ...DISPLAY, detail: REGISTRATION.session_file } }
      : original(method, params));
    await expect(h.service.confirmDecision(REQUEST)).rejects.toThrow(/^확인 요청을 완료하지 못했습니다/);
    expect(h.confirm).not.toHaveBeenCalled();
    expect(h.helper.approve).not.toHaveBeenCalled();
    expect(h.submitted()).toHaveLength(0);
  });

  it("waits for the current native owner before submitting an already approved proof", async () => {
    const h = harness();
    const permission = deferred<boolean>();
    h.beforeSubmit.mockImplementation(() => permission.promise);
    const result = h.service.confirmDecision(REQUEST);
    await h.untilDialog();
    h.native.resolve(true);
    await vi.waitFor(() => expect(h.beforeSubmit).toHaveBeenCalledOnce());
    expect(h.helper.approve).toHaveBeenCalledOnce();
    expect(h.submitted()).toHaveLength(0);
    h.service.invalidate();
    permission.resolve(true);
    expect(await result).toEqual({ cancelled: true });
    expect(h.submitted()).toHaveLength(0);
  });

  it("uses the server's text and issues proof only after native approval, exactly once", async () => {
    const h = harness();
    const result = h.service.confirmDecision(REQUEST);
    await h.untilDialog();
    expect(h.rpc.request.mock.calls.map(([method]) => method)).toEqual([
      "akra/kernel/attach", "akra/kernel/confirmation/register", "akra/kernel/confirmation/prepare",
    ]);
    expect(h.helper.approve).not.toHaveBeenCalled();
    expect(h.submitted()).toHaveLength(0);
    expect(h.confirm.mock.calls[0]?.[0]).toEqual(DISPLAY);
    h.native.resolve(true);
    expect(await result).toEqual({ cancelled: false, response: { processed: true } });
    expect(h.helper.approve).toHaveBeenCalledExactlyOnceWith(REGISTRATION.registration_id, INTENT);
    expect(h.submitted()).toEqual([["akra/kernel/command_confirmed", {
      request: { command: { type: "answer_issue_decision", decision_id: "decision-1", option: "approve", note: "공유 메모" }, idempotency_key: "receipt-1" },
      confirmation: { registration_id: REGISTRATION.registration_id, proof_id: PROOF.proof_id, intent: INTENT },
    }]]);
    expect(h.rpc.close).toHaveBeenCalledOnce();
    expect(h.helper.close).toHaveBeenCalledOnce();
  });

  it("cancellation never issues or submits proof", async () => {
    const h = harness();
    const result = h.service.confirmDecision(REQUEST);
    await h.untilDialog();
    h.native.resolve(false);
    expect(await result).toEqual({ cancelled: true });
    expect(h.helper.approve).not.toHaveBeenCalled();
    expect(h.submitted()).toHaveLength(0);
  });

  it.each(["close", "replace", "disconnect", "helper-close"] as const)("%s invalidates a pending dialog before any approval", async (action) => {
    const h = harness();
    const result = h.service.confirmDecision(REQUEST);
    await h.untilDialog();
    if (action === "close") h.closeOwner();
    else if (action === "replace") h.service.invalidate();
    else if (action === "disconnect") h.disconnect();
    else h.closeHelper();
    h.native.resolve(true);
    expect(await result).toEqual({ cancelled: true });
    expect(h.confirm.mock.calls[0]?.[1].aborted).toBe(true);
    expect(h.helper.approve).not.toHaveBeenCalled();
    expect(h.submitted()).toHaveLength(0);
  });

  it("drops proof that arrives after the renderer has been replaced", async () => {
    const h = harness();
    const proof = deferred<typeof PROOF>();
    h.helper.approve.mockImplementation(() => proof.promise);
    const result = h.service.confirmDecision(REQUEST);
    await h.untilDialog();
    h.native.resolve(true);
    await vi.waitFor(() => expect(h.helper.approve).toHaveBeenCalledOnce());
    h.service.invalidate();
    proof.resolve(PROOF);
    expect(await result).toEqual({ cancelled: true });
    expect(h.submitted()).toHaveLength(0);
  });

  it("serializes different windows and cancels queued work from a replaced renderer", async () => {
    const queue = new IssueConfirmationQueue();
    const first = harness(queue);
    const second = harness(queue);
    const a = first.service.confirmDecision(REQUEST);
    const b = second.service.confirmMode({ workspace: "/work/repo", mode: "observe", key: "mode-1" });
    await first.untilDialog();
    expect(second.createHelper).not.toHaveBeenCalled();
    second.service.invalidate();
    first.native.resolve(false);
    expect(await a).toEqual({ cancelled: true });
    expect(await b).toEqual({ cancelled: true });
    expect(second.createRpc).not.toHaveBeenCalled();
  });

  it("keeps the mode command and its caller-supplied retry key intact", async () => {
    const h = harness();
    const result = h.service.confirmMode({ workspace: "/work/repo", mode: "observe", key: "same-key" });
    await h.untilDialog();
    expect(h.rpc.request).toHaveBeenCalledWith("akra/kernel/confirmation/prepare", {
      request: { command: { type: "set_issue_automation_mode", mode: "observe" }, idempotency_key: "same-key" },
    });
    h.native.resolve(true);
    await result;
    expect(h.submitted()).toHaveLength(1);
  });

  it("snapshots input before queuing it and rejects client-supplied proof or actor fields", async () => {
    const h = harness();
    const input = { ...REQUEST };
    const result = h.service.confirmDecision(input);
    input.option = "different";
    await h.untilDialog();
    h.native.resolve(true);
    await result;
    expect(h.submitted()[0]?.[1]).toMatchObject({ request: { command: { option: "approve" } } });
    for (const field of ["confirmation", "by", "caller", "intent", "proof_id"]) {
      expect(() => h.service.confirmDecision({ ...REQUEST, [field]: "PRIVATE-INPUT" })).toThrow("확인 요청을 완료하지 못했습니다");
    }
    expect(() => h.service.confirmDecision({ ...REQUEST, note: "한".repeat(1_366) })).toThrow();
    expect(() => h.service.confirmDecision({ ...REQUEST, option: "untrusted option" })).toThrow();
  });

  it("does not return bridge secrets even when a malformed server response echoes one", async () => {
    const h = harness();
    h.rpc.request.mockImplementation(async (method) => {
      if (method === "akra/kernel/confirmation/register") return { binding: BINDING };
      if (method === "akra/kernel/confirmation/prepare") return { intent: INTENT, display: DISPLAY };
      if (method === "akra/kernel/command_confirmed") return { processed: false, failure: PROOF.proof_id };
      return {};
    });
    const result = h.service.confirmDecision(REQUEST);
    await h.untilDialog();
    h.native.resolve(true);
    expect(await result).toEqual({ cancelled: false, outcome: "unknown" });
  });

  it.each(["disconnect", "timeout"] as const)("reports an unknown result after a committed command loses its %s response", async (failure) => {
    const h = harness();
    const originalRequest = h.rpc.request.getMockImplementation()!;
    let committed = 0;
    h.rpc.request.mockImplementation(async (method, params) => {
      if (method !== "akra/kernel/command_confirmed") return originalRequest(method, params);
      committed++;
      if (failure === "disconnect") h.disconnect();
      throw new Error("PRIVATE-PROOF and private response details");
    });
    const result = h.service.confirmDecision(REQUEST);
    await h.untilDialog();
    h.native.resolve(true);
    expect(await result).toEqual({ cancelled: false, outcome: "unknown" });
    expect(committed).toBe(1);
    expect(h.helper.approve).toHaveBeenCalledOnce();
    expect(h.submitted()).toHaveLength(1);
    expect(h.createRpc).toHaveBeenCalledOnce();
  });

  it("does not report cancellation when a submitted response arrives after renderer replacement", async () => {
    const h = harness();
    const originalRequest = h.rpc.request.getMockImplementation()!;
    const reply = deferred<unknown>();
    h.rpc.request.mockImplementation((method, params) => method === "akra/kernel/command_confirmed" ? reply.promise : originalRequest(method, params));
    const result = h.service.confirmDecision(REQUEST);
    await h.untilDialog();
    h.native.resolve(true);
    await vi.waitFor(() => expect(h.submitted()).toHaveLength(1));
    h.service.invalidate();
    reply.resolve({ processed: true });
    expect(await result).toEqual({ cancelled: false, outcome: "unknown" });
    expect(h.helper.approve).toHaveBeenCalledOnce();
    expect(h.submitted()).toHaveLength(1);
  });

  it("does not echo helper or Gateway failure messages to the renderer", async () => {
    const h = harness();
    h.helper.prepare.mockRejectedValue(new Error("PRIVATE-PROOF raw note /private/session"));
    await expect(h.service.confirmDecision(REQUEST)).rejects.toThrow(/^확인 요청을 완료하지 못했습니다/);
    expect(h.confirm).not.toHaveBeenCalled();
    expect(h.submitted()).toHaveLength(0);
  });
});


describe("utility issue autonomy confirmation", () => {
  it("submits the exact autonomy command once after native approval", async () => {
    const h = harness();
    const result = h.service.confirmAutonomy({ workspace: "/work/repo", level: "propose", commandId: "autonomy-1", key: "autonomy-key" });
    await h.untilDialog();
    expect(h.submitted()).toHaveLength(0);
    h.native.resolve(true);
    expect(await result).toEqual({ cancelled: false, response: { processed: true } });
    expect(h.submitted()).toHaveLength(1);
    expect(h.submitted()[0]?.[1]).toMatchObject({
      request: { command: { type: "set_issue_autonomy", command_id: "autonomy-1", level: "propose" }, idempotency_key: "autonomy-key" },
    });
  });

  it.each([
    { level: "auto" }, { level: "gated" }, { commandId: "" }, { commandId: "bad id" }, { proof_id: "renderer-proof" },
  ])("rejects invalid autonomy input before opening a confirmation helper: %j", (extra) => {
    const h = harness();
    expect(() => h.service.confirmAutonomy({ workspace: "/work/repo", level: "propose", commandId: "autonomy-1", key: "key", ...extra })).toThrow();
    expect(h.createRpc).not.toHaveBeenCalled();
    expect(h.createHelper).not.toHaveBeenCalled();
  });
});
