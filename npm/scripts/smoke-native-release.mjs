#!/usr/bin/env node
// Execute only the verified, unpacked release launcher on its matching host.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { resolvePlatformConfig } from "../lib/platform.js";
import { cleanupVerifiedReleaseAssets, extractVerifiedArchive, findAndVerifyArchive } from "./verify-native-release-assets.mjs";

export function isolatedEnvironment(root, platform = process.platform, inherited = process.env) {
  const home = path.join(root, "profile");
  const env = { HOME: home, AKRA_HOME: path.join(home, ".akra"), TEMP: root, TMP: root, TMPDIR: root };
  if (platform === "win32") {
    const windows = inherited.SystemRoot ?? inherited.SYSTEMROOT;
    if (!windows) throw new Error("SystemRoot is required for Windows release smoke");
    const system = path.win32.join(windows, "System32");
    Object.assign(env, { SystemRoot: windows, WINDIR: windows, ComSpec: path.win32.join(system, "cmd.exe"),
      USERPROFILE: home, APPDATA: path.join(home, "Roaming"), LOCALAPPDATA: path.join(home, "Local"),
      PATH: system, PATHEXT: ".COM;.EXE;.BAT;.CMD" });
  } else {
    env.PATH = "/usr/bin:/bin";
    env.LC_ALL = "C";
  }
  return env;
}

export function smokeLauncher(bundle, root, platform = process.platform, run = spawnSync, inherited = process.env) {
  const cwd = path.join(root, "outside checkout");
  const env = isolatedEnvironment(root, platform, inherited);
  mkdirSync(cwd, { recursive: true });
  mkdirSync(env.HOME, { recursive: true });
  const windows = platform === "win32";
  const launcher = path.join(bundle, windows ? "akra.cmd" : "akra");
  function invoke(args, expected) {
    // All arguments are fixed literals below. A variable expansion supplies the
    // quoted absolute launcher; no caller-controlled shell command is assembled.
    const command = windows ? env.ComSpec : launcher;
    const argv = windows ? ["/d", "/v:off", "/s", "/c", '""%AKRA_SMOKE_LAUNCHER%" ' + args.join(" ") + '"'] : args;
    const childEnv = windows ? { ...env, AKRA_SMOKE_LAUNCHER: launcher } : env;
    const result = run(command, argv, { cwd, env: childEnv, windowsHide: true,
      windowsVerbatimArguments: windows, encoding: "utf8", timeout: 30_000, maxBuffer: 1024 * 1024 });
    if (result.error) throw result.error;
    assert.equal(result.status, 0, "release launcher failed: " + args[0]);
    assert.ok(result.stdout.includes(expected), "unexpected release launcher output: " + args[0]);
  }
  invoke(["--help"], "Usage: akra");
  invoke(["config", "path", "--global"], env.AKRA_HOME);
  invoke(["config", "set", "--global", "parallel.evidence_mode", "observe"], "parallel.evidence_mode = observe");
  invoke(["config", "get", "--global", "parallel.evidence_mode"], "parallel.evidence_mode = observe");
  assert.ok(existsSync(env.AKRA_HOME), "fixture profile was not created");
}

export async function smokeArchive({ directory, version, target, profile = "release" }) {
  const host = resolvePlatformConfig();
  if (!host || host.targetTriple !== target) throw new Error("release smoke requires the matching native OS/architecture");
  const verified = await findAndVerifyArchive(path.resolve(directory), version, target, profile);
  const root = mkdtempSync(path.join(tmpdir(), "akra release smoke "));
  try {
    const bundle = extractVerifiedArchive(verified, path.join(root, "installed archive"));
    smokeLauncher(bundle, root);
    console.log("verified native archive launcher and isolated profile: " + target);
  } finally {
    cleanupVerifiedReleaseAssets(new Map([[target, verified]]));
    rmSync(root, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const [directory, version, target, profile = "release", ...extra] = process.argv.slice(2);
  if (!directory || !version || !target || extra.length) {
    console.error("Usage: node npm/scripts/smoke-native-release.mjs <assets-dir> <version> <target> [release|debug]");
    process.exitCode = 1;
  } else {
    await smokeArchive({ directory, version, target, profile }).catch(error => {
      console.error(error.message); process.exitCode = 1;
    });
  }
}
