import { describe, expect, it } from "vitest";

import { PALETTE_ORDER } from "../src/shared/commands";
import { matchingSlashCommands, slashCommandQuery } from "../src/renderer/conversation/slashCommands";

describe("composer slash commands (#2837)", () => {
  it("uses the palette command table and accepts English ids and Korean titles", () => {
    expect(matchingSlashCommands("").map((spec) => spec.id)).toEqual(PALETTE_ORDER);
    expect(matchingSlashCommands("SETT").map((spec) => spec.id)).toEqual(["settings"]);
    expect(matchingSlashCommands("현황").map((spec) => spec.id)).toEqual(["fleet"]);
    expect(matchingSlashCommands("does-not-exist")).toEqual([]);
  });

  it("only handles a complete command draft with the caret at the end", () => {
    expect(slashCommandQuery("/", 1)).toBe("");
    expect(slashCommandQuery("/REVIEW", 7)).toBe("review");
    expect(slashCommandQuery("/설정", 3)).toBe("설정");
    for (const text of ["read /review", "/usr/local", "/review this", "/review\n", "//comment", ""]) {
      expect(slashCommandQuery(text, text.length), text).toBeNull();
    }
    expect(slashCommandQuery("/review", 3)).toBeNull();
    expect(slashCommandQuery("/review", 0, 7)).toBeNull();
  });
});
