import { mkdirSync, mkdtempSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

// @ts-expect-error -- plain ESM script without type declarations
import { restoreSpawnHelperMode } from "../scripts/fix-pty-permissions.mjs";

describe("node-pty spawn-helper permission", () => {
  // Windows 에는 실행 권한 비트가 없다.
  it.skipIf(process.platform === "win32")("adds the execute bit that the macOS prebuild is missing", () => {
    const root = mkdtempSync(join(tmpdir(), "akra-pty-"));
    try {
      for (const platform of ["darwin-arm64", "darwin-x64"]) {
        mkdirSync(join(root, "prebuilds", platform), { recursive: true });
        writeFileSync(join(root, "prebuilds", platform, "spawn-helper"), "binary", { mode: 0o644 });
      }
      expect(restoreSpawnHelperMode(root)).toHaveLength(2);
      expect(statSync(join(root, "prebuilds", "darwin-arm64", "spawn-helper")).mode & 0o111).not.toBe(0);
      // Running it again changes nothing.
      expect(restoreSpawnHelperMode(root)).toEqual([]);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  it("does nothing when node-pty has no prebuilds", () => {
    expect(restoreSpawnHelperMode(join(tmpdir(), "akra-no-node-pty"))).toEqual([]);
  });
});
