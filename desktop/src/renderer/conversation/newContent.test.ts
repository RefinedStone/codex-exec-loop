import { describe, expect, it } from "vitest";

import { newContentTurnId, RESTORE_MAX_OLDER_PAGES, restoreStep } from "./newContent";

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
});
