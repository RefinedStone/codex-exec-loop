// push 버튼과 PR 만들기 양식을 언제 보여 줄지, 양식의 처음 값이 무엇인지 정한다. 순수 함수이다.
import type { GitBranchSummary } from "../../shared/bridge";

export type LoadedSummary = Extract<GitBranchSummary, { ok: true }>;

/** push 할 커밋이 있다: push 전인 커밋이 있고 브랜치가 있다. */
export function canPush(summary: LoadedSummary): boolean {
  return summary.branch !== null && summary.commits.some((commit) => commit.pushed === false);
}

/** PR 을 만들 수 있다: GitHub 저장소이고, 브랜치를 모두 push 했고, 커밋이 있고, 열린 PR 이 없다. */
export function canCreatePullRequest(summary: LoadedSummary): boolean {
  return (
    summary.repository !== null &&
    summary.branch !== null &&
    summary.remoteBranch !== null &&
    summary.base !== null &&
    summary.commits.length > 0 &&
    !canPush(summary) &&
    summary.pullRequest?.state !== "OPEN"
  );
}

/** 새 PR 의 처음 값이다. 제목은 브랜치의 첫 커밋 제목, 본문은 커밋 제목 목록(오래된 것부터)이다. */
export function initialPullRequestDraft(summary: LoadedSummary): { title: string; body: string; base: string } {
  const oldestFirst = [...summary.commits].reverse();
  return {
    title: oldestFirst[0]?.subject ?? "",
    body: oldestFirst.length > 1 ? oldestFirst.map((commit) => `- ${commit.subject}`).join("\n") : "",
    base: (summary.base ?? "").replace(/^origin\//, ""),
  };
}
