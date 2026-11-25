#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { dirname, posix, resolve, win32 } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const booleanOptions = new Set(["--all-features", "--no-default-features", "--timings"]);
const valueOptions = new Set(["--features", "--target", "--profile", "--target-dir"]);

export function parseContractArguments(argv) {
  const separator = argv.indexOf("--");
  if (separator < 0) throw new Error("Separate Cargo options and required test filters with --.");
  const options = argv.slice(0, separator);
  const filters = argv.slice(separator + 1);
  if (!filters.length || filters.some((filter) => !filter.trim() || filter.startsWith("-"))) {
    throw new Error("At least one nonempty test filter is required; libtest flags are not filters.");
  }
  if (new Set(filters).size !== filters.length) throw new Error("Duplicate test filters are not allowed.");
  const seen = new Set();
  for (let index = 0; index < options.length; index += 1) {
    const option = options[index];
    if (seen.has(option)) throw new Error(`Duplicate Cargo option: ${option}`);
    seen.add(option);
    if (booleanOptions.has(option)) continue;
    if (!valueOptions.has(option)) throw new Error(`Unsupported Cargo option: ${option}`);
    const value = options[++index];
    if (!value || value.startsWith("-")) throw new Error(`${option} requires a value.`);
  }
  if (seen.has("--all-features") && seen.has("--features")) {
    throw new Error("Use either --all-features or --features.");
  }
  return { options, filters };
}

export function libraryTestExecutable(output) {
  const executables = new Set();
  for (const line of output.split(/\r?\n/).filter(Boolean)) {
    const message = JSON.parse(line);
    if (message.reason === "compiler-artifact"
      && message.target?.name === "akra_ade"
      && message.target?.kind?.includes("lib")
      && message.profile?.test === true
      && typeof message.executable === "string" && message.executable) {
      executables.add(message.executable);
    }
  }
  if (executables.size !== 1) {
    throw new Error(`Expected one Akra library test executable; Cargo reported ${executables.size}.`);
  }
  return [...executables][0];
}

export function testRuntimeEnvironment(executable, env = process.env, platform = process.platform) {
  // Preserve caller overrides and the loader directories Cargo uses for target artifacts.
  const key = platform === "win32"
    ? Object.keys(env).find((name) => name.toUpperCase() === "PATH") ?? "PATH"
    : platform === "darwin" ? "DYLD_FALLBACK_LIBRARY_PATH" : "LD_LIBRARY_PATH";
  const paths = platform === "win32" ? win32 : posix;
  const directories = [paths.dirname(executable), paths.resolve(paths.dirname(executable), "..")];
  // Setting DYLD_FALLBACK_LIBRARY_PATH replaces dyld's defaults; keep those defaults too.
  const fallback = platform === "darwin" && !env[key]
    ? [env.HOME && paths.resolve(env.HOME, "lib"), "/usr/local/lib", "/usr/lib"].filter(Boolean)
    : [];
  return {
    ...env,
    [key]: [...directories, env[key], ...fallback].filter(Boolean).join(paths.delimiter),
  };
}

export function runNativeContracts(argv, {
  run = spawnSync,
  cwd = root,
  env = process.env,
  write = (text) => process.stdout.write(text),
} = {}) {
  const { options, filters } = parseContractArguments(argv);
  const invoke = (command, args, runtimeEnv = env) => {
    const result = run(command, args, {
      cwd,
      env: runtimeEnv,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "inherit"],
      maxBuffer: 64 * 1024 * 1024,
    });
    if (result.error) throw result.error;
    if (result.status !== 0) {
      if (result.stdout) write(result.stdout);
      throw new Error(`${command} failed (${result.signal ?? result.status}).`);
    }
    return result.stdout ?? "";
  };

  // --no-run prepares exactly the same feature/profile/target combination for every filter.
  const build = invoke("cargo", [
    "test", "--locked", "--lib", "--no-run", "--message-format=json", ...options,
  ]);
  const executable = libraryTestExecutable(build);
  const runtimeEnv = testRuntimeEnvironment(executable, env);
  const inventory = invoke(executable, ["--list", "--format", "terse"], runtimeEnv)
    .split(/\r?\n/).filter((line) => line.endsWith(": test"))
    .map((line) => line.slice(0, -6));
  for (const filter of filters) {
    if (!inventory.some((name) => name.includes(filter))) {
      throw new Error(`Native contract filter matched no tests: ${filter}`);
    }
  }

  write(`Prepared one library test executable for ${filters.length} native contract filters.\n`);
  for (const filter of filters) {
    write(`\n==> Native contract: ${filter}\n`);
    const output = invoke(executable, [filter, "--nocapture"], runtimeEnv);
    write(output);
    const summary = output.match(/^test result: ok\. (\d+) passed; 0 failed;/m);
    if (!summary || Number(summary[1]) === 0) {
      throw new Error(`Native contract must run at least one passing, non-ignored test: ${filter}`);
    }
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    runNativeContracts(process.argv.slice(2));
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
