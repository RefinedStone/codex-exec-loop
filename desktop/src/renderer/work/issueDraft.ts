// 이슈 초안의 자료이다 (#2426). Gateway 의 이슈 초안 메서드가 정책과 템플릿을 읽고, 글을 가리고, 검사하고, 게시 의도를
// 기록한 뒤 게시한다. 여기서는 답의 모양과, 화면이 쓰는 작은 규칙만 둔다. 로컬 글을 저장하거나 기록하지 않는다.

export const ISSUE_METHOD = {
  open: "akra/issue/open",
  preview: "akra/issue/preview",
  publish: "akra/issue/publish",
  recheck: "akra/issue/recheck",
  abandon: "akra/issue/abandon",
} as const;

export type IssueDraftTarget = { readonly cwd:string; readonly owner:string; readonly repository:string; readonly number:number } | { readonly cwd:string; readonly jira:import("../../shared/bridge").JiraDraftCoordinate; readonly displayKey:string };

export type IssueDraftField = "title" | "body";
export type IssueSpanClass = "secret" | "name" | "path" | "log";
export type IssueSpanAction = "remove" | "restorable_mask" | "user_confirm";

/** 사용자가 지정한 민감 구간이다. 위치는 그 칸의 글에서 본 UTF-16 단위이다 (입력창의 선택 위치). */
export interface IssueDraftSpan {
  field: IssueDraftField;
  start: number;
  end: number;
  /** 모르면 null 이다. 그때는 사용자가 처리를 고른다. */
  class: IssueSpanClass | null;
  decision: Exclude<IssueSpanAction, "user_confirm"> | null;
}

export interface IssueDraftText {
  title: string;
  body: string;
  spans: IssueDraftSpan[];
}

/** Gateway 가 만든 값이다. 내용을 보지 않고 그대로 돌려보낸다. */
export interface IssueDraftApproval {
  sharedCopyHash: { version: number; sha256: string };
  policyVersion: string;
  templateVersion: string | null;
}

export type IssueIntentState = "recorded" | "sent" | "confirmed" | "needs_confirmation" | "conflict" | "held" | "abandoned";

export interface IssueDraftIntent {
  intentId: string;
  state: IssueIntentState;
}

export interface IssueDraftTemplate {
  status: "not_configured" | "resolved" | "missing" | "unreadable" | "invalid" | "recovery_only";
  path: string | null;
  reason: string | null;
  requiredInputs: { label: string; kind: string; options: string[] }[];
  unsupportedInputs: { label: string; reason: string }[];
}

export interface IssueDraftOpened {
  /** 지금 원격의 글이다. 편집을 시작하는 글이다. */
  title: string;
  body: string;
  editBasis: IssueDraftApproval["sharedCopyHash"];
  template: IssueDraftTemplate;
  requiredSections: string[];
  spanActions: Record<IssueSpanClass, IssueSpanAction>;
  intent: IssueDraftIntent | null;
}

export interface IssueDraftValidationError {
  field: "title" | "template_field" | "required_section" | "code_validator";
  label: string | null;
  /** `failed` 는 필수 코드 검증기가 통과하지 않은 것이다. 게시할 때만 나온다. */
  kind: "missing" | "empty" | "invalid_format" | "failed" | "cancelled" | "isolation_unavailable";
}

export type IssueDraftAutonomy = "off" | "observe" | "propose" | "gated" | "auto";

