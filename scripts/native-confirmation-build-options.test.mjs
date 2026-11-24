import assert from "node:assert/strict";
import test from "node:test";
import { confirmationCargoCommands } from "./check_native_confirmation_process.mjs";

const exactTest = "adapter::inbound::gateway::tests::human_confirmation::native_process::native_helper_process_confirms_replays_and_revokes_on_eof";

test("default invocation preserves Linux default-feature and exact ignored native coverage", () => {
  const commands = confirmationCargoCommands([]);
  assert.deepEqual(commands.build, ["build", "--locked", "--bin", "akra-gateway", "--message-format=json"]);
  assert.deepEqual(commands.test, ["test", "--locked", "--lib", exactTest, "--", "--ignored", "--exact"]);
});

test("production binary and lib test receive identical feature, target, profile and directory options", () => {
  const options = ["--all-features", "--target", "aarch64-apple-darwin", "--profile", "debugging",
    "--target-dir", "/tmp/target with spaces", "--timings"];
  const commands = confirmationCargoCommands(options);
  assert.deepEqual(commands.build.slice(5), options);
  assert.deepEqual(commands.test.slice(3, -4), options);
  assert.deepEqual(commands.test.slice(-4), [exactTest, "--", "--ignored", "--exact"]);
  assert.equal(commands.test.filter((part) => part === exactTest).length, 1);
});

test("explicit feature subsets can retain no-default-features consistently", () => {
  const options = ["--no-default-features", "--features", "tokio-console"];
  const commands = confirmationCargoCommands(options);
  assert.deepEqual(commands.build.slice(5), options);
  assert.deepEqual(commands.test.slice(3, -4), options);
});

test("unrecognized, duplicate, incomplete and conflicting options fail before running Cargo", () => {
  for (const argv of [
    ["--"], ["--ignored"], ["--profile"], ["--target", "--all-features"],
    ["--all-features", "--all-features"], ["--features", "one", "--features", "two"],
    ["--all-features", "--features", "one"],
  ]) assert.throws(() => confirmationCargoCommands(argv));
});
