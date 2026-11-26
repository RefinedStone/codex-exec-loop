import { describe, expect, it } from "vitest";

import { memoryOnLeave, newContentTurnId, RESTORE_MAX_OLDER_PAGES, restoreStep } from "./newContent";

describe("new content divider (#2843)", () => {
  it("marks the first turn after the last seen one", () => {
    expect(newContentTurnId(["a", "b", "c"], "a")).toBe("b");
    expect(newContentTurnId(["a", "b", "c"], "c")).toBeNull();
    expect(newContentTurnId(["a", "b"], null)).toBeNull();
    // The last seen turn may not be loaded (older pages); then there is no divider.
    expect(newContentTurnId(["x", "y"], "a")).toBeNull();
  });

  it("waits for older pages until the remembered turn is loaded", () => {
    expect(restoreStep(["a", "b"], "b", true, 0)).toEqual({ kind: "scroll", index: 1 });
    expect(restoreStep(["x", "y"], "a", true, 0)).toEqual({ kind: "loadOlder" });
    expect(restoreStep(["x", "y"], "a", false, 1)).toEqual({ kind: "bottom" });
    expect(restoreStep(["x", "y"], "a", true, RESTORE_MAX_OLDER_PAGES)).toEqual({ kind: "bottom" });
    expect(restoreStep(["x", "y"], null, true, 0)).toEqual({ kind: "bottom" });
  });

  it("keeps the old memory when the thread is left before it was restored", () => {
    type Memory = { position: { turnId: string; offset: number } | null; lastSeenTurnId: string | null };
    const previous: Memory = { position: { turnId: "turn-4", offset: 80 }, lastSeenTurnId: "turn-10" };
    const empty: Memory = { position: null, lastSeenTurnId: null };
    expect(memoryOnLeave(previous, false, empty)).toBe(previous);
    expect(memoryOnLeave(undefined, false, empty)).toBeUndefined();
    expect(memoryOnLeave(previous, true, empty)).toBe(empty);
  });
});
