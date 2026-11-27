import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { smokeLegacyUpgrade } from "../scripts/smoke-native-release.mjs";
import { findAndVerifyLegacyArchive } from "../scripts/native-release/legacy.mjs";

function planningLaunchers(candidateTasks) {
  let tasks = [];
  const previous = {
    invoke(args, _expected, input) {
      if (args[0] === "--help") return "Usage: akra";
      if (args[0] === "doctor") return "health: planning workspace is healthy";
      assert.equal(args[0], "planning-tool", "the published binary has no config command");
      const request = JSON.parse(input);
      switch (request.op) {
        case "list_tasks":
          return JSON.stringify({ ok: true, tasks });
        case "create_task":
          tasks = [{ id: "persisted-task", title: request.title, description: request.description }];
          return JSON.stringify({ ok: true, committed_task_ids: ["persisted-task"] });
        default:
          throw new Error("Unexpected planning operation");
      }
    },
  };
  const candidate = {
    invoke(_args, _expected, input) {
      assert.equal(JSON.parse(input).op, "list_tasks", "candidate must not write before preservation is proved");
      return JSON.stringify({ ok: true, tasks: candidateTasks(tasks) });
    },
  };
  return { previous, candidate };
}

test("legacy smoke preserves a task created through the old public planning contract", () => {
  const { previous, candidate } = planningLaunchers(tasks => tasks);
  assert.doesNotThrow(() => smokeLegacyUpgrade(previous, candidate));
});

test("legacy smoke rejects lost, duplicated, replaced, or altered persisted tasks", async t => {
  for (const [name, change] of [
    ["lost", () => []],
    ["duplicated", tasks => [...tasks, ...tasks]],
    ["replaced", tasks => tasks.map(task => ({ ...task, id: "replacement" }))],
    ["title changed", tasks => tasks.map(task => ({ ...task, title: "changed" }))],
    ["description changed", tasks => tasks.map(task => ({ ...task, description: "changed" }))],
  ]) {
    await t.test(name, () => {
      const { previous, candidate } = planningLaunchers(change);
      assert.throws(() => smokeLegacyUpgrade(previous, candidate), assert.AssertionError);
    });
  }
});

test("legacy smoke rejects a candidate's explicit authority error", () => {
  const { previous } = planningLaunchers(tasks => tasks);
  const candidate = { invoke: () => JSON.stringify({ ok: false, tasks: [] }) };
  assert.throws(() => smokeLegacyUpgrade(previous, candidate), /candidate cannot read the legacy authority/);
});

test("historical archive acceptance rejects a rewritten checksum even when it matches the file", async () => {
  const directory = mkdtempSync(path.join(tmpdir(), "akra-legacy-checksum-"));
  const name = "codex-exec-loop-native-1.3.5-x86_64-pc-windows-msvc.tar.gz";
  const body = Buffer.from("different archive");
  try {
    writeFileSync(path.join(directory, name), body);
    writeFileSync(path.join(directory, name + ".sha256"),
      createHash("sha256").update(body).digest("hex") + "  " + name + "\n");
    await assert.rejects(
      findAndVerifyLegacyArchive(directory, "1.3.5", "x86_64-pc-windows-msvc"),
      /does not match the pinned published artifact/,
    );
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});

test("historical smoke does not claim unverified release or platform formats", () => {
  assert.throws(() => findAndVerifyLegacyArchive(".", "1.3.4", "x86_64-pc-windows-msvc"),
    /only the published Windows x64 v1.3.5/);
  assert.throws(() => findAndVerifyLegacyArchive(".", "1.3.5", "aarch64-apple-darwin"),
    /only the published Windows x64 v1.3.5/);
});
