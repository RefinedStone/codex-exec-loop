import type { CommandResponse } from "../shared/kernel-protocol/generated";
import type { IssueConfirmationResponse } from "../shared/issueConfirmation";
import type { ConfirmationControl, ConfirmationDisplay, ConfirmationKind } from "../shared/issueConfirmationProcess";
import { SerialQueue } from "../shared/serialQueue";

export { SerialQueue as IssueConfirmationQueue } from "../shared/serialQueue";
export type { ConfirmationDisplay } from "../shared/issueConfirmationProcess";

export interface ConfirmationProcess {
  postMessage(message: ConfirmationControl): void;
  on(event: "message", listener: (message: unknown) => void): this;
  once(event: "exit" | "error", listener: () => void): this;
  kill(): boolean;
}

export interface IssueConfirmationDependencies {
  process(): ConfirmationProcess;
  endpoint(): string;
  executable(): string;
  confirm(display: ConfirmationDisplay, signal: AbortSignal): Promise<boolean>;
  ownerAlive(): boolean;
}

/** main은 창 수명과 native 확인만 소유한다. 커널 프로토콜은 utility process에서 처리한다. */
export class IssueConfirmationService {
  private generation = 0;
  private active: AbortController | undefined;

  constructor(
    private readonly dependencies: IssueConfirmationDependencies,
    private readonly queue: SerialQueue,
  ) {}

  confirmDecision(input: unknown): Promise<IssueConfirmationResponse> {
    return this.enqueue("decision", input);
  }

  confirmMode(input: unknown): Promise<IssueConfirmationResponse> {
    return this.enqueue("mode", input);
  }

  confirmAutonomy(input: unknown): Promise<IssueConfirmationResponse> {
    return this.enqueue("autonomy", input);
  }

  confirmBindingResume(input: unknown): Promise<IssueConfirmationResponse> {
    return this.enqueue("binding_resume", input);
  }

  confirmCommentRequest(input: unknown): Promise<IssueConfirmationResponse> {
    return this.enqueue("comment_request", input);
  }

  confirmCriterionWaiver(input: unknown): Promise<IssueConfirmationResponse> {
    return this.enqueue("criterion_waiver", input);
  }

  invalidate(): void {
    this.generation++;
    this.active?.abort();
  }

  private enqueue(kind: ConfirmationKind, input: unknown): Promise<IssueConfirmationResponse> {
    const generation = this.generation;
    return this.queue.run(() => {
      if (generation !== this.generation || !this.dependencies.ownerAlive()) return Promise.resolve({ cancelled: true });
      return this.run(kind, input, generation);
    });
  }

