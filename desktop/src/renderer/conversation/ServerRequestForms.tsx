// 승인 카드 안에 그리는 양식이다 (#2836): 추가 권한 내용, 도구의 사용자 입력, MCP elicitation.
// 응답은 그 요청의 응답 형식으로 만들어 `onRespond` 로 보낸다.
import { type FormEvent, useState } from "react";

import type { RequestPermissionProfile } from "../../shared/protocol/generated/v2/RequestPermissionProfile";
import type { ToolRequestUserInputQuestion } from "../../shared/protocol/generated/v2/ToolRequestUserInputQuestion";
import { type ElicitationField, type ElicitationRequest, elicitationResponse, userInputResponse } from "./approvals";

export function PermissionsDetail({ permissions }: { permissions: RequestPermissionProfile }) {
  const read = permissions.fileSystem?.read ?? [];
  const write = permissions.fileSystem?.write ?? [];
  return (
    <ul className="approval-permissions" data-testid="approval-permissions">
      {permissions.network?.enabled && <li>네트워크 사용</li>}
      {read.map((path) => <li key={`r:${path}`}>읽기: <code>{path}</code></li>)}
      {write.map((path) => <li key={`w:${path}`}>쓰기: <code>{path}</code></li>)}
      {(permissions.fileSystem?.entries?.length ?? 0) > 0 && <li>파일 시스템 항목 {permissions.fileSystem!.entries!.length}개</li>}
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

function initialValue(field: ElicitationField): unknown {
  if (field.defaultValue !== null) return field.defaultValue;
  if (field.input === "boolean") return false;
  if (field.input === "multiSelect") return [];
  return "";
}

function FieldInput({ field, value, onChange }: { field: ElicitationField; value: unknown; onChange(value: unknown): void }) {
  if (field.input === "boolean") {
    return <input type="checkbox" checked={value === true} onChange={(event) => onChange(event.target.checked)} />;
  }
  if (field.input === "select") {
    return (
      <select value={String(value ?? "")} required={field.required} onChange={(event) => onChange(event.target.value)}>
        <option value="" disabled>선택</option>
        {field.options.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}
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
            {option.label}
          </label>
        ))}
      </span>
    );
  }
  const numeric = field.input === "number" || field.input === "integer";
  return (
    <input type={numeric ? "number" : "text"} step={field.input === "integer" ? 1 : "any"} required={field.required}
      value={String(value ?? "")} onChange={(event) => onChange(event.target.value)} />
  );
}

/** 양식 값을 스키마 타입으로 바꾼다. 비운 선택 칸은 넣지 않는다. */
function elicitationContent(fields: ElicitationField[], values: Record<string, unknown>): Record<string, unknown> {
  const content: Record<string, unknown> = {};
  for (const field of fields) {
    const value = values[field.name];
    if (value === "" || value === undefined) continue;
    content[field.name] = field.input === "number" || field.input === "integer" ? Number(value) : value;
  }
  return content;
}

export function ElicitationForm({ request, onRespond }: { request: ElicitationRequest; onRespond(result: unknown): void }) {
  const fields = request.fields;
  const [values, setValues] = useState<Record<string, unknown>>(() =>
    Object.fromEntries((fields ?? []).map((field) => [field.name, initialValue(field)])));
  const submit = (event: FormEvent) => {
    event.preventDefault();
    onRespond(elicitationResponse("accept", fields ? elicitationContent(fields, values) : null));
  };
  const canAccept = request.url !== null || fields !== null;
  return (
    <form className="approval-form" onSubmit={submit} data-testid="elicitation-form">
      <p className="approval-server">MCP 서버 <code>{request.serverName}</code></p>
      <p>{request.message}</p>
      {request.url && (
        <p>이 주소에서 진행한 뒤 완료를 누르세요: <code className="approval-url" data-testid="elicitation-url">{request.url}</code></p>
      )}
      {fields?.map((field) => (
        <label key={field.name} className="approval-field">
          <span>{field.label}{field.required ? " *" : ""}</span>
          <FieldInput field={field} value={values[field.name]} onChange={(value) => setValues((current) => ({ ...current, [field.name]: value }))} />
          {field.description && <span className="muted">{field.description}</span>}
        </label>
      ))}
      {!canAccept && <p data-testid="elicitation-unsupported">이 양식은 Akra Desktop 이 그릴 수 없습니다. 거절하거나 취소하세요.</p>}
      <div className="approval-buttons">
        {canAccept && <button type="submit" className="button primary" data-testid="elicitation-accept">{request.url ? "완료" : "보내기"}</button>}
        <button type="button" className="button" onClick={() => onRespond(elicitationResponse("decline"))} data-testid="elicitation-decline">거절</button>
        <button type="button" className="button" onClick={() => onRespond(elicitationResponse("cancel"))} data-testid="elicitation-cancel">취소</button>
      </div>
    </form>
  );
}
