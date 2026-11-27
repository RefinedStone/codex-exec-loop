import type { GatewayRequest } from "../src/renderer/work/notionHub";
import { describe, expect, it, vi } from "vitest";
import { IsolatedWorkspaceStarter, type PreparedWorkspace } from "../src/renderer/session/isolatedWorkspace";

const head = "a".repeat(40);
const graph = { repository: true, stale: false, head, refs: [], rows: [] };
const record = { requestId: "request-1", repositoryDir: "/repo", workspaceDir: "/repo-worktree", branch: "akra-work", baseCommit: head, state: "prepared", reason: null };

describe("isolated conversation workspace", () => {
  it("uses the same preparation id after response loss and never repeats the uncertain start", async () => {
    let prepares = 0;
    const request = vi.fn(async (method: string, params?: unknown) => {
      if (method === "akra/git/graph") return graph;
      if (method.endsWith("/get")) return prepares ? record : null;
      expect(params).toEqual({ cwd: "/repo", requestId: record.requestId, baseCommit: head });
      prepares++;
      throw new Error("response lost");
    });
    const starter = new IsolatedWorkspaceStarter(request as GatewayRequest, () => record.requestId);
    const open = vi.fn(async (_workspace: PreparedWorkspace, markStartRequested: () => void) => {
      markStartRequested();
      throw new Error("start response lost");
    });
    await expect(starter.start("/repo", open)).rejects.toThrow("response lost");
    await expect(starter.start("/repo", open)).rejects.toThrow("start response lost");
    await expect(starter.start("/repo", open)).rejects.toThrow("스레드 생성 결과가 불명");
    expect(prepares).toBe(1);
    expect(open).toHaveBeenCalledExactlyOnceWith(record, expect.any(Function));
  });

  it("reuses a prepared workspace when the caller cancels before sending thread/start", async () => {
    const request = vi.fn(async (method: string) => method === "akra/git/graph" ? graph : record);
    const starter = new IsolatedWorkspaceStarter(request as GatewayRequest, () => record.requestId);
    const cancelled = vi.fn(async () => { throw new Error("navigation changed"); });
    await expect(starter.start("/repo", cancelled)).rejects.toThrow("navigation changed");
    const retry = vi.fn(async (_workspace: PreparedWorkspace, markStartRequested: () => void) => {
      markStartRequested();
      return "thread";
    });
    await expect(starter.start("/repo", retry)).resolves.toBe("thread");
    expect(retry).toHaveBeenCalledExactlyOnceWith(record, expect.any(Function));
    expect(request.mock.calls.map(([method]) => method)).toEqual([
      "akra/git/graph", "akra/conversationWorkspace/get", "akra/conversationWorkspace/get",
    ]);
  });

  it("blocks duplicate clicks and refuses unverified workspace coordinates", async () => {
    const request = vi.fn(async (method: string) => method === "akra/git/graph" ? graph : { ...record, workspaceDir: "/repo" });
    const starter = new IsolatedWorkspaceStarter(request as GatewayRequest, () => record.requestId);
    const open = vi.fn(async () => "thread");
    const first = starter.start("/repo", open);
    await expect(starter.start("/repo", open)).rejects.toThrow("진행 중");
    await expect(first).rejects.toThrow("검증하지 못했습니다");
    expect(open).not.toHaveBeenCalled();
  });
});
