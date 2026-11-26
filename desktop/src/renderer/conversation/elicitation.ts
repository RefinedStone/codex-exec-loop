// MCP form의 지원 범위와 제출 검증이다. 렌더링과 검증이 같은 제약을 사용한다.
import type { McpElicitationStringFormat } from "../../shared/protocol/generated/v2/McpElicitationStringFormat";

export interface ElicitationField {
  name: string;
  label: string;
  description: string | null;
  required: boolean;
  input: "text" | "number" | "integer" | "boolean" | "select" | "multiSelect";
  options: Array<{ value: string; label: string }>;
  defaultValue: unknown;
  minimum?: number;
  maximum?: number;
  minLength?: number;
  maxLength?: number;
  minItems?: number;
  maxItems?: number;
  format?: McpElicitationStringFormat;
}

const COMMON_KEYS = ["type", "title", "description", "default"];
const FORMATS: readonly string[] = ["email", "uri", "date", "date-time"];

function record(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : null;
}

function stringList(value: unknown): string[] | null {
  return Array.isArray(value) && value.every((item) => typeof item === "string") ? value as string[] : null;
}

function knownKeys(value: Record<string, unknown>, keys: readonly string[]): boolean {
  return Object.keys(value).every((key) => keys.includes(key));
}

function constOptions(value: unknown): ElicitationField["options"] | null {
  if (!Array.isArray(value)) return null;
  const options: ElicitationField["options"] = [];
  for (const item of value) {
    const option = record(item);
    if (!option || !knownKeys(option, ["const", "title"]) || typeof option.const !== "string") return null;
    options.push({ value: option.const, label: typeof option.title === "string" ? option.title : option.const });
  }
  return options;
}

function limits(schema: Record<string, unknown>, lower: string, upper: string, count = false): Record<string, number> | null {
  const result: Record<string, number> = {};
  for (const key of [lower, upper]) {
    if (schema[key] === undefined || schema[key] === null) continue;
    const value = typeof schema[key] === "bigint" ? Number(schema[key]) : schema[key];
    if (typeof value !== "number" || !Number.isFinite(value) || (count && (!Number.isSafeInteger(value) || value < 0))) return null;
    result[key] = value;
  }
  return result[lower] !== undefined && result[upper] !== undefined && result[lower] > result[upper] ? null : result;
}

function elicitationField(name: string, schema: unknown, required: boolean): ElicitationField | null {
  const fields = record(schema);
  if (!fields) return null;
  const result: ElicitationField = {
    name, required, label: typeof fields.title === "string" && fields.title ? fields.title : name,
    description: typeof fields.description === "string" ? fields.description : null,
    defaultValue: fields.default ?? null, input: "text", options: [],
  };
  if (fields.type === "array") {
    if (!knownKeys(fields, [...COMMON_KEYS, "items", "minItems", "maxItems"])) return null;
    const items = record(fields.items);
    if (!items || !knownKeys(items, ["type", "enum", "anyOf", "oneOf"]) || (items.type !== undefined && items.type !== "string")) return null;
    if (["enum", "anyOf", "oneOf"].filter((key) => key in items).length !== 1) return null;
    const values = stringList(items.enum);
    const options = "enum" in items ? values?.map((value) => ({ value, label: value })) : constOptions(items.anyOf ?? items.oneOf);
    const bounds = limits(fields, "minItems", "maxItems", true);
    if (!options || !bounds) return null;
    Object.assign(result, bounds, { input: "multiSelect", options });
  } else if (fields.type === "string") {
    if (!knownKeys(fields, [...COMMON_KEYS, "enum", "enumNames", "oneOf", "minLength", "maxLength", "format"])) return null;
    if ("enum" in fields && "oneOf" in fields) return null;
    if ("enumNames" in fields && !("enum" in fields)) return null;
    const bounds = limits(fields, "minLength", "maxLength", true);
    if (!bounds || (fields.format != null && (typeof fields.format !== "string" || !FORMATS.includes(fields.format)))) return null;
    Object.assign(result, bounds);
    if (fields.format != null) result.format = fields.format as McpElicitationStringFormat;
    if ("enum" in fields) {
      const values = stringList(fields.enum);
      const names = fields.enumNames == null ? null : stringList(fields.enumNames);
      if (!values || (fields.enumNames != null && (!names || names.length !== values.length))) return null;
      result.input = "select";
      result.options = values.map((value, index) => ({ value, label: names?.[index] ?? value }));
    } else if ("oneOf" in fields) {
      const options = constOptions(fields.oneOf);
      if (!options) return null;
      result.input = "select";
      result.options = options;
    }
  } else if (fields.type === "number" || fields.type === "integer") {
    if (!knownKeys(fields, [...COMMON_KEYS, "minimum", "maximum"])) return null;
    const bounds = limits(fields, "minimum", "maximum");
    if (!bounds) return null;
    Object.assign(result, bounds, { input: fields.type });
  } else if (fields.type === "boolean") {
    if (!knownKeys(fields, COMMON_KEYS)) return null;
    result.input = "boolean";
  } else return null;
  if ((result.input === "select" || result.input === "multiSelect")
    && (result.options.length === 0 || new Set(result.options.map((option) => option.value)).size !== result.options.length)) return null;
  // 잘못된 기본값을 그대로 제출하거나 다른 값으로 고치지 않는다.
  if (fields.default != null && fieldError(result, fields.default) !== null) return null;
  return result;
}

