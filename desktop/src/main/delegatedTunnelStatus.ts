import { execFile } from "node:child_process";
import { readFile } from "node:fs/promises";
import { homedir } from "node:os";
import { isAbsolute, join, relative, resolve } from "node:path";
import { promisify } from "node:util";
import { TunnelRuntimeService } from "akra-tunnel-core/tunnel";

export type TunnelInstall = "ready" | "missing" | "invalid";
export type TunnelConnection = "connected" | "offline" | "unknown";

export async function delegatedTunnelStatus(home: string): Promise<{ install: TunnelInstall; connection: TunnelConnection }> {
  const runtime = join(home, "runtime", "cokacremote");
  const integrity = new TunnelRuntimeService({
    manifestPath: join(runtime, ".akra-runtime.json"),
    schemaVersion: 23,
    artifacts: { entrySha256: join(runtime, "dist", "src", "stdio-server.js") },
  });
  const [{ state }, connection] = await Promise.all([integrity.getStatus(), tunnelConnection(home)]);
  return { install: state, connection };
}

const execute = promisify(execFile);

async function clientJson(binary: string, args: string[]) {
  const { stdout } = await execute(binary, args, {
    windowsHide: true, timeout: 1_200, maxBuffer: 256_000, encoding: "utf8",
  });
  return JSON.parse(stdout);
}

/** tunnel-client keeps its health URL files under its own XDG-style state root, also on Windows. */
export function tunnelClientStateRoot(env = process.env, home = homedir()): string {
  return join(env.XDG_STATE_HOME?.trim() || join(home, ".local", "state"), "tunnel-client");
}

function isInside(root: string, path: string): boolean {
  const child = relative(root, resolve(path));
  return Boolean(child) && !child.startsWith("..") && !isAbsolute(child);
}

/** The launcher uses these two read-only tunnel-client commands for its status. */
export async function tunnelConnection(home: string, run = clientJson, stateRoot = tunnelClientStateRoot()): Promise<TunnelConnection> {
  try {
    const binary = join(home, "bin", process.platform === "win32" ? "tunnel-client.exe" : "tunnel-client");
    const config = JSON.parse(await readFile(join(home, "config.json"), "utf8"));
    if (typeof config?.alias !== "string" || !/^[a-z0-9][a-z0-9._-]{0,127}$/i.test(config.alias)) return "unknown";
    const status = await run(binary, ["runtimes", "status", config.alias, "--json"]);
    const running = status?.process_running ?? status?.running;
    if (running === false || status?.healthy === false || status?.ready === false) return "offline";
    if (running !== true || status?.healthy !== true || status?.ready !== true) return "unknown";
    const urlFile = status.health_url_file ?? status.process?.health?.url_file ?? status.process?.health_url_file;
    const pid = status.process?.pid;
    if (typeof urlFile !== "string" || !isAbsolute(urlFile) || !Number.isSafeInteger(pid) || pid <= 0) return "unknown";
    if (![home, stateRoot].some((root) => isInside(root, urlFile))) return "unknown";
    const health = await run(binary, ["health", "--url-file", urlFile, "--require-control-plane-poll", "--pid", String(pid), "--json"]);
    const poll = health?.control_plane_poll;
    if (poll?.ok !== true || typeof poll.value !== "number" || !Number.isFinite(poll.value)) return "unknown";
    const age = Date.now() - poll.value * 1_000;
    return age >= 0 && age <= 90_000 ? "connected" : "offline";
  } catch {
    return "unknown";
  }
}
