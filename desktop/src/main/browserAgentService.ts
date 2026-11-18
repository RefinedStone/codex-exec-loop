import type { BrowserAgentState, BrowserAutomationTool, BrowserRegistration, BrowserToolResult } from "../shared/browserAgent";
import { isBrowserToolName } from "../shared/browserAgent";
import type { BrowserAgentTransport, BrowserBrokerCall } from "./browserAgentTransport";
import type { BrowserViewChange } from "./browserPanel";

export interface BrowserAutomationPort {
  readonly attached: boolean;
  execute(tool: BrowserAutomationTool, args: unknown, signal: AbortSignal): Promise<BrowserToolResult>;
  dispose(): void;
}

export interface BrowserAgentHost {
  readonly command: string;
  readonly args: readonly string[];
  view(owner: number): { readonly generation: number; readonly visible: boolean; readonly profileId: string; readonly url: string; readonly title: string } | null;
  automation(owner: number, onPause: (reason: string) => void): BrowserAutomationPort;
  onViewChange(listener: (owner: number, change: BrowserViewChange) => void): () => void;
  notify(owner: number, state: BrowserAgentState): void;
}

type Broker = Pick<BrowserAgentTransport, "start" | "prepare" | "bind" | "revoke" | "answer" | "fail" | "close">;
interface BrokerCallbacks {
  onCall(call: BrowserBrokerCall): void;
  onCancel(requestId: string): void;
  onClosed(error: Error): void;
}

interface Registration {
  readonly id: string;
  readonly owner: number;
  thread: string | null;
  generation: number;
  phase: BrowserAgentState["phase"];
  reason: string | null;
  lastAction: string | null;
  engine: BrowserAutomationPort | null;
  pending: { readonly requestId: string; readonly controller: AbortController; resume?: () => void } | null;
}

const idleState = (): BrowserAgentState => ({ phase: "idle", threadId: null, lastAction: null, reason: null, attached: false });
const textResult = (text: string): BrowserToolResult => ({ content: [{ type: "text", text }] });

export class BrowserAgentService {
  private readonly registrations = new Map<string, Registration>();
  private readonly selected = new Map<number, string | null>();
  private readonly preparing = new Set<number>();
  private readonly ownerGeneration = new Map<number, number>();
  private readonly broker: Broker;
  private readonly unsubscribe: () => void;
  private closed = false;

  constructor(private readonly host: BrowserAgentHost, createBroker: (callbacks: BrokerCallbacks) => Broker) {
    this.broker = createBroker({
      onCall: (call) => { void this.call(call); },
      onCancel: (id) => {
        for (const registration of this.registrations.values()) {
          if (registration.pending?.requestId === id) registration.pending.controller.abort();
        }
      },
      onClosed: () => {
        for (const registration of this.registrations.values()) {
          this.suspend(registration, "bridge_stopped");
          registration.phase = "unavailable";
          this.report(registration.owner);
        }
      },
    });
    this.unsubscribe = host.onViewChange((owner, change) => {
      if (change === "shown") { this.report(owner); return; }
      for (const registration of this.registrations.values()) {
        if (registration.owner === owner) this.suspend(registration, change === "replaced" ? "profile_changed" : change);
      }
      if (change === "closed") void this.releaseOwner(owner);
    });
  }

  status(owner: number): BrowserAgentState {
    const registration = this.current(owner);
    return registration ? {
      phase: registration.phase, threadId: registration.thread, reason: registration.reason,
      lastAction: registration.lastAction, attached: registration.engine?.attached ?? false,
    } : idleState();
  }

  async prepare(owner: number): Promise<BrowserRegistration> {
    const view = this.host.view(owner);
    if (this.closed || !view?.visible) throw new Error("브라우저 패널을 먼저 여세요.");
    if (this.preparing.has(owner)) throw new Error("브라우저 연결을 준비하고 있습니다.");
    const generation = this.ownerGeneration.get(owner) ?? 0;
    this.preparing.add(owner);
    try {
      await this.broker.start();
      const prepared = await this.broker.prepare();
      if (this.closed || generation !== (this.ownerGeneration.get(owner) ?? 0) ||
          this.host.view(owner)?.generation !== view.generation || !this.host.view(owner)?.visible) {
        await this.broker.revoke(prepared.registrationId);
        throw new Error("브라우저 보기가 바뀌었습니다. 다시 연결하세요.");
      }
      this.registrations.set(prepared.registrationId, {
        id: prepared.registrationId, owner, thread: null, generation: view.generation,
        phase: "idle", reason: null, lastAction: null, engine: null, pending: null,
      });
      return { ...prepared, command: this.host.command, args: this.host.args };
    } finally {
      this.preparing.delete(owner);
    }
  }

  async bind(owner: number, id: string, thread: string): Promise<void> {
    const registration = this.owned(owner, id);
    if (!thread.trim() || thread.length > 256 || registration.thread !== null && registration.thread !== thread) throw new Error("잘못된 브라우저 대화입니다.");
    if ([...this.registrations.values()].some((entry) => entry.id !== id && entry.thread === thread)) throw new Error("다른 창이 이 브라우저 대화를 사용 중입니다.");
    await this.broker.bind(id, thread);
    if (this.registrations.get(id) !== registration) throw new Error("브라우저 연결이 취소됐습니다.");
    registration.thread = thread;
    this.resumeRegistration(registration);
  }

  async release(owner: number, id: string): Promise<void> {
    const registration = this.owned(owner, id);
    this.suspend(registration, "released");
    this.registrations.delete(id);
    await this.broker.revoke(id);
    this.report(owner);
  }

