import { existsSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, win32 } from "node:path";

import { afterEach, describe, expect, it } from "vitest";

import { dotsCloudFlagPath, isDotsCloudEnabled, setDotsCloudEnabled } from "../src/main/dotsSettings";

const dirs: string[] = [];
afterEach(() => {
  for (const dir of dirs.splice(0)) rmSync(dir, { recursive: true, force: true });
});

function tempHome(): string {
  const dir = mkdtempSync(join(tmpdir(), "akra-dots-"));
  dirs.push(dir);
  return dir;
}

describe("dotsCloudFlagPath", () => {
  it("uses explicit or default profile on Unix", () => {
    expect(dotsCloudFlagPath({ AKRA_HOME: "/x/.akra" }, "linux")).toBe("/x/.akra/dots/cloud-enabled");
    expect(dotsCloudFlagPath({ HOME: "/home/me" }, "linux")).toBe("/home/me/.akra/dots/cloud-enabled");
  });

  it("uses USERPROFILE rather than HOME on Windows", () => {
    expect(dotsCloudFlagPath({ USERPROFILE: "C:/Users/me", HOME: "D:/other" }, "win32"))
      .toBe(win32.join("C:/Users/me", ".akra", "dots", "cloud-enabled"));
    expect(dotsCloudFlagPath({ AKRA_HOME: "E:/profile", USERPROFILE: "C:/Users/me" }, "win32"))
      .toBe(win32.join("E:/profile", "dots", "cloud-enabled"));
  });

  it.each(["", "relative", "/x/../other"])("rejects invalid Unix override %s", (root) => {
    expect(() => dotsCloudFlagPath({ AKRA_HOME: root, HOME: "/home/me" }, "linux")).toThrow("AKRA_HOME");
  });

  it("rejects missing USERPROFILE and invalid Windows overrides without falling back", () => {
    expect(() => dotsCloudFlagPath({ HOME: "D:/other" }, "win32")).toThrow("USERPROFILE");
    for (const root of ["", "C:relative", "C:/x/../other"]) {
      expect(() => dotsCloudFlagPath({ AKRA_HOME: root, USERPROFILE: "C:/Users/me" }, "win32"))
        .toThrow("AKRA_HOME");
    }
  });
});

describe("set and read the flag", () => {
  it("creates the file on enable and removes it on disable", () => {
    const path = join(tempHome(), "dots", "cloud-enabled");
    expect(isDotsCloudEnabled(path)).toBe(false);

    setDotsCloudEnabled(true, path);
    expect(existsSync(path)).toBe(true);
    expect(isDotsCloudEnabled(path)).toBe(true);

    // 두 번 켜도 오류가 없다.
    setDotsCloudEnabled(true, path);
    expect(isDotsCloudEnabled(path)).toBe(true);

    setDotsCloudEnabled(false, path);
    expect(isDotsCloudEnabled(path)).toBe(false);
    // 없는 파일을 꺼도 오류가 없다.
    setDotsCloudEnabled(false, path);
    expect(isDotsCloudEnabled(path)).toBe(false);
  });
});
