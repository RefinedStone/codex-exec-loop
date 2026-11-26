import { describe, expect, it } from "vitest";

import { COMMANDS, commandForKey, PALETTE_ORDER, shortcutIdentity } from "../src/shared/commands";

describe("command table (#2840)", () => {
  it("has one entry per id and one command per shortcut", () => {
    const ids = COMMANDS.map((spec) => spec.id);
    expect(new Set(ids).size).toBe(ids.length);
    const shortcuts = COMMANDS.flatMap((spec) => shortcutIdentity(spec).map((key) => [key, spec.id] as const));
    const owners = new Map<string, string>();
    for (const [key, id] of shortcuts) {
      expect(owners.get(key), `${key} is used by ${owners.get(key)} and ${id}`).toBeUndefined();
      owners.set(key, id);
    }
  });

  it("lists every palette command once with an icon", () => {
    expect(new Set(PALETTE_ORDER).size).toBe(PALETTE_ORDER.length);
    for (const id of PALETTE_ORDER) {
      expect(COMMANDS.find((spec) => spec.id === id)?.paletteIcon, id).toBeTruthy();
    }
  });

  it("finds commands by key position, including the second terminal key", () => {
    expect(commandForKey("KeyK")).toBe("palette");
    expect(commandForKey("Backquote")).toBe("terminal");
    expect(commandForKey("KeyQ")).toBeNull();
  });
});
