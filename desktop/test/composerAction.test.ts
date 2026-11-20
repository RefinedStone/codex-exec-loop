import { beforeEach, describe, expect, it, vi } from "vitest";
import { isSentComposerDraft, runComposerAction, useComposerRequests } from "../src/renderer/conversation/composerAction";

describe("composer requests across navigation", () => {
  beforeEach(() => useComposerRequests.setState({ pendingKeys: [] }));

  it("keeps A pending across A → B → A and preserves the new draft after A completes", async () => {
    let finish!: () => void;
    const pending = new Promise<void>((resolve) => { finish = resolve; });
    const sendAgain = vi.fn(async () => undefined);
    const drafts: Record<string, string> = { A: "first message" };
    const first = runComposerAction("A", async () => {
      await pending;
      if (isSentComposerDraft(drafts.A, "first message")) drafts.A = "";
    });
    // B의 입력창은 독립적으로 사용할 수 있고, 새 A 입력창은 아직 같은 요청의 잠금을 본다.
    await runComposerAction("B", async () => undefined);
    expect(useComposerRequests.getState().pendingKeys).toEqual(["A"]);
    drafts.A = "new unsent message";
    await runComposerAction("A", sendAgain);
    expect(sendAgain).not.toHaveBeenCalled();
    finish();
    await first;
    expect(drafts.A).toBe("new unsent message");
    expect(useComposerRequests.getState().pendingKeys).toEqual([]);
    await runComposerAction("A", sendAgain);
    expect(sendAgain).toHaveBeenCalledOnce();
  });

  it("releases the request lock after failure so retry can run", async () => {
    await expect(runComposerAction("A", async () => { throw new Error("offline"); })).rejects.toThrow("offline");
    expect(useComposerRequests.getState().pendingKeys).toEqual([]);
    const retry = vi.fn(async () => undefined);
    await runComposerAction("A", retry);
    expect(retry).toHaveBeenCalledOnce();
  });

  it("clears only the exact sent draft", () => {
    expect(isSentComposerDraft("sent", "sent")).toBe(true);
    expect(isSentComposerDraft("new", "sent")).toBe(false);
    expect(isSentComposerDraft(undefined, "sent")).toBe(false);
    expect(isSentComposerDraft("sent ", "sent")).toBe(false);
  });
});
