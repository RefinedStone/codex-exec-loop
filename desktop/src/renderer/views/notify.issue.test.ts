import { afterEach, describe, expect, it, vi } from "vitest";
import { useSettings } from "../session/settingsStore";
import { DEFAULT_SETTINGS } from "../../shared/settings";
import { notifyIssueDecision } from "./notify";

// Browser notification behavior belongs to the renderer's DOM type boundary.
afterEach(() => {
  useSettings.setState(DEFAULT_SETTINGS);
  vi.unstubAllGlobals();
});

describe("private issue notifications", () => {
  it.each([
    ["arbiter_applied", "중재자가 이슈 결정에 답변했습니다. 이슈 탭에서 확인해 주세요."],
    ["arbiter_waiting", "이슈 결정에 사람의 답변이 필요합니다. 이슈 탭에서 확인해 주세요."],
  ] as const)("uses fixed %s text and a decision tag without including model or issue content", async (outcome, body) => {
    const shown: Array<{ title: string; options: NotificationOptions }> = [];
    class FakeNotification {
      static permission = "granted";
      constructor(title: string, options: NotificationOptions) { shown.push({ title, options }); }
    }
    vi.stubGlobal("Notification", FakeNotification);
    vi.stubGlobal("window", { akra: { isAppFocused: async () => false } });
    vi.stubGlobal("document", { hasFocus: () => false });
    const current = vi.fn(() => true);
    notifyIssueDecision(vi.fn(), "akra:issue-decision:repo:decision-1", outcome, current);
    await vi.waitFor(() => expect(shown).toHaveLength(1));
    expect(shown[0]).toEqual({ title: "Akra", options: expect.objectContaining({ body, tag: "akra:issue-decision:repo:decision-1" }) });
    expect(current).toHaveBeenCalledOnce();
  });

  it.each([false, true, "arbiter_waiting"] as const)("does not show a late %s request notification when an answer wins while the OS focus check is pending", async (notice) => {
    const shown = vi.fn();
    class FakeNotification { static permission = "granted"; constructor() { shown(); } }
    let focusReply!: (focused: boolean) => void;
    const focused = new Promise<boolean>((resolve) => { focusReply = resolve; });
    vi.stubGlobal("Notification", FakeNotification);
    vi.stubGlobal("window", { akra: { isAppFocused: () => focused } });
    vi.stubGlobal("document", { hasFocus: () => false });
    let current = true;
    notifyIssueDecision(vi.fn(), "tag", notice, () => current);
    current = false;
    focusReply(false);
    await Promise.resolve();
    expect(shown).not.toHaveBeenCalled();
  });

  it("uses fixed reminder text and the existing decision notification tag", async () => {
    const shown: Array<{ title: string; options: NotificationOptions }> = [];
    class FakeNotification {
      static permission = "granted";
      constructor(title: string, options: NotificationOptions) { shown.push({ title, options }); }
    }
    vi.stubGlobal("Notification", FakeNotification);
    vi.stubGlobal("window", { akra: { isAppFocused: async () => false } });
    vi.stubGlobal("document", { hasFocus: () => false });
    const open = vi.fn();
    notifyIssueDecision(open, "akra:issue-decision:repo:decision-1", true);
    await vi.waitFor(() => expect(shown).toHaveLength(1));
    expect(shown[0]).toEqual({ title: "Akra", options: expect.objectContaining({
      body: "이슈 결정의 응답 기한이 다가옵니다. 이슈 탭에서 확인해 주세요.",
      tag: "akra:issue-decision:repo:decision-1",
    }) });
    expect(open).not.toHaveBeenCalled();
  });

  it("shows only fixed text in a background notification and opens only on click", async () => {
    const shown: Array<{ title: string; options: NotificationOptions; notification: FakeNotification }> = [];
    class FakeNotification {
      static permission = "granted";
      onclick: (() => void) | null = null;
      close = vi.fn();
      constructor(title: string, options: NotificationOptions) { shown.push({ title, options, notification: this }); }
    }
    const focus = vi.fn();
    const open = vi.fn();
    vi.stubGlobal("Notification", FakeNotification);
    vi.stubGlobal("window", { akra: { isAppFocused: async () => false }, focus });
    vi.stubGlobal("document", { hasFocus: () => false });
    notifyIssueDecision(open, "akra:issue-decision:repo:decision-1");
    await vi.waitFor(() => expect(shown).toHaveLength(1));
    expect(shown[0]).toMatchObject({
      title: "Akra",
      options: { body: "이슈에 새 결정 요청이 있습니다. 이슈 탭에서 확인해 주세요.", tag: "akra:issue-decision:repo:decision-1" },
    });
    expect(open).not.toHaveBeenCalled();
    shown[0]!.notification.onclick?.();
    expect(focus).toHaveBeenCalledOnce();
    expect(open).toHaveBeenCalledOnce();
    expect(shown[0]!.notification.close).toHaveBeenCalledOnce();
  });

  it("does not notify a focused app even if general notifications are always enabled", async () => {
    const show = vi.fn();
    class FakeNotification { static permission = "granted"; constructor() { show(); } }
    vi.stubGlobal("Notification", FakeNotification);
    vi.stubGlobal("window", { akra: { isAppFocused: async () => true } });
    vi.stubGlobal("document", { hasFocus: () => true });
    useSettings.setState({ notifications: { mode: "always", sound: false } });
    notifyIssueDecision(vi.fn(), "tag");
    await Promise.resolve();
    expect(show).not.toHaveBeenCalled();
    useSettings.setState({ notifications: { mode: "off", sound: false } });
    notifyIssueDecision(vi.fn(), "tag");
    await Promise.resolve();
    expect(show).not.toHaveBeenCalled();
  });
});
