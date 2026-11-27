import { describe, expect, it } from "vitest";
import { revisedField } from "../src/renderer/work/draftReviewSuggestions";

const span = { field: "body", start: 1, end: 5, class: "secret", decision: null } as const;
const draft = { title: "T", body: "xSAFE", spans: [span] };
const shared = { title: "T", body: "x[removed]" };
const suggestion = { field: "body", problem: "expand", originalText: "x", suggestedText: "xS" } as const;

describe("masked review edits", () => {
  it("preserves the full private span when the replacement ends in its first character", () => {
    expect(revisedField(draft, shared, suggestion)).toEqual({ field: "body", value: "xSSAFE", spans: [{ ...span, start: 2, end: 6 }] });
  });
  it("cannot use an excerpt that exists only in private text", () => {
    expect(revisedField(draft, shared, { ...suggestion, originalText: "SAFE" })).toBeNull();
  });
});
