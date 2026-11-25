import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";

import { migrateUserData } from "../src/main/userDataMigration";

const roots: string[] = [];
function appData(): string {
  const root = mkdtempSync(join(tmpdir(), "akra-userdata-"));
  roots.push(root);
  return root;
}
function legacyWith(root: string, file: string, body: string): string {
  const legacy = join(root, "@refinedstone", "akra-desktop");
  mkdirSync(legacy, { recursive: true });
  writeFileSync(join(legacy, file), body);
  return legacy;
}
afterEach(() => { for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true }); });

describe("migrateUserData", () => {
  it("moves the legacy folder once and removes the empty scope folder", () => {
    const root = appData();
    const legacy = legacyWith(root, "app-settings.json", "{\"zoom\":1}");
    const current = join(root, "Akra ADE");

    expect(migrateUserData(root, current)).toEqual({ kind: "moved", from: legacy, to: current });
    expect(readFileSync(join(current, "app-settings.json"), "utf8")).toBe("{\"zoom\":1}");
    expect(existsSync(join(root, "@refinedstone"))).toBe(false);
    expect(migrateUserData(root, current)).toEqual({ kind: "none" });
  });

  it("never overwrites an existing new folder", () => {
    const root = appData();
    const legacy = legacyWith(root, "window-state.json", "old");
    const current = join(root, "Akra ADE");
    mkdirSync(current);
    writeFileSync(join(current, "window-state.json"), "new");

    expect(migrateUserData(root, current)).toEqual({ kind: "both-exist", legacy, current });
    expect(readFileSync(join(current, "window-state.json"), "utf8")).toBe("new");
    expect(readFileSync(join(legacy, "window-state.json"), "utf8")).toBe("old");
  });

  it("keeps using the legacy folder when the move fails", () => {
    const root = appData();
    const legacy = legacyWith(root, "Cookies", "c");
    const current = join(root, "Akra ADE");
    const busy = { existsSync, readdirSync: () => [], rmdirSync: () => {}, renameSync: () => { throw new Error("EBUSY"); } };

    expect(migrateUserData(root, current, busy as never)).toEqual({ kind: "kept-legacy", path: legacy, reason: "EBUSY" });
    expect(readFileSync(join(legacy, "Cookies"), "utf8")).toBe("c");
  });
});
