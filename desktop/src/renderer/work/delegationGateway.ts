import { CHAT_CONVERSATION, type DelegatedChatSession } from "../../shared/delegatedChat";
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
  expired: "기한 만료",
} as const;

interface Evidence { source: string; at: number; delivery: string; phase: string }
interface Attempt {
  id: string;
  backend: string;
  delivery: string;
  phase: string;
  reason?: string | null;
  recovery?: string;
  resumes?: number;
  observations: Evidence[];
}

interface DelegationRecord {
  id: string;
  state: keyof typeof STATE_LABELS;
  reason: string | null;
  delivery: string;
  phase: string;
  outcome_unknown: boolean;
  updated_at: number;
  current_backend?: string | null;
  conversation_url?: string | null;
  session?: { mode: "new" } | { mode: "continue"; chat_url: string };
  can_cancel?: boolean;
  can_retry?: boolean;
  attempts?: Attempt[];
  fallbacks?: { from: string; to: string; reason: string; at: number }[];
}

interface BackendStatus {
  state: string;
  reason?: string | null;
  circuit?: string | null;
  account_match?: string | null;
  tunnel?: { host: boolean; readiness?: string; manual?: boolean; install?: string; connection?: string };
}

type Query =
  | { type: "delegations" | "delegation_backends" | "delegation_my_turn" }
  | { type: "delegation" | "delegation_result"; id: string };

type Command =
  | { type: "submit_delegation"; instruction: string; session: { mode: "new" } | { mode: "continue"; chat_url: string }; idempotency_key: string }
  | { type: "cancel_delegation" | "retry_delegation"; id: string };

interface CommandResponse {
  processed: boolean;
  result?: { type: "delegation_submitted" | "delegation_cancelled" | "delegation_retried"; delegation: DelegationRecord };
  rejection?: { message: string };
  failure?: string;
}

const backendName = (key: string) => key === "chatgpt_chat" ? "ChatGPT" : key === "codex_thread" ? "Codex" : key;

function summary(record: DelegationRecord): DelegationSummary {
  return {
    id: record.id,
    label: record.id.slice(0, 12),
    status: STATE_LABELS[record.state] ?? record.state,
    backend: backendName(record.current_backend ?? "—"),
  };
}

type KernelRequest = <T>(method: typeof KERNEL_METHOD.query | typeof KERNEL_METHOD.command, params: unknown) => Promise<T>;

export function delegationActions(request: KernelRequest, open: (url?: string) => Promise<boolean>): DelegationActions {
  const read = <T>(query: Query) => request<T>(KERNEL_METHOD.query, { query });
  const run = async (command: Command, key: string) => {
    const response = await request<CommandResponse>(KERNEL_METHOD.command, { command, idempotency_key: key });
    if (response.rejection) throw new Error(response.rejection.message);
    if (response.failure) throw new Error(response.failure);
    const expected = command.type === "submit_delegation" ? "delegation_submitted"
      : command.type === "retry_delegation" ? "delegation_retried" : "delegation_cancelled";
    if (!response.processed || response.result?.type !== expected) throw new Error("Gateway 에서 위임 요청의 결과를 확인할 수 없습니다.");
    return response.result.delegation;
  };
  return {
    async overview() {
      const [list, backendAnswer, attention] = await Promise.all([
        read<{ type: "delegations"; delegations: DelegationRecord[] }>({ type: "delegations" }),
        read<{ type: "delegation_backends"; chatgpt_chat: BackendStatus; codex_thread?: BackendStatus }>({ type: "delegation_backends" }),
        read<{ type: "delegation_my_turn"; items: { id: string; reason: string }[] }>({ type: "delegation_my_turn" }),
      ]);
      const backends: [string, BackendStatus][] = [["chatgpt_chat", backendAnswer.chatgpt_chat]];
      if (backendAnswer.codex_thread) backends.push(["codex_thread", backendAnswer.codex_thread]);
      return {
        jobs: list.delegations.map(summary),
        backends: backends.map(([key, value]) => ({
          name: backendName(key),
          status: value.state,
          details: [value.reason, value.circuit, value.tunnel?.readiness, value.account_match,
            value.tunnel?.install && `설치 ${value.tunnel.install}`, value.tunnel?.connection && `연결 ${value.tunnel.connection}`].filter(
            (item): item is string => typeof item === "string" && item.length > 0,
          ),
        })),
        myTurn: attention.items,
      };
    },
    async detail(id) {
      const { delegation } = await read<{ type: "delegation"; delegation: DelegationRecord | null }>({ type: "delegation", id });
      if (!delegation || delegation.id !== id) throw new Error("위임 작업을 찾을 수 없습니다.");
      const { text } = await read<{ type: "delegation_result"; text: string | null }>({ type: "delegation_result", id });
      const url = delegation.conversation_url ?? (delegation.session?.mode === "continue" ? delegation.session.chat_url : undefined);
      return {
        ...summary(delegation),
        label: id,
        canCancel: delegation.can_cancel === true,
        canRetry: delegation.can_retry === true,
        conversationUrl: url && CHAT_CONVERSATION.test(url) ? url : undefined,
        reason: [delegation.outcome_unknown ? "전달 여부를 확인하지 못했습니다." : null, delegation.reason].filter(Boolean).join(" "),
        result: text ?? undefined,
        attempts: delegation.attempts?.map((item) => [
          backendName(item.backend), item.delivery, item.phase, item.reason,
          item.recovery && `복구 ${item.recovery}${item.resumes === undefined ? "" : ` (${item.resumes}회)`}`,
        ].filter(Boolean).join(" · ")),
        fallbacks: delegation.fallbacks?.map(({ from, to, reason, at }) =>
          `${backendName(from)} → ${backendName(to)} · ${reason} · ${new Date(at).toISOString()}`,
        ),
        evidence: delegation.attempts?.flatMap((item) => item.observations.map((evidence) => ({
          source: evidence.source,
          observedAt: new Date(evidence.at).toISOString(),
          text: `${evidence.delivery} · ${evidence.phase}`,
        }))),
      };
    },
    async submit(instruction, session: DelegatedChatSession, key) {
      const target = session.mode === "new" ? session : { mode: session.mode, chat_url: session.chatUrl };
      return (await run({ type: "submit_delegation", instruction, session: target, idempotency_key: key }, key)).id;
    },
    async cancel(id) {
      await run({ type: "cancel_delegation", id }, crypto.randomUUID());
    },
    async retry(id, key) {
      return (await run({ type: "retry_delegation", id }, key)).id;
    },
    async openLogin(url) {
      if (!await open(url)) throw new Error(url ? "이 창에서 ChatGPT 대화를 열 수 없습니다." : "이 창에서 ChatGPT 로그인을 열 수 없습니다.");
    },
  };
}
