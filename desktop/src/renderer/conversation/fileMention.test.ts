import { describe, expect, it } from "vitest";

import { activeMention, applyMention } from "./fileMention";

describe("file mention (#2837)", () => {
  it("finds the @word before the caret only at a word start", () => {
    expect(activeMention("look at @src/ma", 15)).toEqual({ start: 8, query: "src/ma" });
    expect(activeMention("@", 1)).toEqual({ start: 0, query: "" });
    expect(activeMention("mail me@example.com", 19)).toBeNull();
    expect(activeMention("@done here", 10)).toBeNull();
  });

  it("replaces the @word with the chosen path and keeps the rest", () => {
    const at = activeMention("open @ma now", 8)!;
    expect(applyMention("open @ma now", at, 8, "src/main.ts")).toEqual({ text: "open src/main.ts  now", caret: 17 });
    expect(applyMention("@a", activeMention("@a", 2)!, 2, "my file.txt").text).toBe('"my file.txt" ');
  });
});
