import { describe, expect, it } from "vitest";

import { type ActionState, actionRunner } from "../src/renderer/work/useAction";

async function phases(action: () => Promise<void>): Promise<ActionState[]> {
  const seen: ActionState[] = [];
  await actionRunner((state) => seen.push(state))(action);
  return seen;
}

describe("actionRunner", () => {
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

  it("ignores a second call while the first one runs, and runs again after it ends", async () => {
    const seen: ActionState[] = [];
    const run = actionRunner((state) => seen.push(state));
    let calls = 0;
    let finish = () => {};
    const slow = () => {
      calls += 1;
      return new Promise<void>((resolve) => (finish = resolve));
    };
    const first = run(slow);
    await run(slow);
    expect(calls).toBe(1);
    finish();
    await first;
    expect(seen).toEqual([{ phase: "running" }, { phase: "idle" }]);
    void run(slow);
    expect(calls).toBe(2);
  });
});
