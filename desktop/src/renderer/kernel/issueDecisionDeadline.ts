import type { IssueDecisionRequestState } from "./protocol";

export interface IssueDecisionDeadlineDisplay {
  readonly dueAt: string;
  readonly asOf: string | null;
  readonly remaining: string;
  readonly afterDeadline: string;
}

/** 서버 조회 시각으로만 표시한다. 클라이언트 시계나 타이머는 결정을 진행하지 않는다. */
export function issueDecisionDeadline(request: IssueDecisionRequestState, asOfUnixMs?: number | null): IssueDecisionDeadlineDisplay | null {
  if (!request.due_at) return null;
  const due = Date.parse(request.due_at);
  if (!Number.isFinite(due)) return null;
  const asOf = typeof asOfUnixMs === "number" && Number.isSafeInteger(asOfUnixMs)
    && asOfUnixMs >= 0 && asOfUnixMs <= 8_640_000_000_000_000 ? asOfUnixMs : null;
  const recommended = request.options.find((option) => option.id === request.recommended)?.label;
  const afterDeadline = request.on_timeout === "default" && recommended
    ? `기한까지 답변이 없으면 권고한 ‘${recommended}’ 선택을 적용합니다.`
    : request.on_timeout === "wait"
      ? "기한 후에도 사람의 답변을 기다립니다."
      : "기한 후 처리 방식은 다시 확인해 주세요.";
  return {
    dueAt: new Date(due).toISOString(),
    asOf: asOf === null ? null : new Date(asOf).toISOString(),
    remaining: asOf === null ? "조회 시각 미확인" : remainingTime(due - asOf),
    afterDeadline,
  };
}

function remainingTime(milliseconds: number): string {
  if (milliseconds <= 0) return "기한 지남";
  if (milliseconds < 60_000) return "1분 미만 남음";
  const minutes = Math.ceil(milliseconds / 60_000);
  const days = Math.floor(minutes / (24 * 60));
  const hours = Math.floor((minutes % (24 * 60)) / 60);
  const remainder = minutes % 60;
  return [days && `${days}일`, hours && `${hours}시간`, remainder && `${remainder}분`]
    .filter(Boolean).join(" ") + " 남음";
}

export function issueDeadlineDateLabel(iso: string): string {
  return new Date(iso).toLocaleString("ko-KR", { dateStyle: "medium", timeStyle: "short" });
}
