import { describe, expect, it } from "vitest";
import { readCatalog, reconcileCatalog } from "../src/renderer/extensions/extensionState";
import { isPanelId, panels } from "../src/renderer/extensions/panelDefinitions";
describe("extension catalog authority", () => {
  it("accepts the previous repository catalog without inventing a local-user scope", () => {
    const [entry] = readCatalog([{ id: "akra.tech-blog", version: 1, scope: "repository", enabled: true }]);
    expect(entry).toMatchObject({ compatible: true, lifecycle: "enabled", scope: "repository", revision: 0 });
  });
  it("retains a newer confirmed revision after an older response but accepts a new host generation", () => {
    const make = (revision: number, enabled: boolean, generation = "host-1") => readCatalog([{ id: "example", version: 1, scope: "local_user", enabled, generation, revision }]);
    expect(reconcileCatalog(make(8, false), make(7, true))[0]?.enabled).toBe(false);
    expect(reconcileCatalog(make(8, false), make(1, true, "host-2"))[0]?.enabled).toBe(true);
  });
  it("allows catalog entries with no UI and rejects malformed catalogs", () => {
    expect(readCatalog([{ id: "headless", version: 1, enabled: false }])).toHaveLength(1);
    expect(() => readCatalog({})).toThrow();
    expect(() => readCatalog([{ id: "bad" }])).toThrow();
    for (const invalid of [{ scope: "remote" }, { revision: -1 }, { revision: Number.MAX_SAFE_INTEGER + 1 }, { lifecycle: "unknown" }, { generation: {} }]) {
      expect(() => readCatalog([{ id: "bad", version: 1, enabled: true, ...invalid }])).toThrow();
    }
    expect(isPanelId("remote-script.js")).toBe(false);
    expect(isPanelId("skills")).toBe(true);
    expect(new Set(panels.map(panel => panel.id)).size).toBe(panels.length);
  });
});