/** 지원하지 않는 제약이 있으면 수락할 수 없는 양식으로 남긴다. */
export function elicitationFields(schema: unknown): ElicitationField[] | null {
  const fields = record(schema);
  const properties = record(fields?.properties);
  if (!fields || fields.type !== "object" || !properties
    || !knownKeys(fields, ["$schema", "title", "description", "type", "properties", "required"])) return null;
  const required = fields.required == null ? [] : stringList(fields.required);
  if (!required || required.some((name) => !Object.hasOwn(properties, name))) return null;
  const result: ElicitationField[] = [];
  for (const [name, property] of Object.entries(properties)) {
    const field = elicitationField(name, property, required.includes(name));
    if (!field) return null;
    result.push(field);
  }
  return result;
}

export function initialElicitationValue(field: ElicitationField): unknown {
  if (field.defaultValue !== null) return field.defaultValue;
  if (field.input === "boolean") return false;
  if (field.input === "multiSelect") return [];
  // 선택 전 상태와 enum의 실제 빈 문자열 옵션을 구분한다.
  return field.input === "select" ? undefined : "";
}

function validDate(value: string): boolean {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (!match) return false;
  const year = Number(match[1]), month = Number(match[2]), day = Number(match[3]);
  const leap = year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0);
  const days = [31, leap ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
  return month >= 1 && month <= 12 && day >= 1 && day <= days[month - 1]!;
}

