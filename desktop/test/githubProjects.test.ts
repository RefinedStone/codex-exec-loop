// GitHub Project 패널(R11)의 필터, 묶음, 폴더 찾기이다.
import { describe, expect, it } from "vitest";

import type { GitHubProjectItem } from "../src/shared/bridge";
import { addPrompt } from "../src/renderer/conversation/composerKeys";
import { filterByRepository, findFolderForRepository, groupByStatus, ISSUE_HANDOFF_COPY_METHOD, readIssueHandoffPrompt, repositoriesOf } from "../src/renderer/work/githubProjects";

const item = (number: number, repository: string | null, status: string | null): GitHubProjectItem => ({
  kind: repository ? "issue" : "draft",
  title: `Item ${number}`,
  url: repository ? `https://github.com/${repository}/issues/${number}` : null,
  repository,
  number: repository ? number : null,
  status,
});

describe("GitHub Project panel rules", () => {
  const items = [item(1, "sample/widgets", "Todo"), item(2, "Sample/Lib", null), item(3, null, "Todo"), item(4, "sample/lib", "Done")];

  it("filters by repository without case, and keeps everything for all", () => {
    expect(filterByRepository(items, null)).toHaveLength(4);
    expect(filterByRepository(items, "sample/lib").map((found) => found.title)).toEqual(["Item 2", "Item 4"]);
    expect(filterByRepository(items, "other/repo")).toEqual([]);
  });

  it("lists each repository once, by name", () => {
    expect(repositoriesOf(items)).toEqual(["Sample/Lib", "sample/widgets"]);
  });

  it("groups by status in project order with no status last", () => {
    expect(groupByStatus(items).map((group) => [group.status, group.items.map((found) => found.title)])).toEqual([
      ["Todo", ["Item 1", "Item 3"]],
      ["Done", ["Item 4"]],
      [null, ["Item 2"]],
    ]);
  });

  it("finds the first folder whose origin is the repository, and skips folders it cannot read", async () => {
    const origins: Record<string, string | null> = { "/a": "sample/widgets", "/b": "Sample/Lib", "/c": "sample/lib" };
    const asked: string[] = [];
    const read = async (cwd: string) => {
      asked.push(cwd);
      if (cwd === "/broken") throw new Error("not a folder");
      return origins[cwd] ?? null;
    };
    expect(await findFolderForRepository("sample/lib", ["/broken", "/a", "/b", "/c", "/a"], read)).toBe("/b");
    expect(asked).toEqual(["/broken", "/a", "/b"]);
    expect(await findFolderForRepository("other/repo", ["/a", "/a", "/c"], read)).toBeNull();
  });
});

describe("issue hand-off prompt (#2428)", () => {
  // 시험용으로 만든 값이다. 실제 비밀이 아니다. 원격 이슈에는 이 값들이 있고, Gateway 의 공유본에는 없다.
  const SYNTHETIC_TOKEN = "ghp_SyntheticCanary0000";
  const RESTORED_PLAINTEXT = "/home/synthetic-user/app";
  const URL = "https://github.com/sample/widgets/issues/42";

  it("builds the message from the Gateway shared copy only, and the prompt history keeps nothing else", async () => {
    const calls: Array<{ method: string; params: unknown }> = [];
    const request = async <T,>(method: string, params: unknown): Promise<T> => {
      calls.push({ method, params });
      return { sharedCopy: { title: "Crash on start", body: "token=[removed] 로 [[akra-mask:1]] 에서 실패" }, rawBody: `${SYNTHETIC_TOKEN} ${RESTORED_PLAINTEXT}`, comments: [SYNTHETIC_TOKEN] } as T;
    };
    const prompt = await readIssueHandoffPrompt(request, URL, "sample/widgets", 42, "/work/widgets");

    expect(calls).toEqual([{ method: ISSUE_HANDOFF_COPY_METHOD, params: { owner: "sample", repository: "widgets", number: 42, cwd: "/work/widgets" } }]);
    expect(prompt).toContain("Closes sample/widgets#42");
    expect(prompt).toContain(`${URL} (Crash on start)`);
    expect(prompt).toContain("token=[removed] 로 [[akra-mask:1]] 에서 실패");
    expect(prompt).not.toContain("댓글");
    const history = JSON.stringify(addPrompt([], prompt));
    for (const text of [prompt, history]) {
      expect(text).not.toContain(SYNTHETIC_TOKEN);
      expect(text).not.toContain(RESTORED_PLAINTEXT);
    }
  });

  it("marks an empty body, clips long text, and asks without a folder", async () => {
    let body = "";
    const calls: unknown[] = [];
    const request = async <T,>(_method: string, params: unknown): Promise<T> => (calls.push(params), { sharedCopy: { title: "T", body } } as T);
    expect(await readIssueHandoffPrompt(request, URL, "sample/widgets", 42, null)).toContain("(본문 없음)");
    expect(calls[0]).toEqual({ owner: "sample", repository: "widgets", number: 42 });
    body = "x".repeat(30_000);
    expect((await readIssueHandoffPrompt(request, URL, "sample/widgets", 42, null)).length).toBeLessThanOrEqual(20_000 + 10);
  });

  it("does not make a message when the Gateway refuses", async () => {
    const request = async <T,>(): Promise<T> => Promise.reject(new Error("the issue cannot be handed off"));
    await expect(readIssueHandoffPrompt(request, URL, "sample/widgets", 42, null)).rejects.toThrow("cannot be handed off");
  });
});
