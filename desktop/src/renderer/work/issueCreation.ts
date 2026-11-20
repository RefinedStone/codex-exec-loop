// 새 GitHub 이슈는 번호가 생기기 전에 저장소와 생성 의도로 식별한다 (#2436).
// 원문은 저장하지 않는다. 승인과 재개는 Gateway가 준 식별자/공유본에 묶인다.
import { sameApproval, type IssueDraftApproval, type IssueDraftOpened, type IssueDraftPreview } from "./issueDraft";

export const ISSUE_CREATE_METHOD = {
  open: "akra/issue/create/open",
  preview: "akra/issue/create/preview",
  publish: "akra/issue/create/publish",
  recheck: "akra/issue/create/recheck",
  confirm: "akra/issue/create/confirm",
  restart: "akra/issue/create/restart",
} as const;

export interface IssueCreationTarget {
  cwd: string;
  owner: string;
  repository: string;
}

export interface IssueRepositoryReference {
  provider: "github";
  serverId: string;
  repositoryId: string;
  projectScope: { workspaceDir: string };
}

export interface IssueCreationApproval extends IssueDraftApproval {
  repository: IssueRepositoryReference;
  intentId: string;
}

export type IssueCreationIntentState = "prepared" | "held" | "recorded" | "sent" | "needs_confirmation" | "confirmed" | "abandoned";

export interface IssueCreationIntent {
  intentId: string;
  state: IssueCreationIntentState;
  created: {
    issue: IssueRepositoryReference & { issueId: string };
    locator: { owner: string; repository: string; number: number };
    url: string;
  } | null;
  sharedCopy: { title: string; body: string } | null;
  restoreLinkRequired: boolean;
  restoreLinkFinalized: boolean;
  predecessorIntentId: string | null;
}

export interface IssueCreationOpened extends Pick<IssueDraftOpened, "title" | "body" | "template" | "requiredSections" | "spanActions"> {
  repository: IssueRepositoryReference;
  intent: IssueCreationIntent;
}

export interface IssueCreationPreview {
  preview: IssueDraftPreview;
  approval: IssueCreationApproval | null;
}

export type IssueCreationPublished =
  | { status: "confirmed" | "needs_confirmation" | "held"; intent: IssueCreationIntent }
  | { status: "refused"; reason: string }
  | { status: "blocked"; preview: IssueCreationPreview };

export function issueCreationTarget(cwd: string, repository: string): IssueCreationTarget | null {
  const [owner, name, ...rest] = repository.split("/");
  return owner && name && rest.length === 0 ? { cwd, owner, repository: name } : null;
}

export function sameCreationApproval(left: IssueCreationApproval, right: IssueCreationApproval): boolean {
  return sameApproval(left, right) && left.intentId === right.intentId &&
    left.repository.provider === right.repository.provider &&
    left.repository.serverId === right.repository.serverId &&
    left.repository.repositoryId === right.repository.repositoryId &&
    left.repository.projectScope.workspaceDir === right.repository.projectScope.workspaceDir;
}

export function creationNeedsConfirmation(intent: IssueCreationIntent): boolean {
  return intent.state === "sent" || intent.state === "needs_confirmation";
}

export function creationRestorePending(intent: IssueCreationIntent): boolean {
  return intent.state === "confirmed" && intent.restoreLinkRequired && !intent.restoreLinkFinalized;
}

export function creationStatus(intent: IssueCreationIntent): string {
  if (creationRestorePending(intent)) return "restore_pending";
  if (creationNeedsConfirmation(intent)) return "needs_confirmation";
  return intent.state;
}
