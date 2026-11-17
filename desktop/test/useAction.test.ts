import { describe, expect, it } from "vitest";

import { type ActionState, runAction } from "../src/renderer/work/useAction";

async function phases(action: () => Promise<void>): Promise<ActionState[]> {
  const seen: ActionState[] = [];
  await runAction(action, (state) => seen.push(state));
  return seen;
}

describe("runAction", () => {
  it("ends idle after a successful action", async () => {
    expect(await phases(async () => undefined)).toEqual([{ phase: "running" }, { phase: "idle" }]);
  });

  it("reports the failure instead of throwing", async () => {
    expect(
      await phases(async () => {
        throw new Error("no access");
      }),
    ).toEqual([{ phase: "running" }, { phase: "failed", message: "no access" }]);
  });

  it("reports a failure that is not an Error", async () => {
    expect(await phases(() => Promise.reject("offline"))).toEqual([{ phase: "running" }, { phase: "failed", message: "offline" }]);
  });
});
