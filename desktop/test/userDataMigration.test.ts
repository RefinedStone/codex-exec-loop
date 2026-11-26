import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";

import { loadAppSettings } from "../src/main/appSettings";
import { CREDENTIAL_APP_NAME, keepCredentialIdentity, migrateUserData, prepareUserData } from "../src/main/userDataMigration";

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
/** Electron 45 처럼 `getPath("userData")` 가 아직 없는 폴더를 만든다. 기본값은 appData/<제품명> 이다. */
function electronApp(root: string) {
  const paths: Record<string, string> = { appData: root };
  return {
    getPath: (name: string) => {
      const path = paths[name] ?? join(root, "Akra ADE");
      if (name === "userData") mkdirSync(path, { recursive: true });
      return path;
    },
    setPath: (name: string, path: string) => { paths[name] = path; },
  };
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

describe("prepareUserData", () => {
  it("moves the legacy profile before Electron creates the new userData folder", () => {
    const root = appData();
    const legacy = legacyWith(root, "app-settings.json", JSON.stringify({ keepAwake: "on" }));
    const current = join(root, "Akra ADE");

    const first = electronApp(root);
    expect(prepareUserData(first as never, undefined)).toEqual({ kind: "moved", from: legacy, to: current });
    expect(first.getPath("userData")).toBe(current);
    expect(loadAppSettings(join(first.getPath("userData"), "app-settings.json")).keepAwake).toBe("on");

    const second = electronApp(root);
    expect(prepareUserData(second as never, undefined)).toEqual({ kind: "none" });
    expect(loadAppSettings(join(second.getPath("userData"), "app-settings.json")).keepAwake).toBe("on");
  });

  it("keeps the explicit override and the legacy folder when the move fails", () => {
    const root = appData();
    const legacy = legacyWith(root, "Cookies", "c");
    const override = electronApp(root);
    expect(prepareUserData(override as never, join(root, "test"))).toEqual({ kind: "none" });
    expect(override.getPath("userData")).toBe(join(root, "test"));
    expect(existsSync(legacy)).toBe(true);

    const busy = { existsSync, readdirSync: () => [], rmdirSync: () => {}, renameSync: () => { throw new Error("EBUSY"); } };
    const kept = electronApp(root);
    expect(prepareUserData(kept as never, undefined, busy as never)).toMatchObject({ kind: "kept-legacy", path: legacy });
    expect(kept.getPath("userData")).toBe(legacy);
    expect(existsSync(join(root, "Akra ADE"))).toBe(false);
  });
});

describe("keepCredentialIdentity", () => {
  it("keeps the pre-rename app name until ready, then shows the product name", async () => {
    const names: string[] = [];
    let ready!: () => void;
    const app = {
      setName: (name: string) => { names.push(name); },
      whenReady: () => new Promise<void>((resolve) => { ready = resolve; }),
    };

    const restored = keepCredentialIdentity(app as never);
    // Electron picks the OS key store entry from this name before ready.
    expect(names).toEqual([CREDENTIAL_APP_NAME]);
    expect(CREDENTIAL_APP_NAME).toBe(JSON.parse(readFileSync(join(import.meta.dirname, "..", "package.json"), "utf8")).name);

    ready();
    await restored;
    expect(names).toEqual([CREDENTIAL_APP_NAME, "Akra ADE"]);
  });
});
