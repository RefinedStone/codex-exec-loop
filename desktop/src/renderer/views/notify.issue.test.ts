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
