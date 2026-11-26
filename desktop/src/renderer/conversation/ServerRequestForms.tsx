// 승인 카드 안에 그리는 양식이다 (#2836): 추가 권한 내용, 도구의 사용자 입력, MCP elicitation.
// 응답은 그 요청의 응답 형식으로 만들어 `onRespond` 로 보낸다.
import { type FormEvent, useState } from "react";

import type { FileSystemPath } from "../../shared/protocol/generated/v2/FileSystemPath";
import type { RequestPermissionProfile } from "../../shared/protocol/generated/v2/RequestPermissionProfile";
import type { ToolRequestUserInputQuestion } from "../../shared/protocol/generated/v2/ToolRequestUserInputQuestion";
import { type ElicitationField, type ElicitationRequest, elicitationResponse, userInputResponse } from "./approvals";
import { initialElicitationValue, validateElicitation } from "./elicitation";

function permissionPath(path: FileSystemPath): string {
  if (path.type === "path") return path.path;
  if (path.type === "glob_pattern") return "glob: " + path.pattern;
  const special = path.value;
  const scope = special.kind === "unknown" ? special.kind + ": " + special.path : special.kind;
  return scope + ("subpath" in special && special.subpath !== null ? " / " + special.subpath : "");
}

const ACCESS_LABEL = { read: "읽기", write: "쓰기", deny: "거부" } as const;

export function PermissionsDetail({ permissions }: { permissions: RequestPermissionProfile }) {
  const read = permissions.fileSystem?.read ?? [];
  const write = permissions.fileSystem?.write ?? [];
  return (
    <ul className="approval-permissions" data-testid="approval-permissions">
      {permissions.network?.enabled && <li>네트워크 사용</li>}
      {read.map((path) => <li key={`r:${path}`}>읽기: <code>{path}</code></li>)}
      {write.map((path) => <li key={`w:${path}`}>쓰기: <code>{path}</code></li>)}
      {permissions.fileSystem?.entries?.map((entry, index) => (
        <li key={index} data-testid="permission-entry">{ACCESS_LABEL[entry.access]} ({entry.access}): <code>{permissionPath(entry.path)}</code></li>
      ))}
      {permissions.fileSystem?.globScanMaxDepth !== undefined && (
        <li>glob 검색 최대 깊이: {permissions.fileSystem.globScanMaxDepth}</li>
      )}
    </ul>
  );
}

const OTHER = "\u0000other";

function QuestionInput({ question, value, onChange }: {
  question: ToolRequestUserInputQuestion;
  value: string;
  onChange(value: string): void;
}) {
  const options = question.options ?? [];
  const [other, setOther] = useState(false);
  const textType = question.isSecret ? "password" : "text";
  return (
    <fieldset className="approval-question" data-testid="user-input-question">
      <legend>{question.header || question.question}</legend>
      {question.header && <p>{question.question}</p>}
      {options.map((option) => (
        <label key={option.label} className="approval-option">
          <input type="radio" name={question.id} checked={!other && value === option.label}
            onChange={() => { setOther(false); onChange(option.label); }} />
          <span>{option.label}</span>
          {option.description && <span className="muted"> — {option.description}</span>}
        </label>
      ))}
      {question.isOther && options.length > 0 && (
        <label className="approval-option">
          <input type="radio" name={question.id} value={OTHER} checked={other} onChange={() => { setOther(true); onChange(""); }} />
          <span>직접 입력</span>
        </label>
      )}
      {(options.length === 0 || other) && (
        <input type={textType} className="approval-text" aria-label={question.question} value={value}
          onChange={(event) => onChange(event.target.value)} data-testid="user-input-text" />
      )}
    </fieldset>
  );
}

export function UserInputForm({ questions, onRespond }: {
  questions: ToolRequestUserInputQuestion[];
  onRespond(result: unknown): void;
}) {
  const [answers, setAnswers] = useState<Record<string, string>>({});
  const submit = (event: FormEvent) => {
    event.preventDefault();
    onRespond(userInputResponse(answers));
  };
  return (
    <form className="approval-form" onSubmit={submit} data-testid="user-input-form">
      {questions.map((question) => (
        <QuestionInput key={question.id} question={question} value={answers[question.id] ?? ""}
          onChange={(value) => setAnswers((current) => ({ ...current, [question.id]: value }))} />
      ))}
      <div className="approval-buttons">
        <button type="submit" className="button primary" data-testid="user-input-submit">보내기</button>
        <button type="button" className="button" onClick={() => onRespond(userInputResponse({}))} data-testid="user-input-skip">답하지 않기</button>
      </div>
    </form>
  );
}

