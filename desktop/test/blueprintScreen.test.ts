// 블루프린트 화면(R10)이 쓰는 계산이다: 턴 아이템 묶기, 스레드 상태 표시, 계획과 컨텍스트 사용량.
import { beforeEach, describe, expect, it } from "vitest";

import type { PendingApproval } from "../src/renderer/conversation/approvals";
import { applyNotification, contextPercent, type Transcript } from "../src/renderer/conversation/transcript";
import { activitySummary, formatDuration, stepKind, stepLabel, stepStatus, type StepItem, turnBlocks } from "../src/renderer/conversation/turnBlocks";
import { useDesktopStore, type WorkspaceThread } from "../src/renderer/session/desktopStore";
import { allThreads, relativeAge, threadMark } from "../src/renderer/sidebar/threadStatus";
import type { ThreadItem } from "../src/shared/protocol/generated/v2/ThreadItem";

const item = (fields: Record<string, unknown>) => fields as unknown as ThreadItem;
const command = (id: string, fields: Record<string, unknown> = {}) =>
  item({ type: "commandExecution", id, command: "npm test", commandActions: [], status: "completed", aggregatedOutput: null, durationMs: null, ...fields }) as StepItem;

describe("turnBlocks", () => {
  it("joins tool calls that follow each other and keeps the message order", () => {
    const blocks = turnBlocks([
      item({ type: "userMessage", id: "u", content: [{ type: "text", text: "고쳐줘" }, { type: "image" }] }),
      item({ type: "reasoning", id: "r0", summary: [] }),
      item({ type: "reasoning", id: "r1", summary: ["먼저 읽는다", "그다음 고친다"] }),
      command("c1"),
      item({ type: "fileChange", id: "f1", changes: [{ path: "a.ts" }], status: "completed" }),
      item({ type: "agentMessage", id: "a1", text: "중간 보고" }),
      item({ type: "mcpToolCall", id: "m1", server: "notion", tool: "search", status: "failed" }),
      item({ type: "plan", id: "p1", text: "계획" }),
    ]);
    expect(blocks.map((block) => block.type)).toEqual(["user", "reasoning", "activity", "agent", "activity", "other"]);
    expect(blocks[0]).toMatchObject({ text: "고쳐줘\n[image]" });
    expect(blocks[1]).toMatchObject({ text: "먼저 읽는다\n그다음 고친다" });
    expect(blocks[2]).toMatchObject({ steps: [{ id: "c1" }, { id: "f1" }] });
  });

  it("names a step from its command action", () => {
    const read = command("c", { commandActions: [{ type: "read", command: "cat a.ts", name: "a.ts", path: "/w/a.ts" }] });
    const search = command("c", { commandActions: [{ type: "search", command: "rg x", query: "x", path: "src" }] });
    const mixed = command("c", { commandActions: [{ type: "read", command: "cat a", name: "a", path: "a" }, { type: "unknown", command: "wc" }] });
    expect([stepKind(read), stepLabel(read)]).toEqual(["read", "a.ts"]);
    expect([stepKind(search), stepLabel(search)]).toEqual(["search", "x · src"]);
    expect([stepKind(mixed), stepLabel(mixed)]).toEqual(["command", "npm test"]);
    expect(activitySummary([read, search, mixed, mixed])).toBe("파일 1개 읽음 · 검색 1회 · 명령 2개 실행");
  });

  it("shows a declined step as failed", () => {
    expect(stepStatus(command("c", { status: "inProgress" }))).toBe("running");
    expect(stepStatus(command("c", { status: "declined" }))).toBe("failed");
  });

  it("writes short durations", () => {
    expect([formatDuration(400), formatDuration(12_300), formatDuration(134_000)]).toEqual(["0.4초", "12초", "2분 14초"]);
  });
});

