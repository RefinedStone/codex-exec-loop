import { describe, expect, it } from "vitest";

import { newContentTurnId } from "./newContent";

describe("new content divider (#2843)", () => {
  it("marks the first turn after the last seen one", () => {
    expect(newContentTurnId(["a", "b", "c"], "a")).toBe("b");
    expect(newContentTurnId(["a", "b", "c"], "c")).toBeNull();
    expect(newContentTurnId(["a", "b"], null)).toBeNull();
    // The last seen turn may not be loaded (older pages); then there is no divider.
    expect(newContentTurnId(["x", "y"], "a")).toBeNull();
  });
});
