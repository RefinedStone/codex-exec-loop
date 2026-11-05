import { join, resolve } from "node:path";

import { describe, expect, it } from "vitest";

import {
  parseWhoamiSid,
  resolveGatewayEndpoint,
  resolveGatewayExecutable,
  windowsWhoamiPath,
} from "../src/main/gatewayEndpoint";
import { nextReconnectDelay } from "../src/main/gatewayConnection";
import { checkCodexVersion } from "../src/renderer/session/codexVersion";

describe("gateway endpoint", () => {
  it("uses AKRA_HOME/run/gateway.sock on unix", () => {
    expect(resolveGatewayEndpoint("linux", { AKRA_HOME: "/srv/akra" })).toBe(join("/srv/akra", "run", "gateway.sock"));
  });

  it("uses the per-user named pipe on Windows", () => {
    expect(resolveGatewayEndpoint("win32", {}, () => "S-1-5-21-1-2-3-1000")).toBe(
      "\\\\.\\pipe\\akra-gateway-S-1-5-21-1-2-3-1000",
    );
  });

  it("uses the system whoami.exe instead of whatever is first on PATH", () => {
    expect(windowsWhoamiPath({ SystemRoot: "C:\\Windows" })).toBe(join("C:\\Windows", "System32", "whoami.exe"));
  });

  it("parses the SID from whoami csv output", () => {
    expect(parseWhoamiSid('"desktop\\user","S-1-5-21-1-2-3-1000"\r\n')).toBe("S-1-5-21-1-2-3-1000");
    expect(() => parseWhoamiSid("garbage")).toThrow();
  });

  it("prefers AKRA_GATEWAY_BIN, then release, then debug builds", () => {
    const root = resolve("/repo");
    expect(resolveGatewayExecutable(root, "linux", { AKRA_GATEWAY_BIN: "/opt/gw" })).toBe("/opt/gw");
    const debugOnly = (path: string) => path.endsWith(join("debug", "akra-gateway"));
    expect(resolveGatewayExecutable(root, "linux", {}, debugOnly)).toBe(resolve(root, "target", "debug", "akra-gateway"));
    expect(resolveGatewayExecutable(root, "win32", {}, () => false)).toBe("akra-gateway.exe");
  });
});

describe("reconnect backoff", () => {
  it("doubles up to 30 seconds", () => {
    const delays = [1_000];
    for (let index = 0; index < 6; index += 1) {
      delays.push(nextReconnectDelay(delays.at(-1) ?? 0));
    }
    expect(delays).toEqual([1_000, 2_000, 4_000, 8_000, 16_000, 30_000, 30_000]);
  });
});

describe("codex version check", () => {
  it("matches equal versions and flags different ones", () => {
    expect(checkCodexVersion("codex-cli 0.159.0", "codex-cli 0.159.0").matches).toBe(true);
    expect(checkCodexVersion("codex-cli 0.159.0", "codex-cli 0.166.0-alpha.6")).toMatchObject({
      matches: false,
      gateway: "0.166.0-alpha.6",
    });
    expect(checkCodexVersion("codex-cli 0.159.0", null).matches).toBe(false);
  });
});
