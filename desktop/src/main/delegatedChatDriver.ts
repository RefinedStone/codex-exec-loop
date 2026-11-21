import { homedir } from "node:os";
import { join, resolve } from "node:path";

import { acquireHostLease, ChatSessionService, type ChatObservation, type HostLease, type PagePort } from "akra-tunnel-core/chat";

import { CHAT_CONVERSATION } from "./delegatedChatPage";

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
    ...(value.chatUrl && CHAT_CONVERSATION.test(value.chatUrl) ? { conversationUrl: value.chatUrl } : {}),
  };
}

const WAITING = { delivery: "not_sent", phase: "unknown" } as const;
const STARTING = { delivery: "sending", phase: "unknown" } as const;
const MISSING = { delivery: "not_sent", phase: "unreachable", reason: "chat_job_missing" } as const;

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
  private readonly queued = new Map<string, string>();
  private active: { id: string; observation: ChatObservation | null } | undefined;
  private starting: Promise<void> | undefined;
  private closing = false;

  constructor(private readonly options: {
    home: string;
    pageFor(id: string): PagePort | Promise<PagePort>;
    manual(): boolean;
    closePage(): void;
  }) {}

  dispatch(method: "status" | "submit" | "observe" | "readResult" | "stop", id: string, instruction?: string): Promise<unknown> {
    return this.run(async () => {
      const service = await this.core();
      if (method === "status") return service
        ? { state: "ready" }
        : { state: "unavailable", reason: this.closing ? "chat_driver_closed" : "chat_host_busy" };
      if (!service) {
        if (method === "submit" || method === "observe") {
          return { delivery: "not_sent", phase: "unreachable", reason: "chat_host_busy" };
        }
        throw new Error("chat_host_busy");
      }
      if (method === "submit") {
        if (this.queued.has(id)) return WAITING;
        if (this.active?.id === id) {
          return this.active.observation ? this.view(this.active.observation) : WAITING;
        }
        if (this.active || this.queued.size) {
          if (this.queued.size >= 32) return { delivery: "not_sent", phase: "unreachable", reason: "chat_queue_full" };
          this.queued.set(id, instruction!);
          return WAITING;
        }
        if (this.options.manual()) return this.view(null);
        this.start(service, id, instruction!);
        return WAITING;
      }
      if (method === "readResult") {
        const text = await service.readResult(id);
        if (text === null) throw new Error("chat_result_unavailable");
        return { text };
      }
      if (method === "stop") {
        if (this.queued.delete(id)) return {};
        if (this.starting) throw new Error("chat_send_in_progress");
        if (this.options.manual()) throw new Error("chat_manual_action");
        const value = await service.stop(id);
        if (!value || (value.phase !== "stopped" && value.phase !== "complete")) throw new Error("chat_stop_unconfirmed");
        this.settle(id, value);
        return {};
      }
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
      const value = await service.observe(id);
      if (value) this.settle(id, value);
      return value ? this.view(value) : MISSING;
    });
  }

  close(): Promise<void> {
    this.closing = true;
    this.queued.clear();
    this.options.closePage();
    return this.run(async () => {
      await this.starting;
      await this.lease?.release();
    });
  }

  private start(service: ChatSessionService, id: string, instruction: string): void {
    this.active = { id, observation: null };
    this.starting = Promise.resolve().then(async () => {
      // The Gateway wire accepts fresh sessions only. Never reopen a persisted login's chat automatically.
      const value = await service.submit({ requestId: id, session: { mode: "new" }, instruction, maxResumes: 0 });
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
      return { delivery: value ? DELIVERY[value.delivery] : "not_sent", phase: "blocked", reason: "chat_manual_action" };
    }
    return value ? wireObservation(value) : MISSING;
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
