import { describe, expect, it } from "vitest";
import { bytes, compareBytes, formatBytes, formatRatio, ratioWidth, treemapTiles } from "../src/renderer/storage/storageNumbers";
import type { StorageNode } from "../src/renderer/storage/storageTypes";
const node = (id: string, amount: string): StorageNode => ({
  id, snapshot_id: "snapshot", revision: 1, parent_id: null, name: id, kind: "directory",
  allocated_bytes: amount, known_allocated_bytes: amount, logical_bytes: amount,
  ratio: { numerator_bytes: amount, denominator_bytes: null, reason: "root", provisional: false },
  state: "complete", issues: [], protection: [], classification: [],
});
describe("storage byte contract", () => {
  it("preserves integers beyond Number precision through sort, display and JSON", () => {
    const first = "9007199254740993", second = "9007199254740992";
    expect(compareBytes(first, second)).toBe(1);
    expect(compareBytes("9", "100")).toBe(-1);
    expect(JSON.parse(JSON.stringify({ amount: bytes(first).toString() })).amount).toBe(first);
    expect(formatBytes("9663676416")).toBe("9.0 GiB");
    expect(() => bytes("1e9")).toThrow();
  });
  it("uses the server parent denominator for both rounded label and bar", () => {
    const ratio = { numerator_bytes: "9663676416", denominator_bytes: "12884901888", reason: null, provisional: false };
    expect(formatRatio(ratio)).toBe("75.0%");
    expect(ratioWidth(ratio)).toBe(75);
    expect(formatRatio({ ...ratio, provisional: true })).toBe("75.0% · 잠정");
    expect(ratioWidth({ ...ratio, denominator_bytes: "0" })).toBeNull();
    expect(formatRatio({ ...ratio, denominator_bytes: null })).toContain("미확인");
  });
  it("keeps omitted confirmed bytes in the treemap instead of renormalizing a page", () => {
    const tiles = treemapTiles(node("root", "12"), [node("target", "9")]);
    expect(tiles).toHaveLength(2);
    expect(tiles.find(tile => tile.id === "target")!.width * tiles.find(tile => tile.id === "target")!.height).toBe(7500);
    expect(tiles.find(tile => tile.id === null)!.amount).toBe("3");
    expect(tiles.reduce((area, tile) => area + tile.width * tile.height, 0)).toBe(10000);
  });
  it("leaves zero and subpixel entries available in the table and rejects inconsistent snapshots", () => {
    const nodes = [node("tiny", "1"), node("zero", "0"), node("large", "9007199254740993")];
    const tiles = treemapTiles(node("root", "9007199254740994"), nodes);
    expect(nodes.map(item => item.id)).toContain("zero");
    expect(tiles.every(tile => Number.isFinite(tile.width) && Number.isFinite(tile.height))).toBe(true);
    expect(() => treemapTiles(node("root", "1"), [node("bad", "2")])).toThrow();
    expect(treemapTiles(node("root", "0"), [])).toEqual([]);
  });
});
