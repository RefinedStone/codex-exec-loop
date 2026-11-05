import { describe, expect, it } from "vitest";

import { diffFileFromChange, parseGitDiff, totalCounts, turnDiffFiles } from "../src/renderer/conversation/diff";
import { applyNotification, type Transcript } from "../src/renderer/conversation/transcript";
import type { ThreadItem } from "../src/shared/protocol/generated/v2/ThreadItem";

describe("fileChange item diffs", () => {
  it("shows an added file's content as added lines", () => {
    const file = diffFileFromChange({ path: "a.txt", kind: { type: "add" }, diff: "one\r\ntwo\n" });
    expect(file).toMatchObject({ kind: "add", added: 2, removed: 0 });
    expect(file.lines).toEqual([
      { kind: "add", text: "one" },
      { kind: "add", text: "two" },
    ]);
  });

  it("shows a deleted file's content as removed lines", () => {
    const file = diffFileFromChange({ path: "a.txt", kind: { type: "delete" }, diff: "gone\n" });
    expect(file).toMatchObject({ kind: "delete", added: 0, removed: 1 });
  });

  it("reads an update as hunks without file headers, keeping the move path", () => {
    const diff = "@@ -1,2 +1,2 @@\n keep\n-old\n+new\n\\ No newline at end of file\n";
    const file = diffFileFromChange({ path: "a.ts", kind: { type: "update", move_path: "b.ts" }, diff });
    expect(file).toMatchObject({ kind: "update", movePath: "b.ts", added: 1, removed: 1 });
    expect(file.lines.map((line) => line.kind)).toEqual(["hunk", "context", "del", "add", "meta"]);
    expect(file.lines[1]!.text).toBe("keep");
  });
});

describe("turn diff (git format)", () => {
  const diff = [
    "diff --git a/src/a.ts b/src/a.ts",
    "index 1111111..2222222 100644",
    "--- a/src/a.ts",
    "+++ b/src/a.ts",
    "@@ -1 +1 @@",
    "-const a = 1;",
    "+const a = 2;",
    "diff --git a/new.txt b/new.txt",
    "new file mode 100644",
    "index 0000000..3333333",
    "--- /dev/null",
    "+++ b/new.txt",
    "@@ -0,0 +1,2 @@",
    "+hello",
    "+--- not a header",
    "diff --git a/old.txt b/old.txt",
    "deleted file mode 100644",
    "--- a/old.txt",
    "+++ /dev/null",
    "@@ -1 +0,0 @@",
    "-bye",
    "diff --git a/from.ts b/to.ts",
    "similarity index 100%",
    "rename from from.ts",
    "rename to to.ts",
    "",
  ].join("\n");

  it("splits files and reads add, delete and rename headers", () => {
    const files = parseGitDiff(diff);
    expect(files.map((file) => [file.path, file.kind, file.movePath, file.added, file.removed])).toEqual([
      ["src/a.ts", "update", null, 1, 1],
      ["new.txt", "add", null, 2, 0],
      ["old.txt", "delete", null, 0, 1],
      ["from.ts", "update", "to.ts", 0, 0],
    ]);
    // A hunk line that looks like a header stays a content line.
    expect(files[1]!.lines.at(-1)).toEqual({ kind: "add", text: "--- not a header" });
    expect(totalCounts(files)).toEqual({ added: 3, removed: 2 });
  });

  it("decodes paths git quotes, including octal UTF-8 bytes and spaces", () => {
    const quoted = [
      'diff --git "a/\\355\\225\\234 \\352\\270\\200.txt" "b/\\355\\225\\234 \\352\\270\\200.txt"',
      "new file mode 100644",
      "--- /dev/null",
      '+++ "b/\\355\\225\\234 \\352\\270\\200.txt"',
      "@@ -0,0 +1 @@",
      "+x",
      "diff --git a/with space.txt b/with space.txt",
      "--- a/with space.txt\t",
      "+++ b/with space.txt\t",
      "@@ -1 +1 @@",
      "-a",
      "+b",
      'diff --git a/plain.txt "b/tab\\there.txt"',
      "rename from plain.txt",
      'rename to "tab\\there.txt"',
      "",
    ].join("\n");
    expect(parseGitDiff(quoted).map((file) => [file.path, file.kind, file.movePath])).toEqual([
      ["한 글.txt", "add", null],
      ["with space.txt", "update", null],
      ["plain.txt", "update", "tab\there.txt"],
    ]);
  });

  it("returns no files for an empty diff", () => {
    expect(parseGitDiff("")).toEqual([]);
  });
});

describe("turn changes", () => {
  const fileChange = (id: string, status: "completed" | "declined"): ThreadItem => ({
    type: "fileChange",
    id,
    status,
    changes: [{ path: `${id}.txt`, kind: { type: "add" }, diff: "x\n" }],
  });

  it("prefers the turn diff and falls back to applied fileChange items", () => {
    const items = [fileChange("applied", "completed"), fileChange("declined", "declined")];
    expect(turnDiffFiles(null, items).map((file) => file.path)).toEqual(["applied.txt"]);
    expect(turnDiffFiles("", items)).toEqual([]);
    // A diff in a shape the parser does not know falls back to the items instead of hiding the changes.
    expect(turnDiffFiles("unknown format\n", items).map((file) => file.path)).toEqual(["applied.txt"]);
  });

  const transcript = (): Transcript => ({
    threadId: "th",
    title: "t",
    cwd: "/w",
    status: null,
    olderTurnsCursor: null,
    turns: [{ id: "tu", status: "inProgress", errorMessage: null, items: [fileChange("fc", "completed")], diff: null }],
  });

  it("keeps the latest turn/diff/updated on the turn", () => {
    const updated = applyNotification(transcript(), {
      method: "turn/diff/updated",
      params: { threadId: "th", turnId: "tu", diff: "diff --git a/x b/x\n" },
    });
    expect(updated.turns[0]!.diff).toBe("diff --git a/x b/x\n");
  });

  it("replaces a fileChange item's changes on item/fileChange/patchUpdated", () => {
    const changes = [{ path: "fc.txt", kind: { type: "update", move_path: null }, diff: "@@ -1 +1 @@\n-x\n+y\n" }];
    const updated = applyNotification(transcript(), {
      method: "item/fileChange/patchUpdated",
      params: { threadId: "th", turnId: "tu", itemId: "fc", changes },
    });
    const item = updated.turns[0]!.items[0]!;
    expect(item.type === "fileChange" && item.changes).toEqual(changes);
  });
});
