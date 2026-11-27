#!/usr/bin/env node
// Execute only the verified, unpacked release launcher on its matching host.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync, lstatSync, mkdirSync, mkdtempSync, rmSync } from "node:fs";
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

function launcherFixture(bundle, root, platform = process.platform, run = spawnSync, inherited = process.env) {
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
    return result.stdout;
  }
  return { invoke, profile: env.AKRA_HOME };
}

function assertProfileFile(profile) {
  const file = path.join(profile, "config.toml");
  assert.ok(existsSync(file) && lstatSync(file).isFile(), "fixture configuration file was not persisted");
}

function readMode(fixture, value) {
  fixture.invoke(["config", "get", "--global", "parallel.evidence_mode"], "parallel.evidence_mode = " + value + " (global)");
}

function writeMode(fixture, value) {
  fixture.invoke(["config", "set", "--global", "parallel.evidence_mode", value], "parallel.evidence_mode = " + value);
  assertProfileFile(fixture.profile);
}

function verifyLauncher(fixture) {
  fixture.invoke(["--help"], "Usage: akra");
  const config = fixture.invoke(["config", "path", "--global"], path.join(fixture.profile, "config.toml"));
  assert.equal(config.trim(), path.join(fixture.profile, "config.toml"));
}

export function smokeLauncher(bundle, root, platform = process.platform, run = spawnSync, inherited = process.env) {
  const fixture = launcherFixture(bundle, root, platform, run, inherited);
  verifyLauncher(fixture);
  readMode(fixture, "unset");
  // Observe is the builtin default; an enforce write tests a non-default value.
  writeMode(fixture, "enforce");
  readMode(fixture, "enforce");
}

export function smokeUpgrade(previousBundle, candidateBundle, root, platform = process.platform, run = spawnSync, inherited = process.env) {
  assert.notEqual(path.resolve(previousBundle), path.resolve(candidateBundle), "upgrade needs separate installed bundles");
  const previous = launcherFixture(previousBundle, root, platform, run, inherited);
  const candidate = launcherFixture(candidateBundle, root, platform, run, inherited);
  verifyLauncher(previous);
  readMode(previous, "unset");
  writeMode(previous, "enforce");
  readMode(previous, "enforce");
  verifyLauncher(candidate);
  // The new binary must read the old value before it writes anything.
  readMode(candidate, "enforce");
  writeMode(candidate, "observe");
  readMode(candidate, "observe");
}

export async function smokeArchive({ directory, version, target, profile = "release", previous }) {
  const host = resolvePlatformConfig();
  if (!host || host.targetTriple !== target) throw new Error("release smoke requires the matching native OS/architecture");
  if (previous && previous.version.replace(/^v/, "") === version.replace(/^v/, "")) {
    throw new Error("upgrade smoke requires a different previous release version");
  }
  const verified = await findAndVerifyArchive(path.resolve(directory), version, target, profile);
  const archives = new Map([[target, verified]]);
  let root;
  try {
    root = mkdtempSync(path.join(tmpdir(), "akra release smoke "));
    const bundle = extractVerifiedArchive(verified, path.join(root, "installed archive"));
    if (previous) {
      const old = await findAndVerifyArchive(path.resolve(previous.directory), previous.version, target, "release");
      archives.set("previous", old);
      const oldBundle = extractVerifiedArchive(old, path.join(root, "previous archive"));
      smokeUpgrade(oldBundle, bundle, root);
      console.log("verified previous release profile upgrade: " + previous.version + " -> " + version + " " + target);
    } else {
      smokeLauncher(bundle, root);
      console.log("verified native archive launcher and isolated profile: " + target);
    }
  } finally {
    cleanupVerifiedReleaseAssets(archives);
    if (root) rmSync(root, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const [directory, version, target, profile = "release", ...extra] = process.argv.slice(2);
  const previous = extra.length === 3 && extra[0] === "--previous" && extra[1] && extra[2]
    ? { directory: extra[1], version: extra[2] } : undefined;
  if (!directory || !version || !target || !["release", "debug"].includes(profile) || (extra.length && !previous)) {
    console.error("Usage: node npm/scripts/smoke-native-release.mjs <assets-dir> <version> <target> [release|debug] [--previous <assets-dir> <version>]");
    process.exitCode = 1;
  } else {
    await smokeArchive({ directory, version, target, profile, previous }).catch(error => {
      console.error(error.message); process.exitCode = 1;
    });
  }
}
