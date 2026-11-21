import { homedir } from "node:os";
import { join, resolve } from "node:path";

import { acquireHostLease, ChatSessionService, type ChatObservation, type HostLease, type PagePort } from "akra-tunnel-core/chat";

import type { ChatDispatchOptions, ChatMethod, ChatReport } from "../delegation/messages";
import { CHAT_CONVERSATION, MAX_CHAT_RESUMES, type DelegatedChatSession } from "../shared/delegatedChat";
import type { TunnelConnection, TunnelInstall } from "./delegatedTunnelStatus";

const DELIVERY = {
  not_sent: "not_sent",
  sending: "sending",
  host_acknowledged: "accepted",
  message_observed: "observed",
  progress_observed: "observed",
  send_uncertain: "uncertain",
} as const satisfies Record<ChatObservation["delivery"], string>;

function wireObservation(value: ChatObservation) {
  return {
    delivery: DELIVERY[value.delivery],
    phase: value.phase === "running" ? "working" : value.phase === "load_failed" ? "unreachable" : value.phase,
    ...(value.phase === "blocked" ? { reason: "chat_page_blocked" } : {}),
    recovery: value.recovery,
    recoveryExhausted: value.recovery === "exhausted",
    resumes: value.resumes,
    evidence: value.evidence,
    ...(value.chatUrl && CHAT_CONVERSATION.test(value.chatUrl) ? { conversationUrl: value.chatUrl } : {}),
  };
}

const WAITING = { delivery: "not_sent", phase: "unknown" } as const;
const STARTING = { delivery: "sending", phase: "unknown" } as const;
const MISSING = { delivery: "not_sent", phase: "unreachable", reason: "chat_job_missing" } as const;
const ACCOUNT_UNKNOWN = { delivery: "not_sent", phase: "blocked", reason: "chat_account_unknown", recovery: "none", resumes: 0 } as const;
const ACCOUNT_MISMATCH = { ...ACCOUNT_UNKNOWN, reason: "chat_account_mismatch" } as const;

type AccountMatch = "unknown" | "match" | "mismatch";
type ChatReadiness = "unknown" | "ready" | "login_required" | "challenge" | "unreachable";
export interface ChatPageStatus {
  readiness: ChatReadiness;
  accountMatch: AccountMatch;
}
type Submission = {
  instruction: string;
  session: DelegatedChatSession;
  maxResumes: number;
  expectedAccountId?: string;
};

export function delegatedChatHome(): string {
  const override = process.env.AKRA_TUNNEL_HOME?.trim();
  if (override) return resolve(override);
  if (process.platform === "win32") return resolve(process.env.APPDATA?.trim() || homedir(), "AkraTunnel");
  if (process.platform === "darwin") return join(homedir(), "Library", "Application Support", "AkraTunnel");
  return resolve(process.env.XDG_CONFIG_HOME?.trim() || join(homedir(), ".config"), "akra-tunnel");
}

/** Owns the one foreground job for the one page. Core owns the durable job IDs and send semantics. */
export class DelegatedChatDriver {
  private lease: HostLease | null = null;
  private service: ChatSessionService | undefined;
  private serial = Promise.resolve();
  private readonly queued = new Map<string, Submission>();
  private readonly denied = new Map<string, typeof ACCOUNT_UNKNOWN | typeof ACCOUNT_MISMATCH>();
  private active: { id: string; observation: ChatObservation | null } | undefined;
  private starting: Promise<void> | undefined;
  private closing = false;

  constructor(private readonly options: {
    home: string;
    pageFor(id: string): PagePort | Promise<PagePort>;
    manual(): boolean;
    closePage(): void;
    pageStatus?(expectedAccountId?: string): Promise<ChatPageStatus>;
    tunnelStatus?(): Promise<{ install: TunnelInstall; connection: TunnelConnection }>;
  }) {}

