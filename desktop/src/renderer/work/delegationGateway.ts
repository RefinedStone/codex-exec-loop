import { KERNEL_METHOD } from "../kernel/protocol";
import type { DelegationActions, DelegationSummary } from "./DelegationPanel";

const STATE_LABELS = {
  queued: "대기 중",
  dispatching: "전달 중",
  running: "실행 중",
  interrupted: "중단됨",
  awaiting_human: "사용자 확인 필요",
  succeeded: "완료",
  failed: "실패",
  cancelled: "취소됨",
} as const;

interface DelegationRecord {
  id: string;
  state: keyof typeof STATE_LABELS;
  reason: string | null;
  delivery: string;
  phase: string;
  outcome_unknown: boolean;
  updated_at: number;
}

type Query =
  | { type: "delegations" }
  | { type: "delegation" | "delegation_result"; id: string };

type Command =
  | { type: "submit_delegation"; instruction: string; idempotency_key: string }
  | { type: "cancel_delegation"; id: string };

interface CommandResponse {
  processed: boolean;
  result?: { type: "delegation_submitted" | "delegation_cancelled"; delegation: DelegationRecord };
  rejection?: { message: string };
  failure?: string;
}

function summary(record: DelegationRecord): DelegationSummary {
  return {
    id: record.id,
    label: record.id.slice(0, 12),
    status: STATE_LABELS[record.state] ?? record.state,
    backend: "ChatGPT",
  };
}

type KernelRequest = <T>(method: typeof KERNEL_METHOD.query | typeof KERNEL_METHOD.command, params: unknown) => Promise<T>;

export function delegationActions(request: KernelRequest, open: () => Promise<boolean>): DelegationActions {
  const read = <T>(query: Query) => request<T>(KERNEL_METHOD.query, { query });
  const run = async (command: Command, key: string) => {
    const response = await request<CommandResponse>(KERNEL_METHOD.command, { command, idempotency_key: key });
    if (response.rejection) throw new Error(response.rejection.message);
    if (response.failure) throw new Error(response.failure);
    const expected = command.type === "submit_delegation" ? "delegation_submitted" : "delegation_cancelled";
    if (!response.processed || response.result?.type !== expected) throw new Error("Gateway 에서 위임 요청의 결과를 확인할 수 없습니다.");
    return response.result.delegation;
  };
  return {
    async list() {
      const answer = await read<{ type: "delegations"; delegations: DelegationRecord[] }>({ type: "delegations" });
      return answer.delegations.map(summary);
    },
    async detail(id) {
      const { delegation } = await read<{ type: "delegation"; delegation: DelegationRecord | null }>({ type: "delegation", id });
      if (!delegation || delegation.id !== id) throw new Error("위임 작업을 찾을 수 없습니다.");
      const result = delegation.state === "succeeded"
        ? (await read<{ type: "delegation_result"; text: string | null }>({ type: "delegation_result", id })).text
        : null;
      return {
        ...summary(delegation),
        label: id,
        canCancel: !["succeeded", "failed", "cancelled"].includes(delegation.state),
        reason: [delegation.outcome_unknown ? "전달 여부를 확인하지 못했습니다." : null, delegation.reason].filter(Boolean).join(" "),
        result: result ?? undefined,
        evidence: [{
          source: "위임 기록",
          observedAt: new Date(delegation.updated_at).toISOString(),
          text: `${delegation.delivery} · ${delegation.phase}`,
        }],
      };
    },
    async submit(instruction, key) {
      return (await run({ type: "submit_delegation", instruction, idempotency_key: key }, key)).id;
    },
    async cancel(id) {
      await run({ type: "cancel_delegation", id }, crypto.randomUUID());
    },
    async openLogin() {
      if (!await open()) throw new Error("이 창에서 ChatGPT 로그인을 열 수 없습니다.");
    },
  };
}
