import { describe, expect, it } from "vitest";

import { elicitationFields, initialElicitationValue, validateElicitation } from "./elicitation";

function fields(property: Record<string, unknown>, required = true) {
  const parsed = elicitationFields({ type: "object", properties: { value: property }, required: required ? ["value"] : [] });
  expect(parsed).not.toBeNull();
  return parsed!;
}

describe("MCP elicitation submission validation", () => {
  it.each([
    [{ type: "integer", minimum: 2, maximum: 4 }, "1"],
    [{ type: "integer", minimum: 2, maximum: 4 }, "5"],
    [{ type: "integer" }, "2.5"],
    [{ type: "number" }, "1e999"],
    [{ type: "number" }, Number.POSITIVE_INFINITY],
    [{ type: "number" }, "not a number"],
    [{ type: "string", minLength: 2 }, "x"],
    [{ type: "string", maxLength: 2 }, "abc"],
    [{ type: "string", enum: ["a", "b"] }, "c"],
    [{ type: "array", minItems: 1, items: { enum: ["a", "b"] } }, []],
    [{ type: "array", maxItems: 1, items: { enum: ["a", "b"] } }, ["a", "b"]],
    [{ type: "array", items: { anyOf: [{ const: "a", title: "A" }] } }, ["b"]],
    [{ type: "boolean" }, "false"],
  ] as Array<[Record<string, unknown>, unknown]>)("rejects values outside %j", (schema, value) => {
    const result = validateElicitation(fields(schema), { value });
    expect(result.content).toBeNull();
    expect(result.error).toBeTruthy();
  });

  it.each([
    ["email", "person@example.com", "not-an-address"],
    ["email", '"quoted local"@example.com', ".person@example.com"],
    ["uri", "https://example.com/page?q=1", "relative/page"],
    ["uri", "urn:example:page", "https://example.com/bad path"],
    ["date", "2024-02-29", "2025-02-29"],
    ["date", "2026-11-26", "2026-13-01"],
    ["date-time", "2026-11-26T11:30:00+09:00", "2026-11-26T11:30:00"],
    ["date-time", "2026-11-26T11:30:00.123Z", "2026-11-26T24:30:00Z"],
    ["date-time", "2024-02-29T11:30:00Z", "2025-02-29T11:30:00Z"],
  ])("validates %s without changing the accepted value", (format, valid, invalid) => {
    const parsed = fields({ type: "string", format });
    expect(validateElicitation(parsed, { value: valid })).toEqual({ content: { value: valid }, error: null });
    expect(validateElicitation(parsed, { value: invalid }).content).toBeNull();
  });

  it("omits blank optional inputs without imposing their constraints", () => {
    const parsed = elicitationFields({ type: "object", properties: {
      text: { type: "string", minLength: 2 },
      number: { type: "number", minimum: 2 },
      array: { type: "array", minItems: 1, items: { enum: ["a"] } },
      select: { type: "string", enum: ["a"] },
    } })!;
    expect(validateElicitation(parsed, { text: "", number: "", array: [], select: undefined })).toEqual({ content: {}, error: null });
  });

  it("requires property presence while permitting empty strings and arrays without minimums", () => {
    const parsed = elicitationFields({ type: "object", properties: {
      text: { type: "string" }, array: { type: "array", items: { enum: ["a"] } },
    }, required: ["text", "array"] })!;
    expect(validateElicitation(parsed, {} ).content).toBeNull();
    expect(validateElicitation(parsed, { text: "", array: [] })).toEqual({ content: { text: "", array: [] }, error: null });
    expect(validateElicitation(fields({ type: "number" }), { value: "" }).content).toBeNull();
  });

  it("preserves valid defaults including false, zero, empty string, and empty arrays", () => {
    const parsed = elicitationFields({ type: "object", properties: {
      flag: { type: "boolean", default: false },
      count: { type: "integer", minimum: 0, default: 0 },
      text: { type: "string", default: "" },
      emptyEnum: { type: "string", enum: ["", "x"], default: "" },
      titled: { type: "string", oneOf: [{ const: "a", title: "A" }], default: "a" },
      legacy: { type: "string", enum: ["b"], enumNames: ["B"], default: "b" },
      array: { type: "array", items: { enum: ["a"] }, default: [] },
      titledArray: { type: "array", items: { anyOf: [{ const: "c", title: "C" }] }, default: ["c"] },
    } })!;
    const values = Object.fromEntries(parsed.map((field) => [field.name, initialElicitationValue(field)]));
    expect(validateElicitation(parsed, values)).toEqual({ content: {
      flag: false, count: 0, text: "", emptyEnum: "", titled: "a", legacy: "b", array: [], titledArray: ["c"],
    }, error: null });
  });

  it("treats nullable optional schema metadata as absent", () => {
    const parsed = elicitationFields({ type: "object", required: null, properties: {
      number: { type: "number", minimum: null, maximum: null, default: null },
      text: { type: "string", minLength: null, maxLength: null, format: null, default: null },
      array: { type: "array", minItems: null, maxItems: null, items: { enum: ["a"] }, default: null },
      enum: { type: "string", enum: ["a"], enumNames: null, default: null },
    } });
    expect(parsed).not.toBeNull();
    expect(validateElicitation(parsed!, { number: "3", text: "a", array: ["a"], enum: "a" })).toEqual({
      content: { number: 3, text: "a", array: ["a"], enum: "a" }, error: null,
    });
  });

  it("counts Unicode code points rather than UTF-16 code units", () => {
    expect(validateElicitation(fields({ type: "string", minLength: 1, maxLength: 1 }), { value: "😀" }).error).toBeNull();
    expect(validateElicitation(fields({ type: "string", minLength: 2 }), { value: "😀" }).content).toBeNull();
  });

  it.each([
    { type: "integer", default: 1.5 },
    { type: "integer", default: "3" },
    { type: "boolean", default: 0 },
    { type: "string", enum: ["a"], default: "b" },
    { type: "string", minLength: 2, default: "" },
    { type: "array", minItems: 1, items: { enum: ["a"] }, default: [] },
    { type: "array", items: { enum: ["a"] }, default: ["b"] },
    { type: "string", minLength: 4, maxLength: 2 },
    { type: "number", minimum: 4, maximum: 2 },
    { type: "string", format: "hostname" },
  ])("leaves unsupported or inconsistent defaults/constraints unaccepted: %j", (property) => {
    expect(elicitationFields({ type: "object", properties: { value: property } })).toBeNull();
  });
});
