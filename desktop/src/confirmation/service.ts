import type { CommandRequest, CommandResponse } from "../shared/kernel-protocol/generated";
import type { IssueConfirmationResponse } from "../shared/issueConfirmation";
import { isRecord, type ConfirmationRpc } from "./connection";
import type { ConfirmationHelper } from "./transport";

import type { ConfirmationDisplay } from "../shared/issueConfirmationProcess";
import { SerialQueue } from "../shared/serialQueue";
export type { ConfirmationDisplay } from "../shared/issueConfirmationProcess";
export { SerialQueue as IssueConfirmationQueue } from "../shared/serialQueue";

export interface IssueConfirmationDependencies {
  rpc(onClosed: () => void): ConfirmationRpc;
  helper(onClosed: () => void): ConfirmationHelper;
  confirm(display: ConfirmationDisplay, signal: AbortSignal): Promise<boolean>;
  ownerAlive(): boolean;
  beforeSubmit(signal: AbortSignal): Promise<boolean>;
}

export class IssueConfirmationService {
  private generation = 0;
  private active: AbortController | undefined;

  constructor(
    private readonly dependencies: IssueConfirmationDependencies,
    private readonly queue: SerialQueue,
  ) {}

  confirmDecision(input: unknown): Promise<IssueConfirmationResponse> {
    const request = decisionRequest(input);
    return this.enqueue(request.workspace, request.command);
  }

  confirmMode(input: unknown): Promise<IssueConfirmationResponse> {
    const request = modeRequest(input);
    return this.enqueue(request.workspace, request.command);
  }

  invalidate(): void {
    this.generation++;
    this.active?.abort();
  }

  private enqueue(workspace: string, request: CommandRequest): Promise<IssueConfirmationResponse> {
    const generation = this.generation;
    return this.queue.run(async () => {
      if (generation !== this.generation || !this.dependencies.ownerAlive()) return { cancelled: true };
      const controller = new AbortController();
      this.active = controller;
      const current = () => !controller.signal.aborted && generation === this.generation && this.dependencies.ownerAlive();
      let rpc: ConfirmationRpc | undefined;
      let helper: ConfirmationHelper | undefined;
      let submitted = false;
      const close = () => {
        rpc?.close();
        helper?.close();
      };
      controller.signal.addEventListener("abort", close, { once: true });
      try {
        rpc = this.dependencies.rpc(() => controller.abort());
        helper = this.dependencies.helper(() => controller.abort());
        await rpc.request("akra/kernel/attach", { workspace, client_name: "akra-desktop-confirmation" });
        if (!current()) return { cancelled: true };
        const registration = await helper.prepare();
        if (!current()) return { cancelled: true };
        const binding = await rpc.request("akra/kernel/confirmation/register", registration);
        if (!current()) return { cancelled: true };
        if (!isRecord(binding) || !isRecord(binding.binding)) throw failure();
        const registeredBinding = binding.binding;
        if (!BINDING_FIELDS.every((field) => boundedText(registeredBinding[field], 32_768)) ||
          registeredBinding.registration_id !== registration.registration_id) throw failure();
        const prepared = await rpc.request("akra/kernel/confirmation/prepare", { request });
        if (!current()) return { cancelled: true };
        const { intent, display } = preparedConfirmation(prepared, registeredBinding, request.idempotency_key);
        if (containsSecret(display, [registration.registration_id, registration.session_file])) throw failure();
        const approved = await this.dependencies.confirm(display, controller.signal);
        if (!current() || !approved) return { cancelled: true };
        const proof = await helper.approve(registration.registration_id, intent);
        if (!current()) return { cancelled: true };
        if (!(await this.dependencies.beforeSubmit(controller.signal)) || !current()) return { cancelled: true };
        // 호출이 시작되면 응답 유실만으로 저장 여부를 알 수 없다. 이후 중단은 취소로 보고하지 않는다.
        submitted = true;
        const result = await rpc.request("akra/kernel/command_confirmed", {
          request,
          confirmation: { registration_id: registration.registration_id, proof_id: proof.proof_id, intent },
        });
        if (!current()) return { cancelled: false, outcome: "unknown" };
        // 확인 증거와 helper 주소는 성공/오류 응답에도 renderer로 되돌려 보내지 않는다.
        if (containsSecret(result, [proof.proof_id, registration.registration_id, registration.session_file])) throw failure();
        return { cancelled: false, response: commandResponse(result) };
      } catch {
        if (submitted) return { cancelled: false, outcome: "unknown" };
        if (!current()) return { cancelled: true };
        throw failure();
      } finally {
        controller.signal.removeEventListener("abort", close);
        close();
        if (this.active === controller) this.active = undefined;
      }
    });
  }
}

