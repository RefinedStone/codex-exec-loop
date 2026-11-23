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

function compile(source: string, protocol = generated) {
  const directory = mkdtempSync(resolve(tmpdir(), "akra-kernel-types-"));
  try {
    writeFileSync(resolve(directory, "fixture.ts"), `${protocol}\n${source}`);
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
      const issueMode: KernelCommand = { type: "set_issue_automation_mode", mode: "observe" };
      const sync: KernelCommand = { type: "sync_issues" };
      const issueQuery: KernelQuery = { type: "issues", after: null, limit: 100 };
      const issueStatus: KernelQueryAnswer = { type: "issue_automation", status: "unavailable" };
      const presentation: IssueListPresentation = { number: null, url: null, title: null, status: { code: "not_yet_observed", label: "관찰 전" }, reason: { code: "not_yet_observed", summary: "다음 관찰을 기다립니다." } };
      const issueItem: IssueListItem = {
        issue: { provider: "github", server_id: "github.com", repository_id: "R_repo", issue_id: "I_issue", project_scope: { workspace_dir: "/work/repo" } },
        binding_id: "origin", tracked: null, current: { status: "not_yet_observed" }, presentation,
      };
      const issueList: KernelQueryAnswer = { type: "issues", status: "ready", revision: 1, items: [issueItem], groups: [{ id: "collection", label: "수집한 이슈", item_indices: [0] }], next_after: null };
      const issueEvent: KernelEvent = { type: "issues", status: "unavailable" };
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
    'const value: KernelCommand = { type: "set_issue_automation_mode", mode: "auto" };',
    'const value: KernelQueryAnswer = { status: "unavailable" };',
    'const value: KernelEvent = { type: "issues", status: "ready" };',
    'const value: KernelQuery = { type: "issues", after: { opaque: "source-cursor" } };',
    'const value: KernelQueryAnswer = { type: "issues", status: "ready", revision: 1, items: [], next_after: null };',
    'const value: IssueListPresentation = { number: null, url: null, title: null, status: { code: "unknown", label: "확인 불가" }, reason: { code: "source_read_failed", summary: 12 } };',
  ])("rejects a wire-contract violation: %s", (source) => {
    const result = compile(source);
    expect(result.status).toBe(1);
    expect(result.output).toMatch(/error TS(2322|2741)/);
  });

  it("supports bounded numeric schema fields without widening their numeric type", () => {
    const changed: KernelSpec = {
      ...spec, $defs: { ...spec.$defs, IssuePageLimit: { type: "integer", minimum: 1, maximum: 100 } },
    };
    const protocol = renderKernelProtocol(changed);
    expect(protocol).toContain("export type IssuePageLimit = number;");
    expect(compile('const limit: IssuePageLimit = "100";', protocol).status).toBe(1);
  });

  it("generates host reservation and execution metadata without caller execution inputs or private fields", () => {
    const result = compile(`
      const request: KernelCommand = {
        type: "request_task_verification", request_id: "request-1", task_id: "task-1",
        criterion_id: "whitespace", scope_id: "a".repeat(64), expected_acceptance_revision: 3,
      };
      const accepted: CommandResult = { type: "task_verification_requested", attempt_id: "b".repeat(64), changed: true };
      const legacy: TaskAcceptanceCriterion = { id: "a", scenario: "Result", channel: "command", essential: true };
      const selected: TaskAcceptanceCriterion = { ...legacy, verification_recipe: "git_diff_check_v1" };
      const noRecipe: TaskAcceptanceCriterion = { ...legacy, verification_recipe: null };
      const pending: TaskVerificationState = { phase: "pending", state: "pending" };
      const attempt: TaskVerificationAttemptRecord = {
        attempt_id: "b".repeat(64), scope: { task_id: "task-1", scope_id: "a".repeat(64), acceptance_revision: 3, criteria_sha256: "c".repeat(64) },
        criterion_id: "whitespace", recipe: "git_diff_check_v1", attempt: 1, requested_at: "2026-11-23T00:00:00Z",
      };
      const record: TaskVerificationRecord = { attempt, result: { kind: "interrupted" }, finished_at: "2026-11-23T00:00:01Z" };
      const event: KernelRecord = { kind: "task_verification_finished", event_id: "task-verification-finished:" + attempt.attempt_id, evidence_revision: 1, verification: record };
      const criterion: PlanningCriterionEvidenceState = { criterion: selected, state: "fail", host_verification: pending };

      // Each expected diagnostic must remain present when the Rust schema changes.
      // @ts-expect-error execution arguments are host-owned
      const callerArgv: KernelCommand = { ...request, argv: ["git", "diff"] };
      // @ts-expect-error the caller cannot choose an execution result
      const callerStatus: KernelCommand = { ...request, status: "pass" };
      // @ts-expect-error the existing acceptance editor chooses the recipe
      const callerRecipe: KernelCommand = { ...request, recipe: "git_diff_check_v1" };
      // @ts-expect-error private session authority is not a public attempt field
      const privateSession: TaskVerificationAttemptRecord = { ...attempt, session_key: "private" };
      // @ts-expect-error worker lease authority is not a public attempt field
      const privateLease: TaskVerificationAttemptRecord = { ...attempt, lease_generation: "private" };
      // @ts-expect-error worker slot identity is not a public attempt field
      const privateSlot: TaskVerificationAttemptRecord = { ...attempt, slot_id: "slot-private" };
      // @ts-expect-error capture paths are never public receipt metadata
      const privatePath: TaskVerificationRecord = { ...record, artifact_path: "/private/capture" };
      // @ts-expect-error process output is never public receipt metadata
      const rawOutput: TaskVerificationRecord = { ...record, stdout: "private process output" };
      // @ts-expect-error unsupported recipes must remain closed
      const unknownRecipe: TaskAcceptanceCriterion = { ...legacy, verification_recipe: "worker_command" };
    `);
    expect(result).toEqual({ status: 0, output: "" });
  });

  it("supports nullable bounded arrays while preserving their element type", () => {
    const changed: KernelSpec = {
      ...spec,
      $defs: {
        ...spec.$defs,
        TaskEvidenceTaskIds: { type: ["array", "null"], items: { type: "string" }, minItems: 1, maxItems: 256 },
      },
    };
    const protocol = renderKernelProtocol(changed);
    expect(protocol).toContain("export type TaskEvidenceTaskIds = (Array<string> | null);");
    expect(compile(`
      const selected: TaskEvidenceTaskIds = ["task-1", "task-2"];
      const all: TaskEvidenceTaskIds = null;
      const query: KernelQuery = { type: "task_evidence", task_ids: selected, limit: 256 };
    `, protocol)).toEqual({ status: 0, output: "" });
    expect(compile('const ids: TaskEvidenceTaskIds = [123];', protocol).status).toBe(1);
    expect(() => renderKernelProtocol({
      ...spec, $defs: { Broken: { type: "array", minItems: 1, maxItems: 256 } },
    })).toThrow("Broken: array needs items");
  });

  it.each(["oneOf", "anyOf"] as const)("preserves a sibling object discriminator beside %s", (keyword) => {
    const changed: KernelSpec = {
      ...spec,
      $defs: {
        ...spec.$defs,
        TaggedIssueState: {
          [keyword]: [
            { type: "object", properties: { status: { const: "unavailable" } }, required: ["status"] },
            { type: "object", properties: { status: { const: "ready" }, revision: { type: "integer" } }, required: ["status", "revision"] },
          ],
          type: "object", properties: { type: { const: "issues" } }, required: ["type"],
        },
      },
    };
    const protocol = renderKernelProtocol(changed);
    expect(compile(`
      const ready: TaggedIssueState = { type: "issues", status: "ready", revision: 1 };
      const unavailable: TaggedIssueState = { type: "issues", status: "unavailable" };
    `, protocol)).toEqual({ status: 0, output: "" });
    for (const source of [
      'const value: TaggedIssueState = { status: "unavailable" };',
      'const value: TaggedIssueState = { type: "other", status: "unavailable" };',
      'const value: TaggedIssueState = { type: "issues", status: "ready" };',
    ]) {
      expect(compile(source, protocol).status).toBe(1);
    }
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
