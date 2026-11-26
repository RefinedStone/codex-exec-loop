import { describe, expect, it } from "vitest";

import { fitMenuPosition } from "./menuPosition";

const viewport = { width: 800, height: 600 };
const menu = { width: 170, height: 200 };

describe("fitMenuPosition", () => {
  it("keeps the click point when the menu fits", () => {
    expect(fitMenuPosition({ x: 100, y: 100 }, menu, viewport)).toEqual({ x: 100, y: 100 });
  });

  it("opens to the left near the right edge", () => {
    expect(fitMenuPosition({ x: 750, y: 100 }, menu, viewport)).toEqual({ x: 580, y: 100 });
  });

  it("opens upward near the bottom edge", () => {
    expect(fitMenuPosition({ x: 100, y: 550 }, menu, viewport)).toEqual({ x: 100, y: 350 });
  });

  it("pins to the window edge when neither side fits", () => {
    expect(fitMenuPosition({ x: 100, y: 150 }, menu, viewport)).toEqual({ x: 100, y: 150 });
    expect(fitMenuPosition({ x: 100, y: 150 }, { width: 170, height: 500 }, viewport)).toEqual({ x: 100, y: 0 });
    expect(fitMenuPosition({ x: 100, y: 10 }, { width: 170, height: 700 }, viewport)).toEqual({ x: 100, y: 0 });
  });
});