  async releaseOwner(owner: number): Promise<void> {
    this.ownerGeneration.set(owner, (this.ownerGeneration.get(owner) ?? 0) + 1);
    this.selected.delete(owner);
    for (const registration of [...this.registrations.values()]) {
      if (registration.owner !== owner) continue;
      try { await this.release(owner, registration.id); }
      catch (error) { console.warn("Browser registration cleanup failed", error instanceof Error ? error.message : "unknown error"); }
    }
  }

  setActiveThread(owner: number, thread: string | null): void {
    this.selected.set(owner, thread);
    for (const registration of this.registrations.values()) {
      if (registration.owner === owner && registration.thread !== thread) this.suspend(registration, "thread_changed");
    }
    this.report(owner);
  }

  pause(owner: number, reason = "user_control"): void {
    const registration = this.current(owner);
    if (registration) this.suspend(registration, reason);
  }

  resume(owner: number): void {
    const registration = this.current(owner);
    if (!registration) throw new Error("연결된 브라우저 대화가 없습니다.");
    this.resumeRegistration(registration);
    registration.pending?.resume?.();
  }

  async close(): Promise<void> {
    this.closed = true;
    this.unsubscribe();
    for (const registration of this.registrations.values()) this.suspend(registration, "closed");
    this.registrations.clear();
    await this.broker.close();
  }

  private current(owner: number): Registration | undefined {
    const thread = this.selected.get(owner);
    return [...this.registrations.values()].find((entry) => entry.owner === owner && entry.thread !== null && entry.thread === thread);
  }

  private owned(owner: number, id: string): Registration {
    const registration = this.registrations.get(id);
    if (!registration || registration.owner !== owner) throw new Error("이 창의 브라우저 연결이 아닙니다.");
    return registration;
  }

  private report(owner: number): void { this.host.notify(owner, this.status(owner)); }

  private suspend(registration: Registration, reason: string): void {
    if (registration.phase !== "unavailable") {
      registration.phase = "paused";
      registration.reason = reason;
    }
    registration.pending?.controller.abort();
    registration.engine?.dispose();
    registration.engine = null;
    this.report(registration.owner);
  }

  private resumeRegistration(registration: Registration): void {
    const view = this.host.view(registration.owner);
    if (!view?.visible || !registration.thread || this.selected.get(registration.owner) !== registration.thread) throw new Error("해당 대화와 브라우저 패널을 열어 주세요.");
    if (registration.phase === "unavailable") throw new Error("브라우저 연결이 종료됐습니다. 새 브라우저 대화를 시작하세요.");
    registration.engine?.dispose();
    registration.engine = this.host.automation(registration.owner, (reason) => this.suspend(registration, reason));
    registration.generation = view.generation;
    registration.phase = "active";
    registration.reason = null;
    this.report(registration.owner);
  }

  private async call(call: BrowserBrokerCall): Promise<void> {
    const registration = this.registrations.get(call.registrationId);
    const view = registration ? this.host.view(registration.owner) : null;
    if (!registration || registration.thread !== call.threadId || this.selected.get(registration.owner) !== call.threadId || !isBrowserToolName(call.tool)) {
      this.broker.fail(call.requestId, "not_authorized", "Browser ownership does not match this call."); return;
    }
    if (call.tool === "browser_status") {
      this.broker.answer(call.requestId, textResult(JSON.stringify({
        ...this.status(registration.owner), open: view?.visible ?? false,
        profileId: view?.profileId ?? null, url: view?.url ?? null, title: view?.title ?? null,
      }))); return;
    }
    if (registration.phase !== "active" || !view?.visible || registration.generation !== view.generation || !registration.engine) {
      this.broker.fail(call.requestId, "paused", "Open this thread's browser and resume agent control."); return;
    }
    if (registration.pending) { this.broker.fail(call.requestId, "busy", "Another browser action is running."); return; }
    const controller = new AbortController();
    const pending: NonNullable<Registration["pending"]> = { requestId: call.requestId, controller };
    registration.pending = pending;
    registration.lastAction = call.tool;
    const timeout = setTimeout(() => controller.abort(), call.tool === "browser_handoff" ? 600_000 : call.tool === "browser_wait" ? 55_000 : 30_000);
    let rejectAbort: () => void = () => undefined;
    const aborted = new Promise<never>((_, reject) => { rejectAbort = () => reject(new Error("Browser action cancelled or timed out.")); });
    controller.signal.addEventListener("abort", rejectAbort, { once: true });
    this.report(registration.owner);
    try {
      let work: Promise<BrowserToolResult>;
      if (call.tool === "browser_handoff") {
        const args = call.arguments;
        if (!args || typeof args !== "object" || !("reason" in args) || typeof args.reason !== "string" || !args.reason.trim() || args.reason.length > 240) throw new Error("A handoff reason of 1-240 characters is required.");
        registration.phase = "handoff";
        registration.reason = args.reason;
        registration.engine.dispose();
        registration.engine = null;
        work = new Promise((resolve) => { pending.resume = () => resolve(textResult("The user resumed browser control.")); });
        this.report(registration.owner);
      } else {
        work = registration.engine.execute(call.tool, call.arguments, controller.signal);
      }
      const result = await Promise.race([work, aborted]);
      if (!controller.signal.aborted && this.registrations.get(registration.id) === registration) this.broker.answer(call.requestId, result);
    } catch (error) {
      this.broker.fail(call.requestId, "action_failed", error instanceof Error ? error.message : "Browser action failed.");
    } finally {
      clearTimeout(timeout);
      controller.signal.removeEventListener("abort", rejectAbort);
      if (registration.pending === pending) registration.pending = null;
      if (registration.phase === "handoff") { registration.phase = "paused"; registration.reason = "handoff_ended"; }
      this.report(registration.owner);
    }
  }
}