  dispatch(
    method: ChatMethod, id: string, instruction?: string, options?: ChatDispatchOptions,
    report?: (value: ChatReport) => void,
  ): Promise<unknown> {
    if (method === "status") return this.status(options?.expectedAccountId);
    return this.run(async () => {
      const service = await this.core();
      if (!service) {
        if (method === "submit" || method === "observe") {
          return { delivery: "not_sent", phase: "unreachable", reason: "chat_host_busy" };
        }
        throw new Error("chat_host_busy");
      }
      if (method === "submit") {
        const denied = this.denied.get(id);
        if (denied) return denied;
        if (this.queued.has(id)) return this.options.manual() ? this.view(null) : WAITING;
        if (this.active?.id === id) {
          return this.active.observation ? this.view(this.active.observation) : WAITING;
        }
        const submission: Submission = {
          instruction: instruction!,
          session: options?.session ?? { mode: "new" },
          maxResumes: options?.maxResumes ?? MAX_CHAT_RESUMES,
          expectedAccountId: options?.expectedAccountId,
        };
        if (this.active || this.queued.size || this.options.manual()) {
          if (this.queued.size >= 32) return { delivery: "not_sent", phase: "unreachable", reason: "chat_queue_full" };
          this.queued.set(id, submission);
          return this.options.manual() ? this.view(null) : WAITING;
        }
        this.start(service, id, submission);
        return WAITING;
      }
      if (method === "readResult") {
        const text = await service.readResult(id);
        if (text === null) throw new Error("chat_result_unavailable");
        if (text.trim() && Buffer.byteLength(text) <= 1_048_576) report?.({ type: "result", result: text });
        return { text };
      }
      if (method === "stop") {
        if (this.queued.delete(id)) return {};
        if (this.denied.has(id)) return {};
        if (this.starting) throw new Error("chat_send_in_progress");
        if (this.options.manual()) throw new Error("chat_manual_action");
        const value = await service.stop(id);
        if (!value || (value.phase !== "stopped" && value.phase !== "complete")) throw new Error("chat_stop_unconfirmed");
        this.settle(id, value);
        return {};
      }
      const denied = this.denied.get(id);
      if (denied) return denied;
      if (this.options.manual()) {
        if (this.starting && this.active?.id === id) {
          return { delivery: "uncertain", phase: "blocked", reason: "chat_manual_action" };
        }
        return this.view(this.active?.id === id ? this.active.observation : null);
      }
      if (this.starting && this.active?.id === id) return STARTING;
      if (this.queued.has(id) && this.active) return WAITING;
      if (this.starting) throw new Error("chat_send_in_progress");
      if (!this.active && this.queued.has(id)) {
        const [nextId, prompt] = this.queued.entries().next().value!;
        this.queued.delete(nextId);
        this.start(service, nextId, prompt);
        if (nextId === id) return STARTING;
        return WAITING;
      }
      if (this.queued.has(id)) return WAITING;
      // The core can restore a conversation during observation. Let only the
      // foreground job navigate while another job is using the shared page.
      if (this.active && this.active.id !== id) return WAITING;
      const value = await service.observe(id);
      if (value) {
        this.settle(id, value);
        if (value.phase === "running" && value.delivery === "progress_observed") {
          report?.({ type: "checkpoint", message: "Assistant progress observed in the delegated chat." });
        }
      }
      return value ? this.view(value) : MISSING;
    });
  }

  private async status(expectedAccountId?: string) {
    const [[service, page], runtime] = await Promise.all([
      this.run(async () => {
        const service = await this.core();
        return [service, service ? await this.pageStatus(expectedAccountId) : undefined] as const;
      }),
      this.options.tunnelStatus?.(),
    ]);
    return {
      state: service ? "ready" : "unavailable",
      ...(!service && { reason: this.closing ? "chat_driver_closed" : "chat_host_busy" }),
      accountMatch: page?.accountMatch ?? "unknown",
      tunnel: { ...runtime, host: !!service, readiness: page?.readiness, manual: this.options.manual() },
    };
  }

  close(): Promise<void> {
    this.closing = true;
    this.queued.clear();
    this.denied.clear();
    this.options.closePage();
    return this.run(async () => {
      await this.starting;
      await this.lease?.release();
    });
  }

  private start(service: ChatSessionService, id: string, submission: Submission): void {
    this.active = { id, observation: null };
    this.starting = Promise.resolve().then(async () => {
      const { session, instruction, maxResumes, expectedAccountId } = submission;
      if (session.mode === "continue") {
        const { accountMatch } = await this.pageStatus(expectedAccountId);
        if (accountMatch !== "match") {
          if (this.denied.size >= 128) this.denied.delete(this.denied.keys().next().value!);
          this.denied.set(id, accountMatch === "mismatch" ? ACCOUNT_MISMATCH : ACCOUNT_UNKNOWN);
          this.active = undefined;
          return;
        }
      }
      const value = await service.submit({ requestId: id, session, instruction, maxResumes });
      this.settle(id, value);
    }).catch(() => {
      this.active = undefined;
    }).finally(() => { this.starting = undefined; });
  }

  private settle(id: string, value: ChatObservation) {
    if (this.active?.id !== id) return;
    const definitive = value.phase === "complete" || value.phase === "stopped" || value.phase === "failed";
    const cannotProceed = value.recovery === "exhausted" || value.delivery === "not_sent";
    this.active = definitive || cannotProceed ? undefined : { id, observation: value };
  }

  private view(value: ChatObservation | null) {
    if (this.options.manual()) {
      return { ...(value ? wireObservation(value) : WAITING), phase: "blocked", reason: "chat_manual_action" };
    }
    return value ? wireObservation(value) : MISSING;
  }

  private async pageStatus(expectedAccountId?: string): Promise<ChatPageStatus> {
    return this.options.pageStatus?.(expectedAccountId).catch(() => ({ readiness: "unreachable", accountMatch: "unknown" }))
      ?? { readiness: "unknown", accountMatch: "unknown" };
  }

  private async core(): Promise<ChatSessionService | undefined> {
    if (this.closing) return undefined;
    if (this.service) return this.service;
    this.lease = await acquireHostLease(this.options.home);
    if (this.lease) this.service = new ChatSessionService({ ...this.options, lease: this.lease });
    return this.service;
  }

  private run<T>(action: () => Promise<T>): Promise<T> {
    const result = this.serial.then(action);
    this.serial = result.then(() => undefined, () => undefined);
    return result;
  }
}
