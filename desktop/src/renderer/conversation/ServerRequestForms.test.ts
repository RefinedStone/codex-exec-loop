import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import type { RequestPermissionProfile } from "../../shared/protocol/generated/v2/RequestPermissionProfile";
import { elicitationFields } from "./approvals";
import { PermissionsDetail } from "./ServerRequestForms";

describe("server request form review regressions", () => {
  it("shows each filesystem path variant and its access mode before granting it", () => {
    const permissions: RequestPermissionProfile = {
      network: null,
      fileSystem: {
        read: ["/legacy/read"], write: ["/legacy/write"], globScanMaxDepth: 3,
        entries: [
          { path: { type: "path", path: "/workspace/source" }, access: "read" },
          { path: { type: "glob_pattern", pattern: "/workspace/build/**" }, access: "write" },
          { path: { type: "special", value: { kind: "project_roots", subpath: "secrets" } }, access: "deny" },
          { path: { type: "special", value: { kind: "unknown", path: "future_root", subpath: "config" } }, access: "read" },
          ...(["root", "minimal", "tmpdir", "slash_tmp"] as const).map((kind) => ({
            path: { type: "special" as const, value: { kind } }, access: "read" as const,
          })),
        ],
      },
    };
    const html = renderToStaticMarkup(createElement(PermissionsDetail, { permissions }));
    for (const text of ["/legacy/read", "/legacy/write", "/workspace/source", "/workspace/build/**",
      "project_roots", "secrets", "future_root", "config", "root", "minimal", "tmpdir", "slash_tmp",
      "읽기", "쓰기", "거부"]) expect(html).toContain(text);
    expect(html.match(/data-testid="permission-entry"/g)).toHaveLength(8);
  });

  it("retains the requested numeric, text and multi-select constraints", () => {
    const fields = elicitationFields({
      type: "object",
      properties: {
        count: { type: "integer", minimum: 2, maximum: 4, default: 3 },
        name: { type: "string", minLength: 2, maxLength: 8 },
        email: { type: "string", format: "email" },
        tags: { type: "array", minItems: 1, maxItems: 2, items: { type: "string", enum: ["a", "b", "c"] } },
      },
      required: ["count", "name", "tags"],
    });
    expect(fields?.find((field) => field.name === "count")).toMatchObject({ minimum: 2, maximum: 4, defaultValue: 3 });
    expect(fields?.find((field) => field.name === "name")).toMatchObject({ minLength: 2, maxLength: 8 });
    expect(fields?.find((field) => field.name === "email")).toMatchObject({ format: "email" });
    expect(fields?.find((field) => field.name === "tags")).toMatchObject({ minItems: 1, maxItems: 2 });
  });

  it.each([
    { type: "string", pattern: "^[a-z]+$" },
    { type: "number", multipleOf: 2 },
    { type: "array", uniqueItems: true, items: { enum: ["a"] } },
  ])("does not silently drop a constraint it cannot render: %j", (property) => {
    expect(elicitationFields({ type: "object", properties: { value: property } })).toBeNull();
  });
});
