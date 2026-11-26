import { afterEach, describe, expect, it, vi } from "vitest";
import { MODEL_LIST_TIMEOUT_MS, NextTurnSettingsController } from "../src/renderer/session/nextTurnSettings";

const model = (name = "model-a", efforts = ["low", "high"]) => ({
  model: name, displayName: name.toUpperCase(), hidden: false,
  supportedReasoningEfforts: efforts.map((reasoningEffort) => ({ reasoningEffort, description: reasoningEffort })),
  defaultReasoningEffort: efforts[0] ?? "none",
});
const page = (data = [model()], nextCursor: string | null = null) => ({ data, nextCursor });
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((complete) => { resolve = complete; });
  return { promise, resolve };
}
afterEach(() => vi.useRealTimers());

describe("next turn model settings", () => {
  it("reads all model pages and uses the server's supported effort names", async () => {
    const list = vi.fn().mockResolvedValueOnce(page([model()], "next")).mockResolvedValueOnce(page([model("model-b", ["future-effort"])]));
    const settings = new NextTurnSettingsController(list);
    settings.connectionChanged(true);
    await settings.refresh();
    expect(list.mock.calls).toEqual([[{ limit: 100, includeHidden: false }], [{ limit: 100, includeHidden: false, cursor: "next" }]]);
    settings.setModel("t", "model-b");
    expect(settings.prepare("t").params).toEqual({ model: "model-b", effort: "future-effort" });
    expect(() => settings.setEffort("t", "invented")).toThrow("지원하는");
    settings.dispose();
  });

  it("does not replace a newer list with a late response", async () => {
    const old = deferred<unknown>(); const recent = deferred<unknown>();
    const list = vi.fn().mockReturnValueOnce(old.promise).mockReturnValueOnce(recent.promise);
    const settings = new NextTurnSettingsController(list);
    settings.connectionChanged(true);
    const first = settings.refresh(); const second = settings.refresh();
    recent.resolve(page([model("new")])); await second;
    old.resolve(page([model("old")])); await first;
    expect(settings.catalogSnapshot().models.map((entry) => entry.model)).toEqual(["new"]);
    settings.dispose();
  });

  it("marks a timed-out catalog unavailable and ignores its late response", async () => {
    vi.useFakeTimers();
    const pending = deferred<unknown>();
    const settings = new NextTurnSettingsController(() => pending.promise);
    settings.connectionChanged(true);
    const loading = settings.refresh();
    await vi.advanceTimersByTimeAsync(MODEL_LIST_TIMEOUT_MS); await loading;
    expect(settings.catalogSnapshot().phase).toBe("failed");
    pending.resolve(page()); await Promise.resolve();
    expect(settings.catalogSnapshot().phase).toBe("failed");
    expect(settings.prepare("t").params).toEqual({});
    settings.dispose();
  });

  it("keeps inheritance usable when model/list is unsupported or malformed", async () => {
    const list = vi.fn().mockRejectedValueOnce(new Error("unknown method")).mockResolvedValueOnce(page([{
      ...model(), defaultReasoningEffort: "missing",
    }]));
    const settings = new NextTurnSettingsController(list);
    settings.connectionChanged(true);
    await settings.refresh();
    expect(settings.catalogSnapshot().error).toContain("unknown method");
    expect(settings.prepare("t").params).toEqual({});
    await settings.refresh();
    expect(settings.catalogSnapshot().phase).toBe("failed");
    expect(() => settings.setModel("t", "model-a")).toThrow("지원 목록");
    settings.dispose();
  });

  it("retains an invalid draft after the support list changes and blocks dispatch", async () => {
    const list = vi.fn().mockResolvedValueOnce(page()).mockResolvedValueOnce(page([model("model-a", ["low"])]));
    const settings = new NextTurnSettingsController(list);
    settings.connectionChanged(true); await settings.refresh();
    settings.setModel("t", "model-a"); settings.setEffort("t", "high");
    await settings.refresh();
    expect(settings.snapshot("t").draft.effort).toBe("high");
    expect(() => settings.prepare("t")).toThrow("지원 목록");
    settings.reset("t");
    expect(settings.prepare("t").params).toEqual({});
    settings.dispose();
  });

  it("separates host and thread drafts and fences accepted responses across reconnect", async () => {
    const first = new NextTurnSettingsController(async () => page());
    const second = new NextTurnSettingsController(async () => page());
    first.connectionChanged(true); second.connectionChanged(true);
    await first.refresh(); await second.refresh();
    first.observeSnapshot("same", "model-a", "low");
    first.setModel("same", "model-a");
    const captured = first.prepare("same");
    expect(first.prepare("other").params).toEqual({});
    expect(second.prepare("same").params).toEqual({});
    first.connectionChanged(false); first.accepted(captured);
    expect(first.snapshot("same").draft.model).toBe("model-a");
    expect(first.snapshot("same").current).toBeNull();
    expect(() => first.prepare("same")).toThrow("모델 목록");
    first.dispose(); second.dispose();
  });

  it("consumes only the sent draft and does not guess applied settings", async () => {
    const settings = new NextTurnSettingsController(async () => page());
    settings.connectionChanged(true); await settings.refresh();
    settings.observeSnapshot("t", "model-a", "low");
    settings.setModel("t", "model-a");
    const sent = settings.prepare("t");
    settings.setEffort("t", "high");
    settings.accepted(sent);
    expect(settings.snapshot("t").draft.effort).toBe("high");
    expect(settings.snapshot("t").current).toBeNull();
    const next = settings.prepare("t");
    settings.observeNotification("t", "model-a", "high");
    const confirmedRevision = settings.snapshot("t").currentRevision;
    settings.accepted(next);
    expect(settings.snapshot("t").draft).toEqual({ model: null, effort: null });
    expect(settings.snapshot("t").current).toEqual({ model: "model-a", effort: "high" });
    expect(settings.snapshot("t").currentRevision).toBe(confirmedRevision);
    settings.dispose();
  });

  it("keeps a newer settings notification ahead of an older resume snapshot", () => {
    const settings = new NextTurnSettingsController(async () => page());
    settings.connectionChanged(true);
    const revision = settings.snapshot("t").currentRevision;
    settings.observeNotification("t", "new", "high");
    settings.observeSnapshot("t", "old", "low", revision);
    expect(settings.snapshot("t").current).toEqual({ model: "new", effort: "high" });
    settings.dispose();
  });

  it.each(["ack-first", "resume-first"])("invalidates a resume snapshot overlapping an accepted model (%s)", async (order) => {
    const settings = new NextTurnSettingsController(async () => page([model("model-a"), model("model-b")]));
    settings.connectionChanged(true); await settings.refresh();
    settings.observeSnapshot("t", "model-a", "low");
    settings.setModel("t", "model-b");
    const sent = settings.prepare("t");
    const pendingResumeRevision = settings.snapshot("t").currentRevision;
    if (order === "resume-first") settings.observeSnapshot("t", "model-a", "low", pendingResumeRevision);
    settings.accepted(sent);
    if (order === "ack-first") settings.observeSnapshot("t", "model-a", "low", pendingResumeRevision);
    expect(settings.snapshot("t").current).toBeNull();
    expect(settings.snapshot("t").currentRevision).toBeGreaterThan(pendingResumeRevision);
    expect(() => settings.setEffort("t", "high")).toThrow("지원하는");
    settings.dispose();
  });

  it("rejects repeated pagination cursors and late responses from an old connection", async () => {
    const list = vi.fn().mockResolvedValueOnce(page([], "same")).mockResolvedValueOnce(page([], "same"));
    const settings = new NextTurnSettingsController(list);
    settings.connectionChanged(true); await settings.refresh();
    expect(settings.catalogSnapshot().error).toContain("반복");
    const pending = deferred<unknown>(); list.mockReturnValueOnce(pending.promise);
    const loading = settings.refresh(); settings.connectionChanged(false);
    pending.resolve(page()); await loading;
    expect(settings.catalogSnapshot().phase).toBe("offline");
    settings.dispose();
  });

  it("does not reactivate a disconnected catalog after a late initialize response", () => {
    const settings = new NextTurnSettingsController(async () => page());
    const initializing = settings.connectionChanged(false);
    settings.connectionChanged(false);
    settings.connectionChanged(true, initializing);
    expect(settings.catalogSnapshot().phase).toBe("offline");
    settings.dispose();
  });
});