  private run(kind: ConfirmationKind, input: unknown, generation: number): Promise<IssueConfirmationResponse> {
    return new Promise((resolve, reject) => {
      const controller = new AbortController();
      this.active = controller;
      const current = () => !controller.signal.aborted && generation === this.generation && this.dependencies.ownerAlive();
      let child: ConfirmationProcess | undefined;
      let settled = false;
      let childExited = false;
      let ready = false;
      let prompted = false;
      let approved = false;
      let submissionAllowed = false;
      let cancellationTimer: NodeJS.Timeout | undefined;
      const readyTimer = setTimeout(() => finish(), 10_000);

      const stop = () => {
        if (!child || childExited) return;
        const ownedChild = child;
        try { ownedChild.postMessage({ type: "dispose" }); } catch { /* 종료 중인 child에는 재전송하지 않는다. */ }
        const timer = setTimeout(() => ownedChild.kill(), 6_000);
        timer.unref();
        ownedChild.once("exit", () => clearTimeout(timer));
      };
      const finish = (response?: IssueConfirmationResponse) => {
        if (settled) return;
        settled = true;
        clearTimeout(readyTimer);
        clearTimeout(cancellationTimer);
        controller.signal.removeEventListener("abort", cancel);
        const wasCurrent = current();
        controller.abort();
        stop();
        if (this.active === controller) this.active = undefined;
        if (submissionAllowed && (!response || !wasCurrent || response.cancelled)) {
          resolve({ cancelled: false, outcome: "unknown" });
        } else if (!wasCurrent) {
          resolve({ cancelled: true });
        } else if (response) {
          resolve(response);
        } else {
          reject(failure());
        }
      };
      const send = (message: ConfirmationControl) => {
        if (settled || !child) return;
        try { child.postMessage(message); } catch { finish(); }
      };
      const cancel = () => {
        if (settled) return;
        send({ type: "cancel" });
        // child가 멈췄어도 다른 창의 확인을 계속 막아 두지 않는다.
        cancellationTimer = setTimeout(() => finish(), 5_000);
      };
      controller.signal.addEventListener("abort", cancel, { once: true });
      const handleMessage = (message: unknown) => {
        if (settled) return;
        if (!isRecord(message) || typeof message.type !== "string") return finish();
        if (message.type === "ready" && !ready && onlyFields(message, ["type"])) {
          if (!current()) return finish();
          ready = true;
          clearTimeout(readyTimer);
          send({ type: "begin", kind, input, endpoint: this.dependencies.endpoint(), executable: this.dependencies.executable() });
        } else if (message.type === "prompt" && ready && !prompted && onlyFields(message, ["type", "display"])) {
          if (!current()) return finish();
          const display = confirmationDisplay(message.display);
          if (!display) return finish();
          prompted = true;
          void this.dependencies.confirm(display, controller.signal).then((answer) => {
            if (!current() || settled) return;
            approved = answer === true;
            send({ type: "answer", approved });
          }, () => finish());
        } else if (message.type === "will-submit" && approved && !submissionAllowed && onlyFields(message, ["type"])) {
          if (!current()) return finish();
          // child는 이 응답을 받은 뒤에만 제출한다. 응답 유실/종료 시 취소로 잘못 보고하지 않는다.
          submissionAllowed = true;
          send({ type: "submit", allowed: true });
        } else if (message.type === "result" && ready && onlyFields(message, ["type", "response"])) {
          const response = confirmationResponse(message.response);
          if (!response || (!response.cancelled && !submissionAllowed)) return finish();
          finish(response);
        } else if (message.type === "failed" && onlyFields(message, ["type"])) {
          finish();
        } else {
          finish();
        }
      };
      const receive = (message: unknown) => {
        try { handleMessage(message); } catch { finish(); }
      };
      try {
        child = this.dependencies.process();
        child.on("message", receive);
        child.once("exit", () => { childExited = true; finish(); });
        child.once("error", () => finish());
      } catch {
        finish();
      }
    });
  }
}

function confirmationDisplay(value: unknown): ConfirmationDisplay | undefined {
  if (!isRecord(value) || !onlyFields(value, ["message", "detail", "confirm_label"]) ||
    !text(value.message, 8_192) || !text(value.detail, 32_768, true) || !text(value.confirm_label, 256)) return;
  return { message: value.message, detail: value.detail, confirm_label: value.confirm_label };
}

function confirmationResponse(value: unknown): IssueConfirmationResponse | undefined {
  if (!isRecord(value)) return;
  if (value.cancelled === true && onlyFields(value, ["cancelled"])) return { cancelled: true };
  if (value.cancelled !== false) return;
  if (value.outcome === "unknown" && onlyFields(value, ["cancelled", "outcome"])) return { cancelled: false, outcome: "unknown" };
  if (onlyFields(value, ["cancelled", "response"]) && isRecord(value.response) && typeof value.response.processed === "boolean" &&
    onlyFields(value.response, ["processed", "failure", "operation", "rejection", "result"])) {
    return { cancelled: false, response: value.response as CommandResponse };
  }
  return;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
function onlyFields(value: Record<string, unknown>, fields: readonly string[]): boolean {
  return Object.keys(value).every((field) => fields.includes(field));
}
function text(value: unknown, max: number, empty = false): value is string {
  return typeof value === "string" && (empty || value.length > 0) && Buffer.byteLength(value, "utf8") <= max;
}
function failure(): Error {
  return new Error("확인 요청을 완료하지 못했습니다. 상태를 다시 읽고 시도해 주세요.");
}
