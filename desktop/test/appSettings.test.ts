// 앱 설정의 계산이다 (plan 26 U2): 검증, 저장, 알림과 잠자기 방지의 판정, 글자 크기 토큰, 설정 찾기.
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

import { loadAppSettings, saveAppSettings } from "../src/main/appSettings";
import { fontTokens, matchesSetting } from "../src/renderer/session/settingsView";
import { DEFAULT_SETTINGS, shouldKeepAwake, shouldNotify, validateSettings } from "../src/shared/settings";

describe("app settings", () => {
  it("replaces unknown values with the defaults and clamps font sizes", () => {
    expect(validateSettings(null)).toEqual(DEFAULT_SETTINGS);
    expect(validateSettings({ fonts: { ui: 99, reading: 3, code: "big", terminal: 14.4 }, notifications: { mode: "loud", sound: "yes" }, keepAwake: 1, extra: true })).toEqual({
      fonts: { ui: 24, reading: 10, code: 13, terminal: 14 },
      notifications: { mode: "background", sound: true },
      keepAwake: "off",
    });
    expect(validateSettings({ notifications: { mode: "always", sound: false }, keepAwake: "working" })).toMatchObject({ notifications: { mode: "always", sound: false }, keepAwake: "working" });
  });

  it("survives a missing or broken file and reads back what it saved", () => {
    const directory = mkdtempSync(join(tmpdir(), "akra-app-settings-"));
    try {
      const path = join(directory, "app-settings.json");
      expect(loadAppSettings(path)).toEqual(DEFAULT_SETTINGS);
      writeFileSync(path, "{broken");
      expect(loadAppSettings(path)).toEqual(DEFAULT_SETTINGS);
      const changed = { ...DEFAULT_SETTINGS, keepAwake: "on" as const, fonts: { ...DEFAULT_SETTINGS.fonts, reading: 17 } };
      saveAppSettings(path, changed);
      expect(loadAppSettings(path)).toEqual(changed);
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  });

  it("decides when to notify and when to keep the computer awake", () => {
    expect([shouldNotify("off", false), shouldNotify("background", true), shouldNotify("background", false), shouldNotify("always", true)]).toEqual([false, false, true, true]);
    expect([shouldKeepAwake("off", true), shouldKeepAwake("working", false), shouldKeepAwake("working", true), shouldKeepAwake("on", false)]).toEqual([false, false, true, true]);
  });

  it("moves the UI tokens together and sets one token for reading and one for code", () => {
    expect(fontTokens(DEFAULT_SETTINGS.fonts)).toEqual({ "--text-xs": "12px", "--text-s": "12px", "--text-m": "13px", "--text-l": "14px", "--text-reading": "15px", "--text-code": "13px" });
    expect(fontTokens({ ui: 15, reading: 18, code: 12, terminal: 13 })).toMatchObject({ "--text-xs": "14px", "--text-m": "15px", "--text-reading": "18px", "--text-code": "12px" });
  });

  it("finds a setting by its name, hint or section", () => {
    const row = { section: "전원", label: "잠자기 방지", hint: "에이전트가 일하는 동안" };
    expect([matchesSetting(row, ""), matchesSetting(row, "잠자기"), matchesSetting(row, "전원"), matchesSetting(row, "에이전트"), matchesSetting(row, "글자")]).toEqual([true, true, true, true, false]);
  });
});
