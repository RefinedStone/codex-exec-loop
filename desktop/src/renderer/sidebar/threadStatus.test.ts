import { describe, expect, it } from "vitest";

import type { WorkspaceThread } from "../session/desktopStore";
import { archiveBlocked, threadMark } from "./threadStatus";

const thread = { id: "t1", title: "작업" } as WorkspaceThread;

describe("archiveBlocked", () => {
  it("blocks a running turn that waits for approval although the mark shows approval", () => {
    const source = { activeThreadIds: ["t1"], unreadThreadIds: [], pendingApprovals: [{ threadId: "t1" }] } as never;
    expect(threadMark(thread, source)).toBe("approval");
    expect(archiveBlocked("t1", source)).toBe(true);
  });

  it("allows an idle thread", () => {
    expect(archiveBlocked("t1", { activeThreadIds: ["t2"] })).toBe(false);
  });
});
