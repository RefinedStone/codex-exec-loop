import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, test, vi } from "vitest";

import { tunnelClientStateRoot, tunnelConnection } from "../src/main/delegatedTunnelStatus";

test("tunnel connection requires current control-plane evidence", async () => {
  const home = await mkdtemp(join(tmpdir(), "akra-tunnel-status-"));
  try {
    await writeFile(join(home, "config.json"), JSON.stringify({ alias: "fixture" }));
    const healthFile = join(home, "tunnel", "health.url");
    const status = { process_running: true, healthy: true, ready: true, process: { pid: 42, health_url_file: healthFile } };
    const run = vi.fn()
      .mockResolvedValueOnce({ process_running: false })
      .mockResolvedValueOnce(status).mockResolvedValueOnce({})
      .mockResolvedValueOnce(status).mockResolvedValueOnce({ control_plane_poll: { ok: true, value: Date.now() / 1_000 - 120 } })
      .mockResolvedValueOnce(status).mockResolvedValueOnce({ control_plane_poll: { ok: true, value: Date.now() / 1_000 - 1 } });
    for (const result of ["offline", "unknown", "offline", "connected"]) {
      expect(await tunnelConnection(home, run)).toBe(result);
    }
    expect(run.mock.calls.map(([, args]) => args[0])).toEqual(["runtimes", "runtimes", "health", "runtimes", "health", "runtimes", "health"]);
    expect(run).toHaveBeenLastCalledWith(expect.any(String), [
      "health", "--url-file", healthFile, "--require-control-plane-poll", "--pid", "42", "--json",
    ]);
  } finally {
    await rm(home, { recursive: true, force: true });
  }
});

test("tunnel connection accepts health files only under the home or the tunnel-client state root", async () => {
  const home = await mkdtemp(join(tmpdir(), "akra-tunnel-status-"));
  const stateRoot = await mkdtemp(join(tmpdir(), "tunnel-client-state-"));
  try {
    await writeFile(join(home, "config.json"), JSON.stringify({ alias: "fixture" }));
    const fresh = { control_plane_poll: { ok: true, value: Date.now() / 1_000 - 1 } };
    const statusFor = (file: string) => ({ process_running: true, healthy: true, ready: true, health_url_file: file, process: { pid: 42 } });
    const run = vi.fn()
      .mockResolvedValueOnce(statusFor(join(stateRoot, "health", "fixture.url"))).mockResolvedValueOnce(fresh)
      .mockResolvedValueOnce(statusFor(join(stateRoot, "..", "elsewhere.url")));
    expect(await tunnelConnection(home, run, stateRoot)).toBe("connected");
    expect(await tunnelConnection(home, run, stateRoot)).toBe("unknown");
    expect(run).toHaveBeenCalledTimes(3);
  } finally {
    await rm(home, { recursive: true, force: true });
    await rm(stateRoot, { recursive: true, force: true });
  }
});

test("tunnel-client state root follows XDG_STATE_HOME", () => {
  expect(tunnelClientStateRoot({ XDG_STATE_HOME: "/state" }, "/home/user")).toBe(join("/state", "tunnel-client"));
  expect(tunnelClientStateRoot({}, "/home/user")).toBe(join("/home/user", ".local", "state", "tunnel-client"));
});
