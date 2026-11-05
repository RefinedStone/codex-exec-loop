import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

import { DEFAULT_SETTINGS, loadSettings, normalizeUrl, profilePartition, saveSettings, validateSettings } from "../src/main/browserSettings";
import { profileIdFromName } from "../src/renderer/browser/profileId";

describe("browser address", () => {
  it("opens only http(s) and about:blank", () => {
    expect(normalizeUrl("qa.example.com/login")).toBe("https://qa.example.com/login");
    expect(normalizeUrl("localhost:5173")).toBe("http://localhost:5173/");
    expect(normalizeUrl("127.0.0.1:8080/a")).toBe("http://127.0.0.1:8080/a");
    expect(normalizeUrl("  https://example.com  ")).toBe("https://example.com/");
    expect(normalizeUrl("about:blank")).toBe("about:blank");
    for (const blocked of ["file:///C:/Windows/win.ini", "javascript:alert(1)", "chrome://settings", "two words", "nodot", ""]) {
      expect(normalizeUrl(blocked)).toBeNull();
    }
  });
});

describe("browser profiles and presets", () => {
  it("keeps each profile in its own persistent partition", () => {
    expect(profilePartition("qa-admin")).toBe("persist:akra-browser-qa-admin");
    expect(() => profilePartition("../escape")).toThrow();
    expect(() => profilePartition("Upper")).toThrow();
  });

  it("makes profile ids from names without collisions", () => {
    expect(profileIdFromName("QA Admin", [])).toBe("qa-admin");
    expect(profileIdFromName("QA Admin", ["qa-admin"])).toBe("qa-admin-2");
    expect(profileIdFromName("학습자", [])).toBe("profile");
  });

  it("drops invalid entries and presets of unknown profiles", () => {
    const settings = validateSettings({
      profiles: [{ id: "qa", name: " QA " }, { id: "qa", name: "dup" }, { id: "Bad Id", name: "x" }, { id: "empty", name: "" }],
      presets: [
        { name: "login", url: "qa.example.com", profileId: "qa" },
        { name: "unknown profile", url: "https://a.example", profileId: "nope" },
        { name: "bad url", url: "file:///etc/passwd", profileId: "qa" },
      ],
    });
    expect(settings).toEqual({
      profiles: [{ id: "qa", name: "QA" }],
      presets: [{ name: "login", url: "https://qa.example.com/", profileId: "qa" }],
    });
    expect(validateSettings(null)).toEqual(DEFAULT_SETTINGS);
  });

  it("saves through a temporary file and loads defaults from a broken file", async () => {
    const folder = mkdtempSync(join(tmpdir(), "akra-browser-"));
    try {
      const path = join(folder, "browser-settings.json");
      expect(await loadSettings(path)).toEqual(DEFAULT_SETTINGS);
      const saved = await saveSettings(path, { profiles: [{ id: "qa", name: "QA" }], presets: [] });
      expect(JSON.parse(readFileSync(path, "utf8"))).toEqual(saved);
      expect(await loadSettings(path)).toEqual(saved);
      writeFileSync(path, "{ not json");
      expect(await loadSettings(path)).toEqual(DEFAULT_SETTINGS);
    } finally {
      rmSync(folder, { recursive: true, force: true });
    }
  });
});
