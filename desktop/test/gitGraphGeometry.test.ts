import { describe, expect, it } from "vitest";

import { graphWidth, LANE_WIDTH, laneColorIndex, laneX, MAX_VISIBLE_LANES, ROW_HEIGHT, rowPaths } from "../src/renderer/git/graphGeometry";

describe("commit graph row geometry", () => {
  it("draws a straight line through the node for a commit in the middle of a branch", () => {
    const paths = rowPaths({ lane: 0, lanesIn: [0], lanesOut: [0], lanesThrough: [] });
    const x = laneX(0);
    expect(paths).toEqual([
      { d: `M${x} 0L${x} ${ROW_HEIGHT / 2}`, colorLane: 0 },
      { d: `M${x} ${ROW_HEIGHT}L${x} ${ROW_HEIGHT / 2}`, colorLane: 0 },
    ]);
  });

  it("bends the line to another lane and colors it by that lane", () => {
    const [second] = rowPaths({ lane: 0, lanesIn: [], lanesOut: [1], lanesThrough: [] });
    expect(second!.colorLane).toBe(1);
    expect(second!.d.startsWith(`M${laneX(1)} ${ROW_HEIGHT}C`)).toBe(true);
    expect(second!.d.endsWith(`${laneX(0)} ${ROW_HEIGHT / 2}`)).toBe(true);
  });

  it("draws passing lanes from the top edge to the bottom edge", () => {
    const [passing] = rowPaths({ lane: 1, lanesIn: [], lanesOut: [], lanesThrough: [0] });
    expect(passing).toEqual({ d: `M${laneX(0)} 0L${laneX(0)} ${ROW_HEIGHT}`, colorLane: 0 });
  });

  it("sizes the graph column to the widest row and caps it", () => {
    expect(graphWidth([])).toBe(LANE_WIDTH);
    expect(graphWidth([{ lane: 0, lanesIn: [], lanesOut: [0, 2], lanesThrough: [] }])).toBe(3 * LANE_WIDTH);
    expect(graphWidth([{ lane: 40, lanesIn: [], lanesOut: [], lanesThrough: [] }])).toBe(MAX_VISIBLE_LANES * LANE_WIDTH);
  });

  it("cycles lane colors", () => {
    expect(laneColorIndex(0)).toBe(0);
    expect(laneColorIndex(9)).toBe(1);
  });
});
