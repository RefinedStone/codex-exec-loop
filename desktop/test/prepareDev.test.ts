import { join } from "node:path";

import { describe, expect, it } from "vitest";

// @ts-expect-error -- plain ESM script without type declarations
import { electronBinaryReady, gatewayBuildSkipReason } from "../scripts/prepare-dev.mjs";

describe("npm run dev preparation", () => {
  const root = join("/repo", "desktop", "node_modules", "electron");
  const pathFile = join(root, "path.txt");
  const binary = join(root, "dist", "Electron.app/Contents/MacOS/Electron");

  it("needs the Electron download when path.txt or the binary is missing", () => {
    expect(electronBinaryReady(root, () => false)).toBe(false);
    // check_node_surfaces.sh installs with ELECTRON_SKIP_BINARY_DOWNLOAD=1: no path.txt and no dist/.
    expect(electronBinaryReady(root, (path: string) => path === pathFile, () => "Electron.app/Contents/MacOS/Electron\n")).toBe(false);
  });

  it("accepts an Electron binary that install.js already fetched", () => {
    const files = new Set([pathFile, binary]);
    expect(electronBinaryReady(root, (path: string) => files.has(path), () => "Electron.app/Contents/MacOS/Electron")).toBe(true);
  });

  it("builds akra-gateway unless the app is pointed at another gateway", () => {
    expect(gatewayBuildSkipReason({})).toBeNull();
    expect(gatewayBuildSkipReason({ AKRA_GATEWAY_ENDPOINT: "/tmp/fake.sock" })).toMatch(/AKRA_GATEWAY_ENDPOINT/);
    expect(gatewayBuildSkipReason({ AKRA_GATEWAY_BIN: "/opt/gw" })).toMatch(/AKRA_GATEWAY_BIN/);
    expect(gatewayBuildSkipReason({ AKRA_DESKTOP_BLUEPRINT: "1" })).toMatch(/AKRA_DESKTOP_BLUEPRINT/);
  });
});
