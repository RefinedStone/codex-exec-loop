import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { isRecord } from "./issueConfirmationConnection";

const MAX_FRAME_BYTES = 64 * 1024;
type Pending = { resolve(value: unknown): void; reject(error: Error): void; timer: NodeJS.Timeout };

export interface ConfirmationHelper {
  prepare(): Promise<{ registration_id: string; session_file: string }>;
  approve(registrationId: string, intent: unknown): Promise<{ proof_id: string }>;
  close(): void;
}

/** 증거는 이 프로세스의 stdin/stdout에서만 이동한다. argv, 환경 변수, 로그에 넣지 않는다. */
export class IssueConfirmationTransport implements ConfirmationHelper {
  private child: ChildProcessWithoutNullStreams | undefined;
  private starting: Promise<void> | undefined;
  private resolveReady: (() => void) | undefined;
  private rejectReady: ((error: Error) => void) | undefined;
  private readyTimer: NodeJS.Timeout | undefined;
  private closed = false;
  private ready = false;
  private sequence = 0;
  private bytes: Buffer<ArrayBufferLike> = Buffer.alloc(0);
  private readonly pending = new Map<string, Pending>();

  constructor(private readonly command: string, private readonly onClosed: () => void) {}

  async prepare(): Promise<{ registration_id: string; session_file: string }> {
    const result = await this.control({ command: "prepare" });
    if (!isRecord(result) || !this.text(result.registration_id) || !this.text(result.session_file)) {
      this.finish(true);
      throw this.failure();
    }
    return { registration_id: result.registration_id, session_file: result.session_file };
  }

  async approve(registrationId: string, intent: unknown): Promise<{ proof_id: string }> {
    const result = await this.control({ command: "approve", registration_id: registrationId, intent });
    if (!isRecord(result) || !this.text(result.proof_id)) {
      this.finish(true);
      throw this.failure();
    }
    return { proof_id: result.proof_id };
  }

  close(): void {
    this.finish(false);
  }

  private start(): Promise<void> {
    if (this.closed) return Promise.reject(this.failure());
    if (this.starting) return this.starting;
    this.starting = new Promise((resolve, reject) => {
      this.resolveReady = resolve;
      this.rejectReady = reject;
    });
    this.readyTimer = setTimeout(() => this.finish(true), 10_000);
    try {
      const child = spawn(this.command, ["desktop-confirmation-bridge"], {
        detached: false,
        windowsHide: true,
        stdio: "pipe",
      });
      this.child = child;
      child.stdout.on("data", (chunk: Buffer) => this.receive(chunk));
      // helper의 오류 원문에도 증거가 포함될 수 있으므로 로그로 중계하지 않는다.
      child.stderr.resume();
      child.on("error", () => this.finish(true));
      child.on("exit", () => this.finish(true));
      child.stdout.on("end", () => this.finish(true));
      child.stdin.on("error", () => this.finish(true));
    } catch {
      this.finish(true);
    }
    return this.starting;
  }

  private async control(command: Record<string, unknown>): Promise<unknown> {
    await this.start();
    if (this.closed) throw this.failure();
    const id = `confirmation-${++this.sequence}`;
    const line = JSON.stringify({ id, ...command }) + "\n";
    if (Buffer.byteLength(line, "utf8") > MAX_FRAME_BYTES) throw this.failure();
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => this.finish(true), 10_000);
      this.pending.set(id, { resolve, reject, timer });
      try {
        if (!this.child || this.child.stdin.writableLength > MAX_FRAME_BYTES) this.finish(true);
        else this.child.stdin.write(line);
      } catch {
        this.finish(true);
      }
    });
  }

  private receive(chunk: Buffer): void {
    if (this.closed) return;
    this.bytes = Buffer.concat([this.bytes, chunk]);
    try {
      for (;;) {
        const end = this.bytes.indexOf(10);
        if (end < 0) {
          if (this.bytes.length > MAX_FRAME_BYTES) throw this.failure();
          return;
        }
        if (end + 1 > MAX_FRAME_BYTES) throw this.failure();
        const value: unknown = JSON.parse(this.bytes.subarray(0, end).toString("utf8"));
        this.bytes = this.bytes.subarray(end + 1);
        if (!isRecord(value)) throw this.failure();
        if (!this.ready) {
          if (value.type !== "ready") throw this.failure();
          this.ready = true;
          clearTimeout(this.readyTimer);
          this.resolveReady?.();
          this.resolveReady = undefined;
          this.rejectReady = undefined;
          continue;
        }
        if (typeof value.id !== "string") throw this.failure();
        const pending = this.pending.get(value.id);
        if (!pending) throw this.failure();
        this.pending.delete(value.id);
        clearTimeout(pending.timer);
        if ("error" in value || !("result" in value)) pending.reject(this.failure());
        else pending.resolve(value.result);
      }
    } catch {
      this.finish(true);
    }
  }

  private finish(notify: boolean): void {
    if (this.closed) return;
    this.closed = true;
    clearTimeout(this.readyTimer);
    this.bytes = Buffer.alloc(0);
    const error = this.failure();
    this.rejectReady?.(error);
    this.resolveReady = undefined;
    this.rejectReady = undefined;
    for (const pending of this.pending.values()) {
      clearTimeout(pending.timer);
      pending.reject(error);
    }
    this.pending.clear();
    const child = this.child;
    if (child && child.exitCode === null && child.signalCode === null) {
      // EOF는 helper가 가진 등록과 아직 쓰지 않은 모든 증거를 폐기한다.
      child.stdin.end();
      const timer = setTimeout(() => child.kill(), 5_000);
      timer.unref();
      child.once("exit", () => clearTimeout(timer));
    }
    if (notify) this.onClosed();
  }

  private text(value: unknown): value is string {
    return typeof value === "string" && value.length > 0 && Buffer.byteLength(value, "utf8") < MAX_FRAME_BYTES;
  }

  private failure(): Error {
    return new Error("확인 증거를 준비할 수 없습니다.");
  }
}
