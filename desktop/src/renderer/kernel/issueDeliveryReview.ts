import type { IssueDecisionRequestState } from "./protocol";

const REASON_LABEL = {
  protected_path: "보호 경로 변경",
  diff_too_large: "변경량 한도 초과",
  binary_diff: "바이너리 변경",
} as const;

/** 저장된 결정의 근거를 표시한다. 위험도나 전달 허가는 클라이언트에서 판정하지 않는다. */
export function issueDeliveryReview(request: IssueDecisionRequestState) {
  const range = request.basis.delivery;
  const review = request.delivery_review;
  if (request.kind !== "risk_exception" || request.gate !== "delivery"
    || !request.basis.work_order_id || !range || !review) return null;
  return {
    baseSha: range.base_commit_sha,
    sourceSha: range.source_commit_sha,
    files: `${review.changed_files.toLocaleString("ko-KR")}개`,
    lines: review.changed_lines == null ? "미확인 · 바이너리 포함" : `${review.changed_lines.toLocaleString("ko-KR")}줄 (추가 + 삭제)`,
    reasons: review.reasons.map((code) => ({ code, label: REASON_LABEL[code] })),
  };
}
