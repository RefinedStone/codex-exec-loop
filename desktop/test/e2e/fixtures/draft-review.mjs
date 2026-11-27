const reviewJob = { id: "draft-job", state: "running", caller: "issue_draft_review", can_cancel: true, reason: null };
export const reviewKernel = { kernel: "draft-workspace", queries: {}, onRequest(method, params) {
  if (params.query?.type === "delegation" && params.query.id === reviewJob.id) return { result: { type: "delegation", delegation: reviewJob } };
  if (params.query?.type === "delegation_result" && params.query.id === reviewJob.id) return { result: { type: "delegation_result", text: reviewJob.state === "succeeded" ? JSON.stringify({ suggestions: [{ field: "body", problem: "Name the step", originalText: "run with", suggestedText: "reproduce with" }] }) : null } };
  if (params.command?.type === "cancel_delegation" && params.command.id === reviewJob.id) {
    Object.assign(reviewJob, { state: "cancelled", can_cancel: false, reason: "fixture cancellation" });
    return { result: { processed: true, result: { type: "delegation_cancelled", delegation: reviewJob } } };
  }
} };
export function reviewRequest(params, evaluate) {
  const checked = evaluate(params.draft.text);
  if (!checked.publishable || checked.approval.sharedCopyHash.sha256 !== params.approval.sharedCopyHash.sha256) throw new Error("unvalidated draft");
  Object.assign(reviewJob, { state: "running", can_cancel: true, reason: null });
  return reviewJob;
}
export async function exerciseReview({panel, issue, approve, body, check, until}) {
  const request = panel.getByRole("button", { name: "공유본 검토 요청" });
  const refresh = panel.getByRole("button", { name: "검토 결과 확인" });
  const apply = panel.getByRole("button", { name: "권고 적용" });
  check("review only starts on request", !issue.requests.some(entry => entry.method === "akra/issue/review"));
  await approve.check();
  await request.click();
  await panel.getByRole("button", { name: "검토 취소" }).click();
  check("cancel is visible and does not invalidate publication", await until(async () => (await panel.innerText()).includes("fixture cancellation")) && await approve.isChecked());
  await request.click();
  const before = await body.inputValue();
  await body.fill(before + "new edit");
  Object.assign(reviewJob, { state: "succeeded", can_cancel: false });
  await refresh.click();
  await apply.waitFor();
  check("a late recommendation cannot replace a newer edit", await apply.isDisabled() && (await body.inputValue()).endsWith("new edit"));
  await request.click();
  Object.assign(reviewJob, { state: "succeeded", can_cancel: false });
  await refresh.click();
  await approve.check();
  await apply.click();
  check("applying the recommendation invalidates approval and preserves private text", !(await approve.isChecked()) && (await body.inputValue()) === (before + "new edit").replace("run with", "reproduce with"));
  check("the revised preview keeps both masks", await until(async () => (await panel.getByTestId("issue-draft-shared-body").innerText()).includes("reproduce with [removed]")) && (await panel.getByTestId("issue-draft-span").count()) === 2);
  check("review does not publish", !issue.requests.some(entry => entry.method === "akra/issue/publish"));
}
