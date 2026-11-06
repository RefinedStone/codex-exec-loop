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

  it("lets AKRA_GATEWAY_ENDPOINT point the app at another gateway", () => {
    expect(resolveGatewayEndpoint("win32", { AKRA_GATEWAY_ENDPOINT: "\\\\.\\pipe\\fake" }, () => "unused")).toBe(
      "\\\\.\\pipe\\fake",
    );
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

  it("prefers AKRA_GATEWAY_BIN, then the newer of the release and debug builds", () => {
    const root = resolve("/repo");
    const release = resolve(root, "target", "release", "akra-gateway");
    const debug = resolve(root, "target", "debug", "akra-gateway");
    const builtAt = (times: Record<string, number>) => (path: string) => times[path] ?? null;
    expect(resolveGatewayExecutable(root, "linux", { AKRA_GATEWAY_BIN: "/opt/gw" })).toBe("/opt/gw");
    expect(resolveGatewayExecutable(root, "linux", {}, builtAt({ [debug]: 1 }))).toBe(debug);
    expect(resolveGatewayExecutable(root, "linux", {}, builtAt({ [release]: 1, [debug]: 2 }))).toBe(debug);
    expect(resolveGatewayExecutable(root, "linux", {}, builtAt({ [release]: 2, [debug]: 1 }))).toBe(release);
    expect(resolveGatewayExecutable(root, "win32", {}, () => null)).toBe("akra-gateway.exe");
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
