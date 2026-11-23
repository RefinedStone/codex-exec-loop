import { describe, expect, it } from "vitest";
import { prepareStorageNavigation, storageTreeRows, storageTreeTabStop, type StorageNavigationQuery } from "../src/renderer/storage/storageNavigation";
import { treemapTiles } from "../src/renderer/storage/storageNumbers";
import type { StorageNode, StoragePage, StorageSnapshot } from "../src/renderer/storage/storageTypes";

const snapshot: StorageSnapshot = { schema_version: 1, id: "snapshot", revision: 1, volume_id: "volume", root_node_id: "root",
  state: "complete", started_at: 1, finished_at: 2, files_scanned: 1, directories_scanned: 2, issues: [] };
const node = (id: string, parent_id: string | null, kind: StorageNode["kind"], amount: string): StorageNode => ({
  snapshot_id: snapshot.id, revision: 1, id, parent_id, kind, name: id, allocated_bytes: amount, known_allocated_bytes: amount, logical_bytes: amount,
  ratio: { numerator_bytes: amount, denominator_bytes: parent_id ? "9" : null, reason: parent_id ? null : "root", provisional: false },
  state: "complete", issues: [], protection: [], classification: [],
});
const root = node("root", null, "directory", "9"), folder = node("folder", "root", "directory", "9");
const bucket = node("bucket", "folder", "direct_files", "9"), file = node("file", "folder", "file", "8");
const page = (items: StorageNode[]): StoragePage => ({ snapshot_id: snapshot.id, revision: 1, items, total_count: items.length, next_cursor: null });
const chain = [root, folder, file];

describe("canonical storage selection", () => {
  it("reveals the file row without adding its bytes to a directory or treemap page", async () => {
    const requests: { action: string; input: unknown }[] = [];
    const call: StorageNavigationQuery = async <T,>(action: string, input?: unknown) => {
      requests.push({ action, input });
      const id = (input as { node_id: string }).node_id;
      return (action === "ancestors" ? { snapshot_id: snapshot.id, node_id: file.id, ancestors: chain } : page(id === "root" ? [folder] : [bucket])) as T;
    };
    const navigation = (await prepareStorageNavigation(call, snapshot, file.id, () => true))!;
    expect(requests[0]).toEqual({ action: "ancestors", input: { snapshot_id: "snapshot", node_id: "file" } });
    expect(navigation.pages.get("folder")!.items).toEqual([bucket]);
    const nodes = new Map([...chain, bucket].map(item => [item.id, item]));
    const rows = storageTreeRows("root", nodes, navigation.pages, new Set(["root", "folder"]), chain.map(item => item.id));
    expect(rows.map(row => [row.node.id, row.depth, row.detailOnly])).toEqual([
      ["root", 1, false], ["folder", 2, false], ["bucket", 3, false], ["file", 3, true],
    ]);
    expect(storageTreeTabStop(rows, nodes, "file")).toBe("file");
    const tiles = treemapTiles(folder, navigation.pages.get("folder")!.items);
    expect(tiles.map(tile => [tile.id, tile.amount])).toEqual([["bucket", "9"]]);
  });

  it("retains one reachable tree tab stop when the selected descendant is collapsed", () => {
    const nodes = new Map([...chain, bucket].map(item => [item.id, item]));
    const pages = new Map([["root", page([folder])], ["folder", page([bucket])]]);
    const rows = storageTreeRows("root", nodes, pages, new Set(["root"]), chain.map(item => item.id));
    expect(rows.map(row => row.node.id)).toEqual(["root", "folder"]);
    expect(storageTreeTabStop(rows, nodes, "file")).toBe("folder");
    expect(storageTreeTabStop(rows, nodes, "gone")).toBe("root");
  });

  it("drops a late path before it schedules child reads", async () => {
    let resolve!: (value: unknown) => void, current = true;
    const requests: string[] = [];
    const call: StorageNavigationQuery = <T,>(action: string) => { requests.push(action); return new Promise<T>(done => { resolve = value => done(value as T); }); };
    const pending = prepareStorageNavigation(call, snapshot, file.id, () => current);
    current = false;
    resolve({ snapshot_id: snapshot.id, node_id: file.id, ancestors: chain });
    expect(await pending).toBeNull();
    expect(requests).toEqual(["ancestors"]);
  });

  it("rejects a broken ancestry or a page from another observation", async () => {
    const broken: StorageNavigationQuery = async <T,>() => ({ snapshot_id: snapshot.id, node_id: file.id, ancestors: [root, file] }) as T;
    await expect(prepareStorageNavigation(broken, snapshot, file.id, () => true)).rejects.toThrow("경로");
    const stale: StorageNavigationQuery = async <T,>(action: string) => (action === "ancestors"
      ? { snapshot_id: snapshot.id, node_id: file.id, ancestors: chain } : { ...page([]), revision: 2 }) as T;
    await expect(prepareStorageNavigation(stale, snapshot, file.id, () => true)).rejects.toThrow("바뀌었습니다");
  });
});
