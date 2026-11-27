import { describe, expect, it } from "vitest";
import { diffFileFromChange } from "../src/renderer/conversation/diff";
import { diffCoordinates } from "../src/renderer/conversation/diffCoordinates";

describe("diff file coordinates", () => {
  it("keeps deletion and addition sides distinct across separated hunks", () => {
    const file = diffFileFromChange({ path: "file.ts", kind: { type: "update", move_path: null },
      diff: "@@ -10,3 +20,2 @@\n-old one\n same\n-old two\n+new one\n@@ -50 +70 @@\n-later old\n+later new\n" });
    expect(diffCoordinates(file)).toEqual([
      { text: "@@ -10,3 +20,2 @@", oldLine: 10, newLine: 20, hunk: true },
      { text: "old one", oldLine: 10, newLine: null, hunk: false },
      { text: "same", oldLine: 11, newLine: 20, hunk: false },
      { text: "old two", oldLine: 12, newLine: null, hunk: false },
      { text: "new one", oldLine: null, newLine: 21, hunk: false },
      { text: "@@ -50 +70 @@", oldLine: 50, newLine: 70, hunk: true },
      { text: "later old", oldLine: 50, newLine: null, hunk: false },
      { text: "later new", oldLine: null, newLine: 70, hunk: false },
    ]);
  });

  it.each(["add", "delete"] as const)("numbers a whole-file %s only on its existing side", (type) => {
    const file = diffFileFromChange({ path: "file.txt", kind: { type }, diff: "first\nsecond\n" });
    expect(diffCoordinates(file)).toEqual([
      { text: "first", oldLine: type === "delete" ? 1 : null, newLine: type === "add" ? 1 : null, hunk: false },
      { text: "second", oldLine: type === "delete" ? 2 : null, newLine: type === "add" ? 2 : null, hunk: false },
    ]);
  });

  it("keeps a zero-length hunk side empty and ignores EOF metadata", () => {
    const file = diffFileFromChange({ path: "file.txt", kind: { type: "update", move_path: null },
      diff: "@@ -7 +7,0 @@\n-removed\n\\ No newline at end of file\n" });
    expect(diffCoordinates(file)).toEqual([
      { text: "@@ -7 +7,0 @@", oldLine: 7, newLine: null, hunk: true },
      { text: "removed", oldLine: 7, newLine: null, hunk: false },
    ]);
  });

  it("does not invent file coordinates for a partial update without a hunk header", () => {
    const file = diffFileFromChange({ path: "file.txt", kind: { type: "update", move_path: null },
      diff: "-unknown old\n+unknown new\n+another unknown line\n" });
    expect(diffCoordinates(file).map((row) => [row.oldLine, row.newLine])).toEqual([
      [null, null], [null, null], [null, null],
    ]);
  });
});
