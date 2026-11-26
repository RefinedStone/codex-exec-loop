// 서버 요청을 화면용 값으로 바꾼다 (P3 §6.4, #2836). 명령 실행·파일 변경 승인, 추가 권한 승인, 도구의 사용자 입력,
// MCP elicitation 을 다룬다. 명령·파일 승인은 응답 타입이 정한 값 중 세 가지만 보여 준다.
import type { McpServerElicitationRequestResponse } from "../../shared/protocol/generated/v2/McpServerElicitationRequestResponse";
import type { PermissionsRequestApprovalResponse } from "../../shared/protocol/generated/v2/PermissionsRequestApprovalResponse";
import type { RequestPermissionProfile } from "../../shared/protocol/generated/v2/RequestPermissionProfile";
import type { ToolRequestUserInputQuestion } from "../../shared/protocol/generated/v2/ToolRequestUserInputQuestion";
import type { ToolRequestUserInputResponse } from "../../shared/protocol/generated/v2/ToolRequestUserInputResponse";
import type { JsonRpcId } from "../rpc/jsonRpcClient";

/** writeStdin 은 이미 실행 중인 프로세스(터미널)에 입력을 보내는 승인이다. 새 명령 실행과 구분해 보여 준다. */
export type ApprovalKind = "command" | "writeStdin" | "fileChange" | "permissions" | "userInput" | "elicitation";
export type ApprovalDecision = "accept" | "acceptForSession" | "decline";

/** 허용·거부 버튼으로 답하는 요청이다. 나머지(사용자 입력, elicitation)는 자기 양식으로 답한다. */
export function answersWithDecision(kind: ApprovalKind): boolean {
  return kind === "command" || kind === "writeStdin" || kind === "fileChange" || kind === "permissions";
}

/** MCP elicitation 양식의 칸 하나이다. 스키마가 정한 기본 타입만 그린다. */
export interface ElicitationField {
  name: string;
  label: string;
  description: string | null;
  required: boolean;
  input: "text" | "number" | "integer" | "boolean" | "select" | "multiSelect";
  options: Array<{ value: string; label: string }>;
  defaultValue: unknown;
}

export interface ElicitationRequest {
  serverName: string;
  message: string;
  /** url 방식이면 사용자가 열어 볼 주소이다. */
  url: string | null;
  /** 양식 방식이면 칸 목록이다. 그릴 수 없는 스키마이면 null 이고, 수락 없이 거절·취소만 할 수 있다. */
  fields: ElicitationField[] | null;
}

export const APPROVAL_DECISIONS: readonly ApprovalDecision[] = ["accept", "acceptForSession", "decline"];

export interface PendingApproval {
  requestId: JsonRpcId;
  kind: ApprovalKind;
  threadId: string;
  turnId: string;
  itemId: string;
  command: string | null;
  cwd: string | null;
  reason: string | null;
  permissions?: RequestPermissionProfile;
  questions?: ToolRequestUserInputQuestion[];
  elicitation?: ElicitationRequest;
}

const APPROVAL_METHODS: Record<string, ApprovalKind> = {
  "item/commandExecution/requestApproval": "command",
  "item/fileChange/requestApproval": "fileChange",
  "item/permissions/requestApproval": "permissions",
  "item/tool/requestUserInput": "userInput",
  "mcpServer/elicitation/request": "elicitation",
};

function optionalString(value: unknown): string | null {
  return typeof value === "string" && value.length > 0 ? value : null;
}

function record(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : null;
}

function stringList(value: unknown): string[] | null {
  return Array.isArray(value) && value.every((item) => typeof item === "string") ? value as string[] : null;
}

function constOptions(value: unknown): Array<{ value: string; label: string }> | null {
  if (!Array.isArray(value)) return null;
  return value.flatMap((item) => {
    const option = record(item);
    return option && typeof option.const === "string" ? [{ value: option.const, label: typeof option.title === "string" ? option.title : option.const }] : [];
  });
}

