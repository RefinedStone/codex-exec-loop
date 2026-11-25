import { taskEvidenceFixture } from "./taskEvidence.ts";

/** 합성된 host 실행 metadata. 실제 경로, 출력, 실행 capability를 포함하지 않는다. */
export function taskVerificationFixture() {
  const evidence = taskEvidenceFixture();
  const task = evidence.tasks[0]!;
  const entry = task.criteria[0]!;
  const attempt = {
    attempt_id: "1".repeat(64), scope: { ...task.scope }, criterion_id: entry.criterion.id,
    recipe: "git_diff_check_v1", attempt: 1, requested_at: "2026-11-23T00:00:00Z",
  };
  const emptyStream = {
    sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855", size_bytes: 0, complete: true,
  };
  return {
    ...evidence, evidence_revision: 6,
    tasks: [{
      ...task,
      criteria: [{
        ...entry, criterion: { ...entry.criterion, verification_recipe: "git_diff_check_v1" },
        host_verification: {
          attempt, phase: "finished", state: "passed",
          receipt: {
            event_id: `task-verification-finished:${attempt.attempt_id}`, evidence_revision: 6,
            verification: {
              attempt: { ...attempt, scope: { ...attempt.scope } },
              manifest: {
                recipe: "git_diff_check_v1", base_commit_sha: "2".repeat(40), base_tree_sha: "3".repeat(40),
                source: { ...task.source.tree }, git_binary_sha256: "4".repeat(64),
                command_sha256: "5".repeat(64), environment_sha256: "6".repeat(64), configuration_sha256: "7".repeat(64),
              },
              result: {
                kind: "executed", observation: {
                  outcome: { kind: "exited", exit_code: 0 }, started_at: "2026-11-23T00:00:01Z",
                  finished_at: "2026-11-23T00:00:01.050Z", duration_ms: 50,
                  stdout: { ...emptyStream }, stderr: { ...emptyStream },
                },
              },
              artifact: { sha256: "8".repeat(64), size_bytes: 512 }, finished_at: "2026-11-23T00:00:01.050Z",
            },
          },
        },
      }],
    }],
  };
}
