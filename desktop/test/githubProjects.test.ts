// GitHub Project 패널(R11)의 필터, 묶음, 폴더 찾기이다.
import { describe, expect, it } from "vitest";

import type { GitHubProjectItem } from "../src/shared/bridge";
import { filterByRepository, findFolderForRepository, groupByStatus, repositoriesOf } from "../src/renderer/work/githubProjects";

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