function FieldInput({ field, value, onChange }: { field: ElicitationField; value: unknown; onChange(value: unknown): void }) {
  if (field.input === "boolean") {
    return <input type="checkbox" checked={value === true} onChange={(event) => onChange(event.target.checked)} />;
  }
  if (field.input === "select") {
    const selected = value === undefined ? -1 : field.options.findIndex((option) => option.value === value);
    return (
      <select value={selected < 0 ? "" : String(selected)} aria-required={field.required}
        onChange={(event) => onChange(event.target.value === "" ? undefined : field.options[Number(event.target.value)]!.value)}>
        <option value="">선택</option>
        {field.options.map((option, index) => <option key={option.value} value={String(index)}>{option.label || "(빈 문자열)"}</option>)}
      </select>
    );
  }
  if (field.input === "multiSelect") {
    const selected = Array.isArray(value) ? value as string[] : [];
    return (
      <span className="approval-multi">
        {field.options.map((option) => (
          <label key={option.value}>
            <input type="checkbox" checked={selected.includes(option.value)}
              onChange={(event) => onChange(event.target.checked ? [...selected, option.value] : selected.filter((item) => item !== option.value))} />
            {option.label || "(빈 문자열)"}
          </label>
        ))}
      </span>
    );
  }
  const numeric = field.input === "number" || field.input === "integer";
  const type = numeric ? "number" : field.format === "email" ? "email" : field.format === "uri" ? "url" : "text";
  return (
    <input type={type} step={field.input === "integer" ? 1 : "any"} aria-required={field.required}
      min={field.minimum} max={field.maximum}
      value={String(value ?? "")} onChange={(event) => onChange(event.target.value)} />
  );
}

function fieldHint(field: ElicitationField): string {
  return [
    field.minimum !== undefined ? "최소 " + field.minimum : null,
    field.maximum !== undefined ? "최대 " + field.maximum : null,
    field.minLength !== undefined ? "최소 " + field.minLength + "자" : null,
    field.maxLength !== undefined ? "최대 " + field.maxLength + "자" : null,
    field.minItems !== undefined ? "최소 " + field.minItems + "개" : null,
    field.maxItems !== undefined ? "최대 " + field.maxItems + "개" : null,
    field.format,
  ].filter(Boolean).join(" · ");
}

export function ElicitationForm({ request, onRespond }: { request: ElicitationRequest; onRespond(result: unknown): void }) {
  const fields = request.fields;
  const [values, setValues] = useState<Record<string, unknown>>(() =>
    Object.fromEntries((fields ?? []).map((field) => [field.name, initialElicitationValue(field)])));
  const [error, setError] = useState<string | null>(null);
  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (fields) {
      const result = validateElicitation(fields, values);
      if (result.error !== null) { setError(result.error); return; }
      onRespond(elicitationResponse("accept", result.content));
    } else if (request.url !== null) {
      onRespond(elicitationResponse("accept"));
    }
  };
  const canAccept = request.url !== null || fields !== null;
  return (
    <form className="approval-form" noValidate onSubmit={submit} data-testid="elicitation-form">
      <p className="approval-server">MCP 서버 <code>{request.serverName}</code></p>
      <p>{request.message}</p>
      {request.url && (
        <p>이 주소에서 진행한 뒤 완료를 누르세요: <code className="approval-url" data-testid="elicitation-url">{request.url}</code></p>
      )}
      {fields?.map((field) => {
        const input = <FieldInput field={field} value={values[field.name]} onChange={(value) => {
          setValues((current) => ({ ...current, [field.name]: value }));
          setError(null);
        }} />;
        const detail = <>
          {field.description && <span className="muted">{field.description}</span>}
          {fieldHint(field) && <span className="muted">{fieldHint(field)}</span>}
        </>;
        return field.input === "multiSelect" ? (
          <fieldset key={field.name} className="approval-question approval-field">
            <legend>{field.label}{field.required ? " *" : ""}</legend>{input}{detail}
          </fieldset>
        ) : (
          <label key={field.name} className="approval-field">
            <span>{field.label}{field.required ? " *" : ""}</span>{input}{detail}
          </label>
        );
      })}
      {error && <p role="alert" data-testid="elicitation-error">{error}</p>}
      {!canAccept && <p data-testid="elicitation-unsupported">이 양식은 Akra Desktop 이 그릴 수 없습니다. 거절하거나 취소하세요.</p>}
      <div className="approval-buttons">
        {canAccept && <button type="submit" className="button primary" data-testid="elicitation-accept">{request.url ? "완료" : "보내기"}</button>}
        <button type="button" className="button" onClick={() => onRespond(elicitationResponse("decline"))} data-testid="elicitation-decline">거절</button>
        <button type="button" className="button" onClick={() => onRespond(elicitationResponse("cancel"))} data-testid="elicitation-cancel">취소</button>
      </div>
    </form>
  );
}
