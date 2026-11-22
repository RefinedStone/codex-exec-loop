import { afterEach, describe, expect, it, vi } from "vitest";
import { EMPTY_KERNEL_VIEW, type IssueDecisionAttention } from "../src/renderer/kernel/kernelConnection";
import { issueDecisionAttentionEvents } from "../src/renderer/session/attention";
import { useSettings } from "../src/renderer/session/settingsStore";
import { notifyIssueDecision } from "../src/renderer/views/notify";
import { DEFAULT_SETTINGS } from "../src/shared/settings";

const event: IssueDecisionAttention = {
  workspace: "/repo", kernel: "repo", decisionId: "decision-1",
  issue: { provider: "github", server_id: "github.com", repository_id: "R_repo", issue_id: "I_1", project_scope: { workspace_dir: "/repo" } },
};
const state = (events: IssueDecisionAttention[] = []) => ({
  connection: { phase: "connected" as const, endpoint: "fixture" },
  kernel: { ...EMPTY_KERNEL_VIEW, phase: "attached" as const, kernel: "repo", workspace: "/repo", issueAttention: events },
});

afterEach(() => {
  useSettings.setState(DEFAULT_SETTINGS);
  vi.unstubAllGlobals();
});

describe("private issue attention", () => {
  it("reports only a newly received decision and suppresses duplicate and reconnect population", () => {
    expect(issueDecisionAttentionEvents(state(), state([event]))).toEqual([event]);
    expect(issueDecisionAttentionEvents(state([event]), state([{ ...event }]))).toEqual([]);
    expect(issueDecisionAttentionEvents({ ...state(), connection: { phase: "disconnected", endpoint: "fixture" } }, state([event]))).toEqual([]);
    expect(issueDecisionAttentionEvents({ ...state(), kernel: { ...EMPTY_KERNEL_VIEW, phase: "attaching" } }, state([event]))).toEqual([]);
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
