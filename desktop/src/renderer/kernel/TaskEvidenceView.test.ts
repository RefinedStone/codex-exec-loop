import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { taskEvidenceFixture } from "../../../test/fixtures/taskEvidence";
import type { PlanningEvidenceFreshness } from "./kernelConnection";
import { TaskEvidenceView } from "./TaskEvidenceView";
import { isTaskEvidenceState } from "./taskEvidenceValidation";

const fixture = () => structuredClone(taskEvidenceFixture());

function render(snapshot: unknown, freshness: PlanningEvidenceFreshness = "current", taskId = "task-1"): string {
  if (snapshot != null && !isTaskEvidenceState(snapshot)) throw new Error("The test must use valid kernel evidence metadata.");
  return renderToStaticMarkup(createElement(TaskEvidenceView, { taskId, snapshot, freshness }));
}

function statusBadge(html: string): string {
  const badge = html.match(/<span\b[^>]*data-testid="task-evidence-status"[^>]*>.*?<\/span>/s);
  expect(badge).not.toBeNull();
  return badge![0];
}

function waivedFixture() {
  const snapshot = fixture();
  const task = snapshot.tasks[0]!;
  const entry = task.criteria[0]!;
  return {
    ...snapshot,
    tasks: [{
      ...task,
      criteria: [{
        ...entry,
        state: "waived",
        waiver: {
          event_id: "criterion-waiver:fixture", evidence_revision: 4,
          waiver: {
            scope: { ...task.scope }, criterion_id: entry.criterion.id,
            reason_digest: "1".repeat(64), by: "human", recorded_at: "2026-11-23T00:00:02Z",
          },
        },
      }],
    }],
  };
}

