import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  libraryTestExecutable,
  parseContractArguments,
  runNativeContracts,
  testRuntimeEnvironment,
} from "./run_native_rust_contracts.mjs";

const artifact = (executable, changes = {}) => ({
  reason: "compiler-artifact",
  target: { name: "codex_exec_loop_native", kind: ["lib"] },
  profile: { test: true },
  executable,
  ...changes,
});
const json = (value) => JSON.stringify(value) + "\n";

test("prepares once and executes every filter with identical build options and environment", () => {
  const calls = [];
  const options = ["--all-features", "--target", "native-host", "--profile", "debugging",
    "--target-dir", "target with spaces", "--timings"];
  const env = { CARGO_TARGET_DIR: "external target", AKRA_FIXTURE: "1", PATH: "original-bin" };
  runNativeContracts([...options, "--", "first::", "second::"], {
    env,
    write() {},
    run(command, args, settings) {
      calls.push({ command, args, settings });
      if (command === "cargo") return { status: 0, stdout: json(artifact("/target/debug/deps/akra-tests")) };
      if (args[0] === "--list") return { status: 0, stdout: "first::passes: test\nsecond::passes: test\n" };
      return { status: 0, stdout: "test result: ok. 1 passed; 0 failed; 0 ignored;\n" };
    },
  });
  assert.equal(calls.length, 4);
  assert.deepEqual(calls[0].args, ["test", "--locked", "--lib", "--no-run", "--message-format=json", ...options]);
  assert.deepEqual(calls[0].settings.env, env);
  assert.deepEqual(calls.slice(2).map((call) => call.args), [["first::", "--nocapture"], ["second::", "--nocapture"]]);
  assert.ok(calls.slice(1).every((call) => call.settings.env.CARGO_TARGET_DIR === env.CARGO_TARGET_DIR));
  assert.ok(calls.slice(1).every((call) => call.settings.env.AKRA_FIXTURE === "1"));
});

test("rejects missing, empty, duplicate filters and option injection before compiling", () => {
  for (const args of [
    [], ["--"], ["--", ""], ["--", "--ignored"], ["--", "one", "one"],
    ["--release", "--", "one"], ["--target", "--", "one"],
    ["--target", "--all-features", "--", "one"],
    ["--all-features", "--all-features", "--", "one"],
    ["--all-features", "--features", "x", "--", "one"],
  ]) assert.throws(() => parseContractArguments(args));
  assert.deepEqual(parseContractArguments(["--no-default-features", "--features", "x,y", "--", "one"]),
    { options: ["--no-default-features", "--features", "x,y"], filters: ["one"] });
});

test("only accepts the unique Akra lib test artifact, not dependencies or production binaries", () => {
  const messages = [
    artifact("/other", { target: { name: "dependency", kind: ["lib"] } }),
    artifact("/bin", { target: { name: "codex_exec_loop_native", kind: ["bin"] } }),
    artifact("/ordinary", { profile: { test: false } }),
    artifact("/actual"),
  ];
  assert.equal(libraryTestExecutable(messages.map(json).join("")), "/actual");
  assert.throws(() => libraryTestExecutable(json(messages[0])), /reported 0/);
  assert.throws(() => libraryTestExecutable(json(artifact("/one")) + json(artifact("/two"))), /reported 2/);
  assert.throws(() => libraryTestExecutable("not cargo json\n"));
});

test("missing filters, all-ignored results, failing builds and failed or signalled tests fail closed", () => {
  const check = ({ buildStatus = 0, listing = "one::test: test\n", testResult = {} } = {}) => {
    const calls = [];
    const operation = () => runNativeContracts(["--", "one::"], {
      write() {},
      run(command, args) {
        calls.push(args);
        if (command === "cargo") return { status: buildStatus, stdout: json(artifact("/test")) };
        if (args[0] === "--list") return { status: 0, stdout: listing };
        return { status: 0, stdout: "test result: ok. 1 passed; 0 failed;\n", ...testResult };
      },
    });
    return { operation, calls };
  };
  for (const scenario of [
    { buildStatus: 1 }, { listing: "other::test: test\n" },
    { testResult: { stdout: "test result: ok. 0 passed; 0 failed; 1 ignored;\n" } },
    { testResult: { status: 1 } },
    { testResult: { status: null, signal: "SIGTERM" } },
    { testResult: { error: new Error("spawn failed") } },
  ]) {
    const { operation, calls } = check(scenario);
    assert.throws(operation);
    if (scenario.buildStatus) assert.equal(calls.length, 1);
    if (scenario.listing) assert.equal(calls.length, 2);
  }
});

test("runtime loader paths preserve caller directories and macOS fallback defaults", () => {
  const windows = testRuntimeEnvironment("C:/target/debug/deps/test.exe", { Path: "C:/tools", EXTRA: "yes" }, "win32");
  assert.ok(windows.Path.endsWith(";C:/tools"));
  assert.equal(windows.EXTRA, "yes");
  assert.ok(!("PATH" in windows));
  const mac = testRuntimeEnvironment("/target/debug/deps/test", { HOME: "/Users/test" }, "darwin");
  assert.ok(mac.DYLD_FALLBACK_LIBRARY_PATH.endsWith("/Users/test/lib:/usr/local/lib:/usr/lib"));
});

test("Windows portable entrypoint retains the complete 83-filter inventory", () => {
  const shell = readFileSync(new URL("./check_windows_portable.sh", import.meta.url), "utf8");
  const filters = [...shell.matchAll(/^\s+'([^']+)'/gm)].map((match) => match[1]);
  assert.equal(filters.length, 83);
  assert.equal(new Set(filters).size, 83);
  assert.ok(filters.includes("process_liveness::tests::"));
  assert.ok(filters.includes("application::service::evidence_vault_unlock::"));
  assert.ok(filters.includes("host_owned_worker_commit_"));
  assert.match(shell, /cargo check --locked --all-features/);
  assert.match(shell, /run_native_rust_contracts\.mjs/);
});
