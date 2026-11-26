// 실제 blueprint 저장소의 화면 전이 회귀이다. 브라우저와 시간만 대체하고 Gateway 는 쓰지 않는다.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { attentionOf, deliveryOf, nextInTurn } from "./flow";

let model: typeof import("./store");
const thread = (id: string) => model.useBlueprint.getState().threads.find((item) => item.id === id)!;

beforeEach(async () => {
  vi.useFakeTimers();
  vi.stubGlobal("window", {
    matchMedia: () => ({ matches: false }),
    setTimeout: globalThis.setTimeout,
    clearTimeout: globalThis.clearTimeout,
  });
  vi.resetModules();
  model = await import("./store");
});

afterEach(() => {
  vi.clearAllTimers();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

async function reachApproval(threadId: string): Promise<void> {
  await vi.advanceTimersByTimeAsync(30_000);
  expect(thread(threadId).status).toBe("approval");
}

describe("blueprint delivery and unread transitions", () => {
  it("keeps push and PR actions accessible while review and verification remain", async () => {
    const { createElement } = await import("react");
    const { renderToStaticMarkup } = await import("react-dom/server");
    const { RightPanel } = await import("./RightPanel");
    const html = renderToStaticMarkup(createElement(RightPanel, { panel: "review" }));
    expect(html).toContain("검토 0/3");
    expect(html).toContain("검증 2/3");
    for (const label of ["커밋", "push", "PR 만들기"]) {
      const button = html.match(new RegExp("<button\\b[^>]*>" + label + "</button>"))?.[0];
      expect(button, label + " remains reachable").toBeDefined();
      expect(button).not.toContain("disabled");
    }
  });

  it.each([false, true])("keeps the first unseen queued turn when returning (second finished=%s)", async (secondFinished) => {
    model.openThread("t1");
    model.sendMessage("t1", "first update");
    const first = model.useBlueprint.getState().turns.t1!.at(-1)!.id;
    model.sendMessage("t1", "second update");
    model.showView("fleet");
    await reachApproval("t1");
    model.answerApproval("t1", true);
    await reachApproval("t1");
    const second = model.useBlueprint.getState().turns.t1!.at(-1)!.id;
    expect(second).not.toBe(first);
    if (secondFinished) {
      model.answerApproval("t1", true);
      await vi.advanceTimersByTimeAsync(30_000);
      expect(thread("t1").status).toBe("unread");
    }
    model.openThread("t1");
    expect(model.useBlueprint.getState().newFrom.t1).toBe(first);
    if (!secondFinished) {
      model.answerApproval("t1", true);
      await vi.advanceTimersByTimeAsync(30_000);
    }
    model.showView("fleet");
    model.openThread("t1");
    expect(model.useBlueprint.getState().newFrom.t1).toBeUndefined();
  });

  it("keeps an unresolved CI failure in the attention list after its follow-up turn finishes", async () => {
    const before = model.useBlueprint.getState().turns.t5!.length;
    model.runDelivery("t5", "checks");
    expect(model.useBlueprint.getState().handOff?.threadId).toBe("t5");
    expect(model.useBlueprint.getState().turns.t5).toHaveLength(before);
    model.sendMessage("t5", "fix the failed check");
    await reachApproval("t5");
    model.answerApproval("t5", true);
    await vi.advanceTimersByTimeAsync(30_000);
    expect(thread("t5").status).toBe("idle");
    expect(thread("t5").pullRequest?.checks).toBe("fail");
    const delivery = deliveryOf(thread("t5"), 0, model.REVIEW_FILES.length);
    expect(attentionOf(thread("t5"), undefined, delivery)).toEqual({ group: "mine", reason: "PR #412 의 CI 실패" });
    expect(nextInTurn([thread("t5")], null)).toBe("t5");
  });

  it("starts a new delivery cycle when a merged thread makes more changes", async () => {
    model.openThread("t4");
    for (const file of model.REVIEW_FILES) model.toggleViewed("t4", file.path);
    model.runDelivery("t4", "merge");
    expect(deliveryOf(thread("t4"), model.REVIEW_FILES.length, model.REVIEW_FILES.length)?.next).toBeNull();
    model.sendMessage("t4", "one more change after merge");
    await reachApproval("t4");
    expect(thread("t4")).toMatchObject({ committed: false, pushed: false, pullRequest: null });
    expect(model.useBlueprint.getState().viewed.t4).toEqual([]);
    expect(deliveryOf(thread("t4"), 0, model.REVIEW_FILES.length)?.next?.id).toBe("review");
    model.answerApproval("t4", false);
    await vi.advanceTimersByTimeAsync(30_000);
  });
});