function elicitationField(name: string, schema: unknown, required: boolean): ElicitationField | null {
  const fields = record(schema);
  if (!fields) return null;
  const label = typeof fields.title === "string" && fields.title ? fields.title : name;
  const base = { name, label, description: optionalString(fields.description), required, defaultValue: fields.default ?? null };
  if (fields.type === "array") {
    const items = record(fields.items);
    const values = stringList(items?.enum);
    const options = values ? values.map((value) => ({ value, label: value })) : constOptions(items?.anyOf ?? items?.oneOf);
    return options && options.length > 0 ? { ...base, input: "multiSelect", options } : null;
  }
  if (fields.type === "string") {
    const values = stringList(fields.enum);
    if (values) {
      const names = stringList(fields.enumNames);
      return { ...base, input: "select", options: values.map((value, index) => ({ value, label: names?.[index] ?? value })) };
    }
    const options = constOptions(fields.oneOf);
    return options ? { ...base, input: "select", options } : { ...base, input: "text", options: [] };
  }
  if (fields.type === "number" || fields.type === "integer" || fields.type === "boolean") {
    return { ...base, input: fields.type, options: [] };
  }
  return null;
}

/** 양식 스키마를 칸 목록으로 바꾼다. 모르는 칸이 하나라도 있으면 null 이다 (반쯤 채운 응답을 보내지 않는다). */
export function elicitationFields(schema: unknown): ElicitationField[] | null {
  const fields = record(schema);
  const properties = record(fields?.properties);
  if (!fields || fields.type !== "object" || !properties) return null;
  const required = new Set(stringList(fields.required) ?? []);
  const result: ElicitationField[] = [];
  for (const [name, property] of Object.entries(properties)) {
    const field = elicitationField(name, property, required.has(name));
    if (!field) return null;
    result.push(field);
  }
  return result;
}

function toElicitation(fields: Record<string, unknown>): ElicitationRequest | null {
  if (typeof fields.serverName !== "string" || typeof fields.message !== "string") return null;
  const url = fields.mode === "url" && typeof fields.url === "string" ? fields.url : null;
  return {
    serverName: fields.serverName,
    message: fields.message,
    url,
    fields: url === null ? elicitationFields(fields.requestedSchema) : null,
  };
}

/** 다루는 서버 요청이면 화면용 값을, 아니면 null 을 돌려준다. */
export function toPendingApproval(method: string, params: unknown, requestId: JsonRpcId): PendingApproval | null {
  const kind = APPROVAL_METHODS[method];
  const fields = record(params);
  if (!kind || !fields || typeof fields.threadId !== "string") {
    return null;
  }
  const approval: PendingApproval = {
    requestId,
    kind: kind === "command" && fields.kind === "writeStdin" ? "writeStdin" : kind,
    threadId: fields.threadId,
    turnId: String(fields.turnId ?? ""),
    itemId: String(fields.itemId ?? ""),
    command: optionalString(fields.command),
    cwd: optionalString(fields.cwd),
    reason: optionalString(fields.reason),
  };
  if (kind === "permissions") {
    const permissions = record(fields.permissions);
    return permissions ? { ...approval, permissions: permissions as RequestPermissionProfile } : null;
  }
  if (kind === "userInput") {
    const questions = Array.isArray(fields.questions) ? fields.questions : null;
    return questions && questions.every((question) => typeof record(question)?.id === "string")
      ? { ...approval, questions: questions as ToolRequestUserInputQuestion[] }
      : null;
  }
  if (kind === "elicitation") {
    const elicitation = toElicitation(fields);
    return elicitation ? { ...approval, elicitation } : null;
  }
  return approval;
}

/** 추가 권한 승인의 응답이다. 허용은 요청한 권한을, 거부는 빈 권한을 준다. */
export function permissionsResponse(requested: RequestPermissionProfile, decision: ApprovalDecision): PermissionsRequestApprovalResponse {
  if (decision === "decline") return { permissions: {}, scope: "turn" };
  return {
    permissions: {
      ...(requested.network ? { network: requested.network } : {}),
      ...(requested.fileSystem ? { fileSystem: requested.fileSystem } : {}),
    },
    scope: decision === "acceptForSession" ? "session" : "turn",
  };
}

/** 사용자 입력의 응답이다. 답하지 않은 질문은 넣지 않는다. */
export function userInputResponse(answers: Record<string, string>): ToolRequestUserInputResponse {
  return {
    answers: Object.fromEntries(
      Object.entries(answers).filter(([, answer]) => answer.trim().length > 0).map(([id, answer]) => [id, { answers: [answer] }]),
    ),
  };
}

export function elicitationResponse(
  action: McpServerElicitationRequestResponse["action"],
  content: Record<string, unknown> | null = null,
): McpServerElicitationRequestResponse {
  return { action, content: action === "accept" ? content as McpServerElicitationRequestResponse["content"] : null, _meta: null };
}

export function approvalKey(requestId: JsonRpcId): string {
  return String(requestId);
}