/** 서버가 현재 AI 전달 공유본으로 계산한 예상이다. 실행 허가와 원격 라벨 변경은 없다. */
export type IssueDraftReadiness =
  | {
      status: "assessed";
      assessment: {
        previewOnly: true;
        readiness: "ready" | "needs_info" | "unknown";
        summary: string;
        missingSections: string[];
        errors: IssueDraftValidationError[];
        riskClass: "low" | "normal" | "high";
        riskSummary: string;
        effectiveMode: IssueDraftAutonomy;
        assumedMode: IssueDraftAutonomy;
        modeSummary: string;
        expectedGates: {
          gate: "start" | "spec" | "plan" | "delivery" | "merge" | "close";
          action: "human" | "timeout_default" | "timeout_arbiter" | "auto";
          summary: string;
          actionSummary: string;
        }[];
        reasons: { code: string; summary: string }[];
        policyVersion: string;
        disclosurePolicyVersion: string;
        sharedCopyHash: IssueDraftApproval["sharedCopyHash"];
      };
    }
  | {
      status: "cannot_assess";
      reason: "automation_policy_unavailable" | "policy_blocked" | "requirements_unavailable" |
        "disclosure_confirmation_required" | "invalid_span" | "invalid_labels" | "frozen_draft" | "invalid_input";
      summary: string;
      nextSummary: string;
      policyError: { scope: "user" | "project" | "effective"; field: string; reason: string } | null;
    };

export type IssueReadinessDisplay = IssueDraftReadiness | { status: "pending" } | { status: "unavailable" };

/** 현재 요청을 기다리는 동안 이전 결과를 숨기고, 도착한 서버 값은 그대로 표시한다. */
export function issueReadinessDisplay(result: IssueDraftReadiness | undefined, pending: boolean): IssueReadinessDisplay {
  if (pending) return { status: "pending" };
  return result ?? { status: "unavailable" };
}

/** 쉼표로 나눈 입력 형식만 옮긴다. 선택·위험·관문은 서버가 판단한다. */
export function expectedLabels(value: string): string[] {
  return value.split(",").map((label) => label.trim()).filter((label) => label.length > 0);
}

export type IssueDraftPreview =
  | {
      status: "ready";
      /** 게시될 제목과 본문이다. */
      sharedCopy: { title: string; body: string };
      /** 공유본에 남는 가림 표식이다. */
      markers: { marker: string; field: IssueDraftField }[];
      errors: IssueDraftValidationError[];
      validatorAbsent: boolean;
      publishable: boolean;
      approval: IssueDraftApproval;
      readiness?: IssueDraftReadiness;
    }
  | { status: "needs_confirmation"; spanIndexes: number[]; readiness?: IssueDraftReadiness }
  | { status: "invalid_span"; spanIndex: number; readiness?: IssueDraftReadiness };

export type IssueDraftPublished =
  | { status: "confirmed" | "needs_confirmation" | "held"; intent: IssueDraftIntent }
  | { status: "conflict" }
  | { status: "refused"; reason: string }
  | { status: "blocked"; preview: IssueDraftPreview };

/** `owner/name` 과 번호로 대상을 만든다. 저장소 이름이 그 모양이 아니면 null 이다. */
export function issueDraftTarget(cwd: string, repository: string, number: number): IssueDraftTarget | null {
  const [owner, name, ...rest] = repository.split("/");
  return owner && name && rest.length === 0 ? { cwd, owner, repository: name, number } : null;
}

/** 승인한 값이 지금 미리보기의 값과 같은지 본다. 다르면 승인을 지운다. */
export function sameApproval(left: IssueDraftApproval, right: IssueDraftApproval): boolean {
  return (
    left.sharedCopyHash.version === right.sharedCopyHash.version &&
    left.sharedCopyHash.sha256 === right.sharedCopyHash.sha256 &&
    left.policyVersion === right.policyVersion &&
    left.templateVersion === right.templateVersion
  );
}

/**
 * 한 칸의 글을 고친 뒤에 그 칸의 구간을 옮긴다. 고친 자리의 앞에 있는 구간은 그대로 두고, 뒤에 있는 구간은 길이의 차이만큼
 * 민다. 고친 자리와 겹친 구간은 버린다: 그 구간이 가리키던 글이 바뀌었다.
 */
export function shiftSpans(spans: IssueDraftSpan[], field: IssueDraftField, before: string, after: string): IssueDraftSpan[] {
  let prefix = 0;
  const shortest = Math.min(before.length, after.length);
  while (prefix < shortest && before[prefix] === after[prefix]) prefix += 1;
  let suffix = 0;
  while (suffix < shortest - prefix && before[before.length - 1 - suffix] === after[after.length - 1 - suffix]) suffix += 1;
  const changedEnd = before.length - suffix;
  const delta = after.length - before.length;
  return spans.flatMap((span) => {
    if (span.field !== field || span.end <= prefix) return [span];
    if (span.start >= changedEnd) return [{ ...span, start: span.start + delta, end: span.end + delta }];
    return [];
  });
}

