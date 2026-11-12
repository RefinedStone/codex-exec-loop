import { execFileSync, spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

import { KernelSchemaError, renderKernelProtocol, type KernelSpec } from "../scripts/kernel-schema.mjs";

const desktop = fileURLToPath(new URL("../", import.meta.url));
const spec: KernelSpec = JSON.parse(readFileSync(resolve(desktop, "../schema/akra-kernel-spec.json"), "utf8"));
const generated = renderKernelProtocol(spec);

function compile(source: string) {
  const directory = mkdtempSync(resolve(tmpdir(), "akra-kernel-types-"));
  try {
    writeFileSync(resolve(directory, "fixture.ts"), `${generated}\n${source}`);
    writeFileSync(resolve(directory, "tsconfig.json"), JSON.stringify({
      compilerOptions: { strict: true, noEmit: true, skipLibCheck: true, target: "ES2023", types: [] },
      files: ["fixture.ts"],
    }));
    const result = spawnSync(process.execPath, [resolve(desktop, "node_modules/typescript/bin/tsc"), "-p", directory, "--pretty", "false"], {
      cwd: directory, encoding: "utf8", timeout: 20_000,
    });
    if (result.error) throw result.error;
    return { status: result.status, output: result.stdout + result.stderr };
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}

describe("kernel protocol generation", () => {
  it("matches the checked-in output to the current Rust spec", () => {
    execFileSync(process.execPath, [
      "--experimental-strip-types", "--disable-warning=ExperimentalWarning",
      "scripts/generate-kernel-protocol.mjs", "--check",
    ], { cwd: desktop, encoding: "utf8" });
  });

  it("preserves nullable fields, flattened tags, queries and operation receipts", () => {
    const result = compile(`
      const command: CommandRequest = { command: { type: "reset_planning", target: "queue", confirm: true }, idempotency_key: "request-1" };
      const event: KernelEvent = { type: "planning", status: "ready_without_task", revision: null, next_task: null, active_tasks: [], proposed_tasks: [], skipped_tasks: [] };
      const query: KernelQuery = { type: "operation", op_id: 1 };
      const answer: KernelQueryAnswer = { type: "thread_activity", threads: [] };
      const response: CommandResponse = { processed: true, rejection: null, operation: null };
      const state: ParallelAgentState = "running";
    `);
    expect(result).toEqual({ status: 0, output: "" });
  });

  it.each([
    'const value: KernelCommand = { type: "submit_prompt", thread_id: "thread-1" };',
    'const value: KernelCommand = { type: "reset_planning", target: "unknown" };',
    'const value: ParallelAgentState = "not-a-state";',
    'const value: KernelEvent = { type: "planning", status: "ready_without_task" };',
    'const value: KernelQueryAnswer = { type: "resync" };',
    'const value: CommandRequest = { command: { type: "stop_all" }, idempotency_key: 123 };',
  ])("rejects a wire-contract violation: %s", (source) => {
    const result = compile(source);
    expect(result.status).toBe(1);
    expect(result.output).toMatch(/error TS(2322|2741)/);
  });

  it("fails rather than widening an unresolved schema reference", () => {
    expect(() => renderKernelProtocol({
      ...spec, $defs: { Broken: { $ref: "#/$defs/Missing" } },
    })).toThrow(KernelSchemaError);
  });

  it("fails rather than ignoring a new schema keyword", () => {
    const changed: KernelSpec = JSON.parse(JSON.stringify({
      ...spec, $defs: { Broken: { type: "string", not: { const: "forbidden" } } },
    }));
    expect(() => renderKernelProtocol(changed)).toThrow(KernelSchemaError);
  });
});
