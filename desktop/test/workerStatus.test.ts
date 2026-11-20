// 워커 상태판의 상태 계산이다 (#2378).
import { describe, expect, it } from "vitest";

import type { ParallelAgentState } from "../src/renderer/kernel/protocol";
import { WORKER_STEP, workerStatus } from "../src/renderer/kernel/workerStatus";

describe("workerStatus", () => {
  it("maps roster states to the board states", () => {
    expect(workerStatus("starting")).toBe("waiting");
    expect(workerStatus("running")).toBe("running");
    expect(workerStatus("pr_pending")).toBe("waiting");
    expect(workerStatus("official_refresh_recovery_needed")).toBe("blocked");
    expect(workerStatus("failed")).toBe("failed");
  });

  it("shows done only after the branch is integrated", () => {
    const done = (Object.keys(WORKER_STEP) as ParallelAgentState[]).filter((state) => workerStatus(state) === "done");
    expect(done).toEqual(["cleanup_pending"]);
    expect(workerStatus("reported_complete")).toBe("running");
  });

  it("never shows an unconfirmed state as done", () => {
    expect(workerStatus("unknown")).toBe("unknown");
    expect(workerStatus("a_newer_state" as ParallelAgentState)).toBe("unknown");
  });
});
