// 새 이슈의 승인 범위와 재개 상태를 검증한다 (#2436). 원문 저장소는 만들지 않는다.
import { describe, expect, it } from "vitest";

import {
  creationNeedsConfirmation,
  creationRestorePending,
  creationStatus,
  issueCreationTarget,
  sameCreationApproval,
  type IssueCreationApproval,
  type IssueCreationIntent,
} from "../src/renderer/work/issueCreation";

const approval: IssueCreationApproval = {
  repository: { provider: "github", serverId: "github.com", repositoryId: "R_widgets", projectScope: { workspaceDir: "/work/app" } },
  intentId: "intent-1",
  sharedCopyHash: { version: 1, sha256: "aa" },
  policyVersion: "p1",
  templateVersion: "t1",
};

const intent = (state: IssueCreationIntent["state"], extra: Partial<IssueCreationIntent> = {}): IssueCreationIntent => ({
  intentId: "intent-1", state, created: null, sharedCopy: null,
  restoreLinkRequired: false, restoreLinkFinalized: false, predecessorIntentId: null, ...extra,
});

describe("new issue target and approval", () => {
  it("identifies a repository before an issue number exists", () => {
    expect(issueCreationTarget("/work/app", "sample/widgets")).toEqual({ cwd: "/work/app", owner: "sample", repository: "widgets" });
    for (const invalid of ["widgets", "/widgets", "sample/", "sample/widgets/7"]) {
      expect(issueCreationTarget("/work/app", invalid)).toBeNull();
    }
  });

  it("keeps approval bound to the complete repository, project, intent, and approved copy", () => {
    expect(sameCreationApproval(approval, structuredClone(approval))).toBe(true);
    const repository = approval.repository;
    const changed: IssueCreationApproval[] = [
      { ...approval, repository: { ...repository, serverId: "github.example" } },
      { ...approval, repository: { ...repository, repositoryId: "R_elsewhere" } },
      { ...approval, repository: { ...repository, projectScope: { workspaceDir: "/work/other" } } },
      { ...approval, intentId: "intent-2" },
      { ...approval, sharedCopyHash: { version: 1, sha256: "bb" } },
      { ...approval, sharedCopyHash: { version: 2, sha256: "aa" } },
      { ...approval, policyVersion: "p2" },
      { ...approval, templateVersion: "t2" },
      { ...approval, templateVersion: null },
    ];
    for (const candidate of changed) expect(sameCreationApproval(approval, candidate)).toBe(false);
  });
});

describe("new issue resume states", () => {
  it("only sent or uncertain intents need provider result confirmation", () => {
    for (const state of ["sent", "needs_confirmation"] as const) {
      expect(creationNeedsConfirmation(intent(state))).toBe(true);
      expect(creationStatus(intent(state))).toBe("needs_confirmation");
    }
    for (const state of ["prepared", "held", "recorded", "confirmed", "abandoned"] as const) {
      expect(creationNeedsConfirmation(intent(state))).toBe(false);
      expect(creationStatus(intent(state))).toBe(state);
    }
  });

  it("distinguishes a created issue waiting only for its restoration link", () => {
    const pending = intent("confirmed", { restoreLinkRequired: true });
    expect(creationRestorePending(pending)).toBe(true);
    expect(creationStatus(pending)).toBe("restore_pending");
    expect(creationNeedsConfirmation(pending)).toBe(false);
    expect(creationStatus({ ...pending, restoreLinkFinalized: true })).toBe("confirmed");
    expect(creationRestorePending({ ...pending, state: "sent" })).toBe(false);
    expect(creationRestorePending({ ...pending, restoreLinkRequired: false })).toBe(false);
  });
});
