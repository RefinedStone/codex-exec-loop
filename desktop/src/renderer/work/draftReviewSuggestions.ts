import type { IssueDraftField, IssueDraftText } from "./issueDraft";

type Suggestion = { readonly field: IssueDraftField; readonly problem: string; readonly originalText: string; readonly suggestedText: string };
type Review = { readonly suggestions: readonly Suggestion[] } | { readonly refusal: string };

const isRecord = (value: unknown): value is Record<string, unknown> => typeof value === "object" && value !== null && !Array.isArray(value);
function isSuggestion(value: unknown): value is Suggestion {
  return isRecord(value) && Object.keys(value).length === 4 && (value.field === "title" || value.field === "body") && typeof value.problem === "string" && typeof value.originalText === "string" && typeof value.suggestedText === "string";
}
export function readReview(text: string): Review | null {
  try {
    const value: unknown = JSON.parse(text.trim().replace(/^```(?:json)?\s*([\s\S]*?)\s*```$/, "$1"));
    if (!isRecord(value) || Object.keys(value).some(key => key !== "suggestions" && key !== "refusal")) return null;
    if (typeof value.refusal === "string") return { refusal: value.refusal };
    if (Array.isArray(value.suggestions) && value.suggestions.every(isSuggestion)) return { suggestions: value.suggestions };
    return null;
  } catch (error) {
    if (error instanceof SyntaxError) return null;
    throw error;
  }
}

/** Match both copies: a removed or ambiguous excerpt must never select a different piece of private text. */
export function revisedField(local: IssueDraftText, shared: Readonly<Record<IssueDraftField, string>>, proposal: Suggestion) {
  const { field, originalText, suggestedText } = proposal;
  const raw = local[field];
  if (!originalText || originalText === suggestedText) return null;
  for (const text of [shared[field], raw]) {
    if (!text.includes(originalText) || text.indexOf(originalText) !== text.lastIndexOf(originalText)) return null;
  }
  const start = raw.indexOf(originalText);
  const end = start + originalText.length;
  if (local.spans.some(span => span.field === field && start < span.end && span.start < end)) return null;
  const delta = suggestedText.length - originalText.length;
  const spans = local.spans.map(span => span.field === field && span.start >= end
    ? { ...span, start: span.start + delta, end: span.end + delta } : span);
  return { field, value: raw.slice(0, start) + suggestedText + raw.slice(end), spans };
}