function validFormat(format: McpElicitationStringFormat, value: string): boolean {
  if (format === "date") return validDate(value);
  if (format === "date-time") {
    const match = /^(\d{4}-\d{2}-\d{2})[tT](\d{2}):(\d{2}):(\d{2})(?:\.\d+)?([zZ]|([+-])(\d{2}):(\d{2}))$/.exec(value);
    if (!match || !validDate(match[1]!)) return false;
    const hour = Number(match[2]), minute = Number(match[3]), second = Number(match[4]);
    const offsetHour = Number(match[7] ?? 0), offsetMinute = Number(match[8] ?? 0);
    const offset = (offsetHour * 60 + offsetMinute) * (match[6] === "-" ? -1 : 1);
    const utcMinute = ((hour * 60 + minute - offset) % 1440 + 1440) % 1440;
    return hour <= 23 && minute <= 59 && offsetHour <= 23 && offsetMinute <= 59
      && (second <= 59 || (second === 60 && utcMinute === 1439));
  }
  if (format === "uri") {
    if (!/^[a-z][a-z\d+.-]*:/i.test(value) || /[^\x21-\x7e]|[<>"{}|\\^\x60]|%(?![\da-f]{2})/i.test(value)) return false;
    try { new URL(value); return true; } catch { return false; }
  }
  // 일반 주소와 인용한 local-part, 도메인 리터럴을 지원한다.
  const at = value.lastIndexOf("@");
  if (at < 1) return false;
  const local = value.slice(0, at), domain = value.slice(at + 1);
  const atom = /^[a-z\d!#$%&'*+/=?^_\x60{|}~.-]+$/i.test(local) && !local.startsWith(".") && !local.endsWith(".") && !local.includes("..");
  const quoted = /^"(?:[\x20-\x21\x23-\x5b\x5d-\x7e]|\\[\x20-\x7e])*"$/.test(local);
  if (!atom && !quoted) return false;
  if (domain.startsWith("[") && domain.endsWith("]")) {
    const address = domain.slice(1, -1);
    if (address.startsWith("IPv6:")) {
      try { new URL("http://[" + address.slice(5) + "]/"); return true; } catch { return false; }
    }
    return /^(?:\d{1,3}\.){3}\d{1,3}$/.test(address) && address.split(".").every((part) => Number(part) <= 255);
  }
  return domain.split(".").every((part) => /^[a-z\d](?:[a-z\d-]{0,61}[a-z\d])?$/i.test(part));
}

/** 이미 타입을 맞춘 한 값의 제약을 검사한다. 오류에는 사용자 입력을 넣지 않는다. */
function fieldError(field: ElicitationField, value: unknown): string | null {
  if (field.input === "number" || field.input === "integer") {
    if (typeof value !== "number" || !Number.isFinite(value)) return "유한한 숫자를 입력하세요.";
    if (field.input === "integer" && !Number.isInteger(value)) return "정수를 입력하세요.";
    if (field.minimum !== undefined && value < field.minimum) return field.minimum + " 이상으로 입력하세요.";
    if (field.maximum !== undefined && value > field.maximum) return field.maximum + " 이하로 입력하세요.";
  } else if (field.input === "boolean") {
    if (typeof value !== "boolean") return "참 또는 거짓을 선택하세요.";
  } else if (field.input === "multiSelect") {
    const values = stringList(value);
    if (!values || values.some((item) => !field.options.some((option) => option.value === item))) return "목록에 있는 항목을 선택하세요.";
    if (field.minItems !== undefined && values.length < field.minItems) return field.minItems + "개 이상 선택하세요.";
    if (field.maxItems !== undefined && values.length > field.maxItems) return field.maxItems + "개 이하로 선택하세요.";
  } else {
    if (typeof value !== "string") return "문자열을 입력하세요.";
    if (field.input === "select" && !field.options.some((option) => option.value === value)) return "목록에 있는 항목을 선택하세요.";
    const length = Array.from(value).length;
    if (field.minLength !== undefined && length < field.minLength) return field.minLength + "자 이상 입력하세요.";
    if (field.maxLength !== undefined && length > field.maxLength) return field.maxLength + "자 이하로 입력하세요.";
    if (field.format && !validFormat(field.format, value)) return field.format + " 형식으로 입력하세요.";
  }
  return null;
}

export type ElicitationValidation = { content: Record<string, unknown>; error: null } | { content: null; error: string };

/** HTML 제약만 믿지 않고, 응답을 보내기 직전에 필수 여부와 모든 지원 제약을 검사한다. */
export function validateElicitation(fields: ElicitationField[], values: Record<string, unknown>): ElicitationValidation {
  const entries: Array<[string, unknown]> = [];
  for (const field of fields) {
    let value = values[field.name];
    const numeric = field.input === "number" || field.input === "integer";
    const blankNumber = numeric && typeof value === "string" && value.trim() === "";
    const blankOptional = !field.required && (
      (field.input === "text" && value === "" && field.defaultValue !== "")
      || (field.input === "multiSelect" && Array.isArray(value) && value.length === 0
        && !(Array.isArray(field.defaultValue) && field.defaultValue.length === 0)));
    if (value === undefined || blankNumber || blankOptional) {
      if (field.required) return { content: null, error: field.label + ": 값을 입력하세요." };
      continue;
    }
    if (numeric && typeof value === "string") {
      value = /^[+-]?(?:\d+\.?\d*|\.\d+)(?:e[+-]?\d+)?$/i.test(value) ? Number(value) : Number.NaN;
    }
    const error = fieldError(field, value);
    if (error) return { content: null, error: field.label + ": " + error };
    entries.push([field.name, value]);
  }
  return { content: Object.fromEntries(entries), error: null };
}
