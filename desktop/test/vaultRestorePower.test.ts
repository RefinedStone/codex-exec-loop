import { describe, expect, it, vi } from "vitest";
import { vaultRestorePowerHold } from "../src/main/vaultRestorePower";
import { vaultRestoreView } from "../src/shared/vaultRestore";

describe("Vault Desktop OS lock admission", () => {
  for (const platform of ["win32", "darwin"] as const) {
    it.each(["active", "idle"] as const)(`allows only a known unlocked state on ${platform}: %s`, state => {
      expect(vaultRestorePowerHold(platform, () => state)).toBeNull();
    });
    it(`holds a locked screen on ${platform}`, () => {
      expect(vaultRestorePowerHold(platform, () => "locked")).toBe("os_locked");
    });
    it.each(["unknown", undefined, null, "unexpected"])(`fails closed on ${platform}: %s`, state => {
      expect(vaultRestorePowerHold(platform, () => state)).toBe("os_lock_unavailable");
    });
    it(`fails closed if the OS probe throws on ${platform}`, () => {
      expect(vaultRestorePowerHold(platform, () => { throw new Error("idle state unavailable"); })).toBe("os_lock_unavailable");
    });
  }
  it.each(["linux", "freebsd", "aix"] as const)("holds an unsupported lock-event platform: %s", platform => {
    const probe = vi.fn(() => "active");
    expect(vaultRestorePowerHold(platform, probe)).toBe("os_lock_unsupported");
    expect(probe).not.toHaveBeenCalled();
  });
  it("explains both unavailable and unsupported lock detection without offering reconnect", () => {
    const unknown = vaultRestoreView({ phase: "held", reason: "os_lock_unavailable" });
    expect(unknown.actions).toEqual(["unlock"]);
    expect(unknown.detail).toContain("상태를 확인할 수 없습니다");
    const unsupported = vaultRestoreView({ phase: "held", reason: "os_lock_unsupported" });
    expect(unsupported.actions).toEqual(["unlock"]);
    expect(unsupported.detail).toContain("잠금 감지가 지원되지 않아");
  });
});