describe("task evidence view", () => {
  it("shows criteria and worker metadata in native read-only details", () => {
    const html = render(fixture());
    expect(html).toContain('data-testid="task-evidence"');
    expect(html).toContain('data-task-id="task-1"');
    expect(html).toContain("<summary>완료 기준 · 증거 1개</summary>");
    expect(html).toContain("The expected output exists.");
    expect(html).toContain("criterion with spaces");
    expect(html).toContain("필수");
    expect(html).toContain("명령");
    expect(statusBadge(html)).toContain("작업자 보고: 통과");
    expect(statusBadge(html)).toContain("badge-ok");
    expect(html).toContain("통과·실패·막힘은 작업자가 보고한 결과입니다.");
    expect(html).toContain("fixture-worker");
    expect(html).toMatch(/<time datetime="2026-11-23T00:00:01Z"/i);
    expect(html).toContain("f".repeat(64));
    expect(html).toContain("d".repeat(40));
    expect(html).toContain("42 바이트");
    expect(html).toContain("<summary>조회 기준 정보</summary>");
    expect(html).not.toMatch(/<(button|input|textarea|a)\b/);
    expect(html).not.toContain("검증 완료");
  });

  it.each([
    ["pass", "작업자 보고: 통과", "badge-ok"],
    ["fail", "작업자 보고: 실패", "badge-bad"],
    ["blocked", "작업자 보고: 막힘", "badge-warn"],
  ])("labels a current %s as a worker report", (state, label, tone) => {
    const snapshot = fixture();
    const entry = snapshot.tasks[0]!.criteria[0]!;
    entry.state = state;
    entry.evidence.evidence.status = state;
    const badge = statusBadge(render(snapshot));
    expect(badge).toContain(label);
    expect(badge).toContain(tone);
  });

  it("shows pending criteria without inventing a report", () => {
    const snapshot = fixture();
    const task = snapshot.tasks[0]!;
    const entry = task.criteria[0]!;
    const html = render({
      ...snapshot, tasks: [{
        ...task, criteria: [{ criterion: { ...entry.criterion, essential: false, channel: "screen" }, state: "pending" }],
      }],
    });
    expect(statusBadge(html)).toContain("보고 대기");
    expect(html).toContain("선택");
    expect(html).toContain("화면");
    expect(html).not.toContain("fixture-worker");
    expect(html).not.toContain("파일 SHA-256");
  });

  it("keeps an old-scope Pass historical when the kernel reports stale", () => {
    const snapshot = fixture();
    const entry = snapshot.tasks[0]!.criteria[0]!;
    entry.state = "stale";
    entry.evidence.evidence.scope.scope_id = "0".repeat(64);
    const html = render(snapshot);
    expect(statusBadge(html)).toContain("이전 범위의 보고");
    expect(statusBadge(html)).toContain('data-current="false"');
    expect(statusBadge(html)).not.toContain("통과");
    expect(html).toContain("<summary>이전 작업자 보고</summary>");
    expect(html).toContain("이전 보고 결과");
    expect(html).toContain("0".repeat(64));
    expect(html).not.toContain("badge-ok");
  });

  it("shows native Human waiver metadata without inventing the reason text", () => {
    const html = render(waivedFixture());
    expect(statusBadge(html)).toContain("사람 면제");
    expect(html).toContain("<summary>사람 면제 기록</summary>");
    expect(html).toContain("<summary>이전 작업자 보고</summary>");
    expect(html).toContain("사유 SHA-256");
    expect(html).toContain("1".repeat(64));
    expect(html).toContain("면제 사유 원문은 제공되지 않습니다.");
    expect(html).not.toContain("badge-ok");
  });

  it.each([undefined, null])("distinguishes omitted evidence %s from an empty criterion list", (snapshot) => {
    const html = render(snapshot);
    expect(html).toContain("기준·증거 정보가 제공되지 않았습니다.");
    expect(html).toContain('data-testid="task-evidence-unsupported"');
    expect(html).not.toContain("등록된 완료 기준이 없습니다.");
  });

  it("does not treat a task outside the first page as having no criteria", () => {
    const snapshot = fixture();
    const html = render({ ...snapshot, truncated: true, next_task_id: "task-1" }, "current", "task-2");
    expect(html).toContain("이 작업은 현재 조회 범위에 포함되지 않았습니다.");
    expect(html).not.toContain("등록된 완료 기준이 없습니다.");
    expect(html).not.toContain("The expected output exists.");
    expect(html).not.toContain('data-testid="task-evidence-status"');
  });

  it("keeps a missing task distinct from an explicitly empty task", () => {
    const snapshot = fixture();
    const missing = render(snapshot, "current", "task-2");
    expect(missing).toContain("이 작업의 기준·증거 항목이 반환되지 않았습니다.");
    expect(missing).not.toContain("등록된 완료 기준이 없습니다.");
    snapshot.tasks[0]!.criteria = [];
    const empty = render(snapshot);
    expect(empty).toContain("등록된 완료 기준이 없습니다.");
    expect(empty).toContain('data-testid="task-evidence-empty"');
  });

  it("shows the page boundary even when the selected task is present", () => {
    const html = render({ ...fixture(), truncated: true, next_task_id: "task-1" });
    expect(html).toContain("전체 작업 중 일부를 조회했습니다.");
    expect(html).toContain("The expected output exists.");
    expect(statusBadge(html)).toContain("작업자 보고: 통과");
  });

  it("shows a sanitized invalid snapshot without retaining a positive state", () => {
    const html = render({ unavailable: "invalid", tasks: [], truncated: false });
    expect(html).toContain("증거 정보의 형식이 올바르지 않습니다.");
    expect(html).toContain("(invalid)");
    expect(html).not.toContain('data-testid="task-evidence-status"');
    expect(html).not.toContain("등록된 완료 기준이 없습니다.");
    expect(html).not.toContain("badge-ok");
  });

  it("retains only historical metadata when the task source cannot be observed", () => {
    const snapshot = fixture();
    const task = snapshot.tasks[0]!;
    const html = render({
      ...snapshot,
      tasks: [{ ...task, unavailable: "artifact_missing", criteria: [{ ...task.criteria[0]!, state: null }] }],
    });
    expect(html).toContain("증거 파일을 찾지 못했습니다.");
    expect(html).toContain("The expected output exists.");
    expect(statusBadge(html)).toContain("현재 상태 확인 불가");
    expect(statusBadge(html)).toContain('data-current="false"');
    expect(statusBadge(html)).not.toContain("통과");
    expect(html).toContain("<summary>이전 작업자 보고</summary>");
    expect(html).not.toContain("badge-ok");
  });

  it.each(["refreshing", "stale"] as const)("marks retained Pass metadata as historical while %s", (freshness) => {
    const html = render(fixture(), freshness);
    expect(statusBadge(html)).toContain("마지막 조회: ");
    expect(statusBadge(html)).toContain("작업자 보고: 통과");
    expect(statusBadge(html)).toContain('data-current="false"');
    expect(html).toContain("<summary>이전 작업자 보고</summary>");
    expect(html).toContain(freshness === "refreshing" ? "다시 확인하는 중입니다." : "현재 상태를 확인하지 못했습니다.");
    expect(html).not.toContain("badge-ok");
  });

  it("marks a retained waiver as historical after a failed refresh", () => {
    const html = render(waivedFixture(), "stale");
    expect(statusBadge(html)).toContain("마지막 조회: ");
    expect(statusBadge(html)).toContain("사람 면제");
    expect(html).toContain("<summary>이전 사람 면제 기록</summary>");
    expect(html).not.toContain("<summary>사람 면제 기록</summary>");
  });

  it("escapes source text and preserves long Korean scenarios and full hashes", () => {
    const snapshot = fixture();
    const entry = snapshot.tasks[0]!.criteria[0]!;
    const longScenario = "긴완료기준을확인합니다".repeat(30);
    entry.criterion.scenario = longScenario + '<script>alert("fixture")</script>';
    entry.evidence.evidence.by = "<img src=fixture>";
    const html = render(snapshot);
    expect(html).toContain(longScenario);
    expect(html).toContain("&lt;script&gt;");
    expect(html).toContain("&lt;img src=fixture&gt;");
    expect(html).not.toMatch(/<(script|img)\b/);
    expect(html).toContain("a".repeat(64));
    expect(html).toContain("f".repeat(64));
  });
});
