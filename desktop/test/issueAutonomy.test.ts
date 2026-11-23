import { describe, expect, it } from "vitest";
import { hasValidIssueAutonomy } from "../src/renderer/kernel/issuePresentationValidation";
import type { IssueAutonomyState } from "../src/shared/kernel-protocol/generated";

const ready = (): Extract<IssueAutonomyState, { status: "ready" }> => ({
  status: "ready", requested_level: "auto", effective_level: "propose", cap: "auto", revision: 4,
  policy_version: "current-policy", pending: false,
  choices: (["off", "observe", "propose", "gated", "auto"] as const).map((level) => ({
    level, enabled: true, requires_confirmation: level === "gated" || level === "auto", reason: null,
  })),
});
const valid = (autonomy: unknown) => hasValidIssueAutonomy({ autonomy });

describe("runtime Auto choice projection", () => {
  it("preserves requested Auto and a lower server effective level without inferring delivery permission", () => {
    const projection = ready();
    const before = structuredClone(projection);
    expect(valid(projection)).toBe(true);
    expect(projection).toEqual(before);
  });

  it("retains legacy choices while requiring a matching option for an explicit Auto request", () => {
    const projection = ready();
    const legacy = { ...projection, requested_level: "propose", effective_level: "propose",
      choices: projection.choices.filter((choice) => !["gated", "auto"].includes(choice.level)) };
    expect(valid(legacy)).toBe(true);
    expect(valid({ ...legacy, requested_level: "auto" })).toBe(false);
    expect(valid({ ...projection, choices: projection.choices.filter((choice) => choice.level !== "auto") })).toBe(false);
  });

  it("rejects duplicate and unsupported Auto choices while keeping a disabled server choice", () => {
    const projection = ready();
    const auto = projection.choices.find((choice) => choice.level === "auto")!;
    expect(valid({ ...projection, choices: [...projection.choices, auto] })).toBe(false);
    expect(valid({ ...projection, requested_level: "AUTO" })).toBe(false);
    expect(valid({ ...projection, choices: [...projection.choices, { ...auto, level: "automatic" }] })).toBe(false);
    expect(valid({ ...projection, cap: "gated", choices: projection.choices.map((choice) =>
      choice.level === "auto" ? { ...choice, enabled: false, reason: "현재 정책 상한입니다." } : choice) })).toBe(true);
  });
});
