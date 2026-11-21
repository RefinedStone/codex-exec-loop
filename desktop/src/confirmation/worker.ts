import type { ConfirmationControl, ConfirmationDisplay, ConfirmationEvent } from "../shared/issueConfirmationProcess";
import { IssueConfirmationConnection, isRecord } from "./connection";
import { IssueConfirmationTransport } from "./transport";
import { IssueConfirmationQueue, IssueConfirmationService } from "./service";

type ReplyKind = "answer" | "submit";
type PendingReply = { kind: ReplyKind; complete(answer: boolean): void };

/** 하나의 private parent channel에서 한 행동만 처리한다. 요청/증거를 다음 프로세스로 재생하지 않는다. */
export class IssueConfirmationWorker {
  private service: IssueConfirmationService | undefined;
  private pending: PendingReply | undefined;
  private readonly helpers = new Set<IssueConfirmationTransport>();
  private begun = false;
  private cancelled = false;
  private disposed = false;

  constructor(private readonly send: (event: ConfirmationEvent) => void) {}

  receive(value: unknown): void {
    if (this.disposed) return;
    if (!isRecord(value) || typeof value.type !== "string") return this.fail();
    if (value.type === "cancel" && onlyFields(value, ["type"])) {
      this.cancel();
    } else if (value.type === "begin" && !this.begun && !this.cancelled &&
      onlyFields(value, ["type", "kind", "input", "endpoint", "executable"]) &&
      (value.kind === "decision" || value.kind === "mode") && pathText(value.endpoint) && pathText(value.executable)) {
      this.begun = true;
      void this.run(value as Extract<ConfirmationControl, { type: "begin" }>);
    } else if (value.type === "answer" && onlyFields(value, ["type", "approved"]) &&
      this.pending?.kind === "answer" && typeof value.approved === "boolean") {
      this.pending.complete(value.approved);
    } else if (value.type === "submit" && onlyFields(value, ["type", "allowed"]) &&
      this.pending?.kind === "submit" && typeof value.allowed === "boolean") {
      this.pending.complete(value.allowed);
    } else {
      this.fail();
    }
  }

  async dispose(): Promise<void> {
    this.disposed = true;
    this.cancel();
    await Promise.all([...this.helpers].map((helper) => { helper.close(); return helper.waitClosed(); }));
  }

  private cancel(): void {
    this.cancelled = true;
    this.service?.invalidate();
    this.pending?.complete(false);
  }

  private fail(): void {
    this.cancel();
    if (!this.disposed) this.send({ type: "failed" });
  }

  private async run(message: Extract<ConfirmationControl, { type: "begin" }>): Promise<void> {
    this.service = new IssueConfirmationService({
      rpc: (onClosed) => new IssueConfirmationConnection(message.endpoint, onClosed),
      helper: (onClosed) => {
        const helper = new IssueConfirmationTransport(message.executable, onClosed);
        this.helpers.add(helper);
        return helper;
      },
      ownerAlive: () => !this.cancelled && !this.disposed,
      confirm: (display, signal) => this.reply("answer", { type: "prompt", display }, signal),
      beforeSubmit: (signal) => this.reply("submit", { type: "will-submit" }, signal),
    }, new IssueConfirmationQueue());
    try {
      const response = await (message.kind === "decision"
        ? this.service.confirmDecision(message.input)
        : this.service.confirmMode(message.input));
      if (!this.disposed) this.send({ type: "result", response });
    } catch {
      if (!this.disposed) this.send({ type: "failed" });
    }
  }

  private reply(kind: ReplyKind, event: { type: "prompt"; display: ConfirmationDisplay } | { type: "will-submit" }, signal: AbortSignal): Promise<boolean> {
    if (this.cancelled || this.disposed || signal.aborted || this.pending) return Promise.resolve(false);
    return new Promise((resolve) => {
      const complete = (answer: boolean) => {
        if (this.pending?.complete !== complete) return;
        this.pending = undefined;
        signal.removeEventListener("abort", abort);
        resolve(answer && !this.cancelled && !this.disposed && !signal.aborted);
      };
      const abort = () => complete(false);
      this.pending = { kind, complete };
      signal.addEventListener("abort", abort, { once: true });
      this.send(event);
    });
  }
}

function onlyFields(value: Record<string, unknown>, fields: readonly string[]): boolean {
  return Object.keys(value).every((field) => fields.includes(field));
}
function pathText(value: unknown): value is string {
  return typeof value === "string" && value.length > 0 && Buffer.byteLength(value, "utf8") <= 32_768 && !value.includes("\0");
}