/** 같은 칸의 다른 구간과 겹치지 않는 구간만 더한다. 겹치면 null 이다. */
export function addSpan(spans: IssueDraftSpan[], span: IssueDraftSpan): IssueDraftSpan[] | null {
  if (span.start >= span.end) return null;
  const overlaps = spans.some((other) => other.field === span.field && span.start < other.end && other.start < span.end);
  return overlaps ? null : [...spans, span];
}

/** 화면이 구분해서 보여 주는 게시 상태이다. 상태마다 다음 동작이 다르다. */
export type IssuePublishStatus =
  /** 게시 확인. 원격의 글이 승인한 공유본과 같다. */
  | { kind: "confirmed" }
  /** 게시 결과 확인 필요. 다시 확인하거나 확인을 그만둔다. */
  | { kind: "needs_confirmation"; intentId: string }
  /** 알려진 외부 변경. 원격 글을 다시 읽고 다시 승인한다. */
  | { kind: "external_change" }
  /** 복원 가능 구간을 저장하지 못했다. 공유본만 게시할지 정한다. */
  | { kind: "held" }
  /** 승인이 지금 내용과 다르다. 미리보기를 다시 보고 승인한다. */
  | { kind: "refused"; reason: string };

/** 게시 요청의 답에서 화면 상태를 만든다. `blocked` 는 상태가 아니라 미리보기의 오류이다. */
export function publishStatus(published: IssueDraftPublished): IssuePublishStatus | null {
  switch (published.status) {
    case "confirmed":
      return { kind: "confirmed" };
    case "needs_confirmation":
      return { kind: "needs_confirmation", intentId: published.intent.intentId };
    case "held":
      return { kind: "held" };
    case "conflict":
      return { kind: "external_change" };
    case "refused":
      return { kind: "refused", reason: published.reason };
    case "blocked":
      return null;
  }
}

/** 일지의 의도에서 화면 상태를 만든다 (열 때와 다시 확인할 때). 끝난 의도는 새 편집을 막지 않는다. */
export function intentStatus(intent: IssueDraftIntent | null): IssuePublishStatus | null {
  if (!intent) return null;
  if (intent.state === "confirmed") return { kind: "confirmed" };
  if (intent.state === "recorded" || intent.state === "sent" || intent.state === "needs_confirmation") {
    return { kind: "needs_confirmation", intentId: intent.intentId };
  }
  return null;
}

const ERROR_KIND: Record<IssueDraftValidationError["kind"], string> = { missing: "섹션이 없습니다", empty: "값이 비어 있습니다", invalid_format: "값의 형식이 맞지 않습니다", failed: "통과하지 못했습니다", cancelled: "사용자가 검증을 취소했습니다", isolation_unavailable: "필수 격리를 구성할 수 없어 실행하지 않았습니다. 게시가 차단됩니다. 지원이 검증된 호스트에서 다시 실행하세요 (Linux: Ubuntu 22.04 + bubblewrap 0.13.1, Ubuntu 24.04 기본 프로필 미지원). 격리 없는 실행은 지원하지 않습니다." };
const ERROR_FIELD: Record<Exclude<IssueDraftValidationError["field"], "title">, string> = { template_field: "템플릿 필드", required_section: "필수 섹션", code_validator: "코드 검증기" };

export function validationErrorText(error: IssueDraftValidationError): string {
  const where = error.field === "title" ? "제목" : `${ERROR_FIELD[error.field]} "${error.label ?? ""}"`;
  return `${where}: ${ERROR_KIND[error.kind]}`;
}

/** 사용자가 코드 검증을 취소해서 막혔는지이다 (#2858). */
export function cancelledByUser(errors: readonly IssueDraftValidationError[]): boolean {
  return errors.some((error) => error.kind === "cancelled");
}
