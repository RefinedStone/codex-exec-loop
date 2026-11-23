import type { IssueAutomationAnswer, IssueBindingPauseReceiptRecord, IssueBindingPauseState, KernelCommand } from "./protocol";

export function bindingPauseControlLabel(state: IssueBindingPauseState | null | undefined): string {
  const current = state?.status === "ready" ? state : null;
  const pending = current?.pending;
  return pending ? pending.paused ? "일시정지 처리 중" : "재개 확인 중"
    : current ? current.paused ? "일시정지됨" : "일시정지 해제됨" : "일시정지 상태 확인 필요";
}

export function bindingPauseControlAction(
  state: IssueBindingPauseState | null | undefined,
  connected: boolean,
  busy: boolean,
  canChange: boolean,
): { label: string; disabled: boolean } {
  const current = state?.status === "ready" ? state : null;
  const pending = current?.pending;
  return {
    label: pending ? "처리 중…" : current?.paused ? "이슈 자동화 재개…" : "이슈 자동화 일시정지",
    disabled: !connected || busy || !current || pending != null || !canChange,
  };
}

type ObjectValue = Record<string, unknown>;
const record = (value: unknown): value is ObjectValue => value !== null && typeof value === "object" && !Array.isArray(value);
const revision = (value: unknown): value is number => typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
const identifier = (value: unknown, max: number): value is string => typeof value === "string" && value.length > 0
  && value.trim() === value && new TextEncoder().encode(value).length <= max && !/[\p{Cc}]/u.test(value);
// Persisted IssueAutomationBindingId keeps its exact text; only blank and NUL are invalid.
// New commands and receipts retain the stricter bounded identifier contract above.
const storedBindingId = (value: unknown): value is string => typeof value === "string"
  && /[^\p{White_Space}]/u.test(value) && !value.includes("\0");
const fields = (value: ObjectValue, allowed: readonly string[]) => Object.keys(value).every((key) => allowed.includes(key));

export function hasValidBindingPause(value: ObjectValue): boolean {
  const pause = value.binding_pause;
  if (pause == null) return true; // Older kernels do not expose binding controls.
  if (!record(pause) || !storedBindingId(pause.binding_id) || pause.binding_id !== value.binding_id) return false;
  if (pause.status === "unavailable") return fields(pause, ["status", "binding_id"]);
  if (pause.status !== "ready" || !revision(pause.revision) || typeof pause.paused !== "boolean"
    || !fields(pause, ["status", "binding_id", "revision", "paused", "pending"])) return false;
  return pause.pending == null || (record(pause.pending) && fields(pause.pending, ["command_id", "paused"])
    && identifier(pause.pending.command_id, 256) && typeof pause.pending.paused === "boolean");
}

export function isBindingPauseReceipt(value: unknown): value is IssueBindingPauseReceiptRecord {
  if (!record(value) || !fields(value, ["command_id", "binding_id", "revision", "paused", "outcome", "changed"])
    || !identifier(value.command_id, 256) || !identifier(value.binding_id, 512) || !revision(value.revision)
    || typeof value.paused !== "boolean" || typeof value.changed !== "boolean") return false;
  switch (value.outcome) {
    case "paused": return value.paused && value.changed;
    case "resumed": return !value.paused && value.changed;
    case "unchanged": return !value.changed;
    case "resume_superseded": return value.paused && !value.changed;
    default: return false;
  }
}

export function bindingPauseRequestNotice(
  value: unknown,
  command: Extract<KernelCommand, { type: "set_issue_binding_paused" }>,
): { notice: string; known: boolean } {
  const unknown = { notice: "요청의 접수 여부를 확인하지 못했습니다. 현재 상태를 새로 읽고 확인해 주세요. 같은 요청을 자동으로 다시 보내지 않습니다.", known: false };
  if (!record(value) || !fields(value, ["command_id", "binding_id", "requested_paused", "accepted_revision", "receipt"])
    || value.command_id !== command.command_id || value.binding_id !== command.binding_id || value.requested_paused !== command.paused
    || !revision(value.accepted_revision) || value.accepted_revision < command.expected_revision) return unknown;
  if (value.receipt == null) return { notice: command.paused
    ? "일시정지 요청을 접수했습니다. 이미 시작한 작업의 처리가 정리될 때까지 기다립니다."
    : "재개 요청을 접수했습니다. 현재 정책과 권한을 다시 확인하고 있습니다.", known: true };
  const receipt = value.receipt;
  if (!isBindingPauseReceipt(receipt) || receipt.command_id !== command.command_id || receipt.binding_id !== command.binding_id
    || receipt.revision !== value.accepted_revision || (receipt.outcome !== "resume_superseded" && receipt.paused !== command.paused)
    || (receipt.outcome === "resume_superseded" && command.paused)) return unknown;
  if (receipt.outcome === "unchanged") return { notice: "설정이 이미 요청한 상태입니다.", known: true };
  return { notice: receipt.outcome === "resume_superseded" ? "확인 중 기준이 바뀌어 일시정지를 유지했습니다. 현재 상태에서 다시 확인해 주세요."
    : receipt.paused ? "이 연결의 이슈 자동화를 일시정지했습니다." : "이 연결의 이슈 자동화를 재개했습니다.", known: true };
}

/** Collection/catalog revision and binding run revision are independent. */
export function preserveNewerBindingPause(incoming: IssueAutomationAnswer, previous: IssueAutomationAnswer | null): IssueAutomationAnswer {
  if (incoming.status !== "ready" || previous?.status !== "ready" || incoming.binding_id !== previous.binding_id) return incoming;
  const next = incoming.binding_pause;
  const prior = previous.binding_pause;
  if (next?.status === "ready" && prior?.status === "ready"
    && (next.revision < prior.revision || (next.revision === prior.revision && prior.pending == null && next.pending != null))) {
    return { ...incoming, binding_pause: prior };
  }
  return incoming;
}