const BINDING_FIELDS = ["registration_id", "connection_id", "attachment_id", "client_id", "workspace"] as const;

function preparedConfirmation(value: unknown, binding: Record<string, unknown>, key: string | null | undefined): { intent: unknown; display: ConfirmationDisplay } {
  if (!isRecord(value) || !isRecord(value.intent) || !isRecord(value.intent.binding) || !isRecord(value.display) || value.intent.key !== key) throw failure();
  const intentBinding = value.intent.binding;
  if (!BINDING_FIELDS.every((field) => intentBinding[field] === binding[field])) throw failure();
  const display = value.display;
  if (!boundedText(display.message, 8_192) || !boundedText(display.detail, 32_768, true) || !boundedText(display.confirm_label, 256)) throw failure();
  return { intent: value.intent, display: { message: display.message, detail: display.detail, confirm_label: display.confirm_label } };
}

function commandResponse(value: unknown): CommandResponse {
  if (!isRecord(value) || typeof value.processed !== "boolean") throw failure();
  // 서버가 반환한 기존 CommandResponse 필드만 전달한다.
  return {
    processed: value.processed,
    ...(value.failure !== undefined ? { failure: value.failure } : {}),
    ...(value.operation !== undefined ? { operation: value.operation } : {}),
    ...(value.rejection !== undefined ? { rejection: value.rejection } : {}),
    ...(value.result !== undefined ? { result: value.result } : {}),
  } as CommandResponse;
}

function decisionRequest(value: unknown): { workspace: string; command: CommandRequest } {
  if (!isRecord(value) || !onlyFields(value, ["workspace", "decisionId", "option", "note", "key"]) ||
    !workspace(value.workspace) || !token(value.decisionId, 128) || !token(value.option, 128) || !token(value.key, 128) ||
    (value.note !== undefined && value.note !== null && !boundedText(value.note, 4_096, true))) throw failure();
  return {
    workspace: value.workspace,
    command: {
      command: { type: "answer_issue_decision", decision_id: value.decisionId, option: value.option, ...(value.note != null ? { note: value.note } : {}) },
      idempotency_key: value.key,
    },
  };
}

function modeRequest(value: unknown): { workspace: string; command: CommandRequest } {
  if (!isRecord(value) || !onlyFields(value, ["workspace", "mode", "key"]) || !workspace(value.workspace) ||
    !token(value.key, 128) || (value.mode !== "off" && value.mode !== "observe")) throw failure();
  return { workspace: value.workspace, command: { command: { type: "set_issue_automation_mode", mode: value.mode }, idempotency_key: value.key } };
}

function workspace(value: unknown): value is string {
  return boundedText(value, 32_768) && !value.includes("\0");
}

function token(value: unknown, maxBytes: number): value is string {
  return boundedText(value, maxBytes) && !/[\s\p{Cc}]/u.test(value);
}

function boundedText(value: unknown, maxBytes: number, empty = false): value is string {
  return typeof value === "string" && (empty || value.length > 0) && Buffer.byteLength(value, "utf8") <= maxBytes;
}

function onlyFields(value: Record<string, unknown>, fields: readonly string[]): boolean {
  return Object.keys(value).every((field) => fields.includes(field));
}

function containsSecret(value: unknown, secrets: readonly string[]): boolean {
  const serialized = JSON.stringify(value);
  return typeof serialized !== "string" || secrets.some((secret) => serialized.includes(JSON.stringify(secret).slice(1, -1)));
}

function failure(): Error {
  return new Error("확인 요청을 완료하지 못했습니다. 상태를 다시 읽고 시도해 주세요.");
}