describe("thread marks", () => {
  const thread = (id: string, fields: Partial<WorkspaceThread> = {}): WorkspaceThread => ({ id, title: id, updatedAt: 0, branch: null, failed: false, ...fields });
  const approval = (threadId: string) => ({ threadId }) as PendingApproval;

  it("puts approval before running, failed and unread", () => {
    const all = { activeThreadIds: ["t"], unreadThreadIds: ["t"], pendingApprovals: [approval("t")] };
    expect(threadMark(thread("t", { failed: true }), all)).toBe("approval");
    expect(threadMark(thread("t", { failed: true }), { ...all, pendingApprovals: [] })).toBe("running");
    expect(threadMark(thread("t", { failed: true }), { activeThreadIds: [], unreadThreadIds: ["t"], pendingApprovals: [] })).toBe("failed");
    expect(threadMark(thread("t"), { activeThreadIds: [], unreadThreadIds: ["t"], pendingApprovals: [] })).toBe("unread");
    expect(threadMark(thread("t"), { activeThreadIds: [], unreadThreadIds: [], pendingApprovals: [] })).toBe("idle");
  });

  it("lists threads of every folder, newest first", () => {
    const folders = [
      { cwd: "/a", threads: [thread("old", { updatedAt: 1 })] },
      { cwd: "/b", threads: [thread("new", { updatedAt: 9 })] },
    ];
    expect(allThreads(folders).map((listed) => [listed.id, listed.cwd])).toEqual([["new", "/b"], ["old", "/a"]]);
  });

  it("writes the age from seconds", () => {
    const now = 1_000_000_000;
    const age = (secondsAgo: number) => relativeAge(now / 1000 - secondsAgo, now);
    expect([age(5), age(180), age(7200), age(3 * 86_400)]).toEqual(["방금", "3분", "2시간", "3일"]);
  });
});

describe("unread threads", () => {
  beforeEach(() => useDesktopStore.setState({ activeThreadIds: [], unreadThreadIds: [], openThreadId: null, threadVisible: true, folders: [] }));

  it("marks a thread unread when its turn ends while it is not open, and clears it on open", () => {
    const store = useDesktopStore.getState();
    store.setThreadActive("t1", true);
    store.setThreadActive("t1", false);
    expect(useDesktopStore.getState().unreadThreadIds).toEqual(["t1"]);
    store.setOpenThread("t1");
    expect(useDesktopStore.getState().unreadThreadIds).toEqual([]);
  });

  it("does not mark the open thread or a thread that was not running", () => {
    const store = useDesktopStore.getState();
    store.setOpenThread("t1");
    store.setThreadActive("t1", true);
    store.setThreadActive("t1", false);
    store.setThreadActive("t2", false);
    expect(useDesktopStore.getState().unreadThreadIds).toEqual([]);
  });
});

describe("store values the review fixes added", () => {
  beforeEach(() => useDesktopStore.setState({ activeThreadIds: [], unreadThreadIds: [], openThreadId: null, threadVisible: true, folders: [] }));

  it("marks the open thread unread when its turn ends while another view is on screen", () => {
    const store = useDesktopStore.getState();
    store.setOpenThread("t1");
    useDesktopStore.setState({ threadVisible: false });
    store.setThreadActive("t1", true);
    store.setThreadActive("t1", false);
    expect(useDesktopStore.getState().unreadThreadIds).toEqual(["t1"]);
  });

  it("follows the failed state of a thread from status changes", () => {
    const thread = { id: "t1", title: "t", updatedAt: 0, branch: null, failed: false };
    useDesktopStore.setState({ folders: [{ cwd: "/w", threads: [thread] }] });
    const store = useDesktopStore.getState();
    store.setThreadFailed("t1", true);
    expect(useDesktopStore.getState().folders[0]?.threads[0]?.failed).toBe(true);
    const failedFolders = useDesktopStore.getState().folders;
    store.setThreadFailed("t1", true);
    expect(useDesktopStore.getState().folders).toBe(failedFolders);
    store.setThreadFailed("t1", false);
    expect(useDesktopStore.getState().folders[0]?.threads[0]?.failed).toBe(false);
  });
});

describe("plan and token usage notifications", () => {
  const transcript: Transcript = {
    threadId: "th",
    title: "t",
    cwd: "/w",
    status: null,
    olderTurnsCursor: null,
    tokenUsage: null,
    turns: [{ id: "tu", status: "inProgress", errorMessage: null, items: [], diff: null, plan: null, startedAt: null, durationMs: null }],
  };

  it("keeps the latest plan on its turn", () => {
    const plan = [{ step: "읽는다", status: "completed" }, { step: "고친다", status: "inProgress" }];
    const updated = applyNotification(transcript, { method: "turn/plan/updated", params: { threadId: "th", turnId: "tu", explanation: null, plan } });
    expect(updated.turns[0]?.plan).toEqual(plan);
  });

  it("computes the context share from the last request", () => {
    expect(contextPercent(transcript)).toBeNull();
    const usage = (modelContextWindow: number | null) => ({ total: { totalTokens: 900_000 }, last: { totalTokens: 50_000 }, modelContextWindow });
    const withUsage = (window: number | null) => applyNotification(transcript, { method: "thread/tokenUsage/updated", params: { threadId: "th", turnId: "tu", tokenUsage: usage(window) } });
    expect(contextPercent(withUsage(200_000))).toBe(25);
    expect(contextPercent(withUsage(null))).toBeNull();
  });
});
