import { existsSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

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
  it("uses AKRA_HOME when set", () => {
    expect(dotsCloudFlagPath({ AKRA_HOME: "/x/.akra" }, () => "/home/me")).toBe(join("/x/.akra", "dots", "cloud-enabled"));
  });

  it("falls back to ~/.akra", () => {
    expect(dotsCloudFlagPath({}, () => "/home/me")).toBe(join("/home/me", ".akra", "dots", "cloud-enabled"));
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
