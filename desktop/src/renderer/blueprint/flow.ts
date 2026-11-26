// 사용자 흐름(plan 33)의 계산이다: 주의 묶음과 이유, 전달 단계와 다음 행동. 모두 순수 함수이다.
// 실제 앱에서는 kernel 의 주의 항목 목록(plan 23)과 git·PR 관찰이 이 값을 준다. 블루프린트는 mock 스레드에서 계산한다.
import type { MockApproval, MockThread } from "./mockData";

export type AttentionGroup = "mine" | "read" | "working" | "rest";

/** 묶음의 이름과 순서는 plan 23 §2 를 따른다. */
export const ATTENTION_GROUPS: { id: AttentionGroup; title: string; hint: string }[] = [
  { id: "mine", title: "내 차례", hint: "답하지 않은 요구가 있습니다" },
  { id: "read", title: "읽을 것", hint: "끝났고 아직 읽지 않았습니다" },
  { id: "working", title: "작업 중", hint: "에이전트가 작업합니다" },
  { id: "rest", title: "나머지", hint: "기다리는 것이 없습니다" },
];

export interface Attention {
  group: AttentionGroup;
  /** 행에 보이는 이유 1 줄이다. 없으면 빈 문자열이다. */
  reason: string;
}

/** 완료로 정리했거나 미룬 스레드는 목록에서 뺀다. */
export function isListed(thread: MockThread): boolean {
  return !thread.done && thread.snoozedUntil === null;
}

export type DeliveryStepId = "review" | "verify" | "commit" | "push" | "pr" | "checks" | "merge";

export interface DeliveryStep {
  id: DeliveryStepId;
  label: string;
  /** `active` 는 다음에 할 단계이다. `blocked` 는 다음 단계이고 실패가 막고 있다. */
  state: "done" | "active" | "blocked" | "todo";
}

export interface Delivery {
  steps: DeliveryStep[];
  /** 끝나지 않은 첫 단계이다. 모두 끝났으면 null 이다. */
  next: DeliveryStep | null;
  /** 다음 단계의 버튼 글이다. 사람이 할 일이 없으면 (CI 검사 중) null 이다. */
  action: string | null;
  /** 다음 단계가 왜 남았는지의 1 줄이다. */
  hint: string;
}

const ACTIONS: Record<DeliveryStepId, { action: string | null; hint: string }> = {
  review: { action: "변경 검토", hint: "아직 보지 않은 파일이 있습니다" },
  verify: { action: "검증 맡기기", hint: "증거가 없는 완료 기준이 있습니다" },
  commit: { action: "커밋", hint: "커밋하지 않은 변경이 있습니다" },
  push: { action: "push", hint: "push 하지 않은 커밋이 있습니다" },
  pr: { action: "PR 만들기", hint: "이 브랜치의 PR 이 없습니다" },
  checks: { action: null, hint: "CI 검사가 끝나기를 기다립니다" },
  merge: { action: "GitHub 에서 열기", hint: "CI 가 통과했습니다. 머지할 수 있습니다" },
};

/**
 * 변경에서 머지까지의 단계이다. 변경도 PR 도 없는 스레드는 null 이다.
 * 단계는 안내이다. 앞 단계가 남아도 뒤 단계를 막지 않는다 (plan 24 의 observe).
 */
export function deliveryOf(thread: MockThread, viewedFiles: number, changedFiles: number): Delivery | null {
  const pullRequest = thread.pullRequest;
  if (thread.added + thread.removed === 0 && !pullRequest) return null;
  const merged = pullRequest?.merged === true;
  const checks = pullRequest?.checks;
  const done: Record<DeliveryStepId, boolean> = {
    review: merged || (changedFiles > 0 && viewedFiles >= changedFiles),
    verify: merged || !thread.criteria || thread.criteria.passed >= thread.criteria.total,
    commit: merged || thread.committed,
    push: merged || thread.pushed,
    pr: pullRequest !== null,
    checks: merged || checks === "pass",
    merge: merged,
  };
  const labels: [DeliveryStepId, string][] = [
    ["review", changedFiles > 0 && !merged ? `검토 ${Math.min(viewedFiles, changedFiles)}/${changedFiles}` : "검토"],
    ["verify", thread.criteria ? `검증 ${thread.criteria.passed}/${thread.criteria.total}` : "검증"],
    ["commit", "커밋"],
    ["push", "push"],
    ["pr", pullRequest ? `PR #${pullRequest.number}` : "PR"],
    ["checks", checks === "fail" ? "CI 실패" : checks === "pending" ? "CI 검사 중" : "CI"],
    ["merge", "머지"],
  ];
  // 완료 기준이 없는 스레드에는 검증 단계를 그리지 않는다 (plan 23 §5).
  const shown = labels.filter(([id]) => id !== "verify" || thread.criteria !== null);
  // 실패한 CI 는 앞 단계가 남아도 먼저이다. 고치면 변경이 바뀌고 앞 단계를 다시 한다.
  const failed = checks === "fail" && !merged;
  const nextId = failed ? "checks" : (shown.find(([id]) => !done[id])?.[0] ?? null);
  const steps = shown.map(([id, label]): DeliveryStep => ({ id, label, state: done[id] ? "done" : id !== nextId ? "todo" : failed ? "blocked" : "active" }));
  const next = steps.find((step) => step.id === nextId) ?? null;
  if (!next) return { steps, next: null, action: null, hint: "" };
  if (failed) return { steps, next, action: "실패 기록 넘기기", hint: "CI 검사가 실패했습니다" };
  if (next.id === "verify") return { steps, next, action: ACTIONS.verify.action, hint: `증거가 없는 기준: ${thread.criteria?.missing ?? ""}` };
  return { steps, next, ...ACTIONS[next.id] };
}

/** 스레드의 주의 묶음과 이유이다. 이유는 원인(승인, 실패)이 먼저이고, 없으면 전달의 다음 단계이다. */
export function attentionOf(thread: MockThread, approval: MockApproval | undefined, delivery: Delivery | null): Attention {
  const next = delivery?.next ? `다음: ${delivery.next.label}` : "";
  if (thread.status === "approval") {
    return { group: "mine", reason: approval ? `승인 필요: ${approval.command}` : "승인 필요" };
  }
  // PR 실패는 턴의 실행 상태와 별개이다. 후속 턴이 끝나도 해결 전까지 내 차례에 남는다.
  if (thread.pullRequest?.checks === "fail" && !thread.pullRequest.merged) {
    return { group: "mine", reason: `PR #${thread.pullRequest.number} 의 CI 실패` };
  }
  switch (thread.status) {
    case "failed":
      return { group: "mine", reason: "턴 실패" };
    case "unread":
      return { group: "read", reason: next ? `끝남 · ${next}` : "끝남" };
    case "running":
      return { group: "working", reason: "" };
    case "idle":
      return { group: "rest", reason: next };
  }
}

/** `currentId` 다음의 "내 차례" 스레드이다. 끝에서는 처음으로 돌아간다. 하나도 없으면 null 이다. */
export function nextInTurn(threads: readonly MockThread[], currentId: string | null): string | null {
  const mine = threads.filter((thread) => isListed(thread) && attentionOf(thread, undefined, null).group === "mine").map((thread) => thread.id);
  return mine[(mine.indexOf(currentId ?? "") + 1) % mine.length] ?? null;
}
