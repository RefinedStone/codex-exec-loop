import { readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

// @ts-expect-error -- plain ESM script without type declarations
import { defaultE2eSpecs, MANUAL_E2E_SPECS } from "../scripts/run-e2e.mjs";

describe("npm run e2e spec list", () => {
  const fileNames = readdirSync(fileURLToPath(new URL("./e2e/", import.meta.url)));

  it("runs every e2e spec except the listed manual ones", () => {
    const specs: string[] = defaultE2eSpecs(fileNames);
    expect(specs).toEqual(expect.arrayContaining(["branch-publish.mjs", "hunk-actions.mjs", "jira-versions.mjs", "approval-flow.mjs"]));
    const manual = [...(MANUAL_E2E_SPECS as Map<string, string>).keys()];
    const notRun = fileNames.filter((name) => name.endsWith(".mjs") && !specs.includes(name));
    expect(notRun.sort()).toEqual(manual.sort());
    expect(specs.some((name) => name.endsWith(".cjs"))).toBe(false);
  });
});
