import { describe, expect, it } from "vitest";
import type { GitChangedFile } from "../src/shared/bridge";
import { filterChangedFiles } from "../src/renderer/conversation/workingTreeNavigation";

const files: GitChangedFile[] = [
  { path: "src/new.ts", originalPath: "src/old.ts", status: "R ", untracked: false, directory: false, added: 2, removed: 1 },
  { path: "src/other.ts", originalPath: null, status: " M", untracked: false, directory: false, added: 0, removed: 1 },
  { path: "docs/", originalPath: null, status: "??", untracked: true, directory: true, added: null, removed: null },
];

describe("working tree navigation", () => {
  it("matches both sides of a rename without altering the original list", () => {
    expect(filterChangedFiles(files, " OLD.TS ")).toEqual([files[0]]);
    expect(filterChangedFiles(files, "new.ts")).toEqual([files[0]]);
    expect(filterChangedFiles(files, "src/")).toEqual(files.slice(0, 2));
    expect(filterChangedFiles(files, "nothing")).toEqual([]);
    expect(filterChangedFiles(files, "")).toEqual(files);
  });
});
