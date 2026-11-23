import type { StorageNode, StoragePage, StorageSnapshot } from "./storageTypes";

export type StorageNavigationQuery = <T>(action: string, input: unknown) => Promise<T>;

export interface StorageNavigation {
  chain: StorageNode[];
  pages: Map<string, StoragePage>;
}
const matches = (value: { snapshot_id: string; revision: number }, basis: StorageSnapshot) => value.snapshot_id === basis.id && value.revision === basis.revision;

/** 선택 경로만 추가로 읽는다. 파일 상세를 집계 페이지에 섞지 않는다. */
export async function prepareStorageNavigation(call: StorageNavigationQuery, basis: StorageSnapshot, nodeId: string, current: () => boolean): Promise<StorageNavigation | null> {
  const result = await call<{ snapshot_id: string; node_id: string; ancestors: StorageNode[] }>("ancestors", { snapshot_id: basis.id, node_id: nodeId });
  if (!current()) return null;
  const chain = result.ancestors;
  if (result.snapshot_id !== basis.id || result.node_id !== nodeId || !chain.length || chain[0]!.id !== basis.root_node_id
    || chain.at(-1)!.id !== nodeId || new Set(chain.map(node => node.id)).size !== chain.length
    || chain.some((node, index) => !matches(node, basis) || index > 0 && node.parent_id !== chain[index - 1]!.id)) {
    throw new Error("선택한 항목의 분석 경로가 일치하지 않습니다.");
  }
  const pages = await Promise.all(chain.filter(node => node.kind === "directory").map(async parent => {
    const page = await call<StoragePage>("list_children", { snapshot_id: basis.id, node_id: parent.id, limit: 100, sort: "allocated_desc" });
    if (!matches(page, basis) || page.items.some(node => !matches(node, basis) || node.parent_id !== parent.id)) {
      throw new Error("탐색 중 분석 결과가 바뀌었습니다.");
    }
    return [parent.id, page] as const;
  }));
  return current() ? { chain, pages: new Map(pages) } : null;
}

export interface StorageTreeRow { node: StorageNode; depth: number; detailOnly: boolean; }
/** 집계 children과 선택 경로를 화면에서만 합친다. 원본 pages는 treemap/분모에 그대로 쓴다. */
export function storageTreeRows(rootId: string | undefined, nodes: ReadonlyMap<string, StorageNode>, pages: ReadonlyMap<string, StoragePage>, expanded: ReadonlySet<string>, revealedPath: readonly string[]): StorageTreeRow[] {
  const result: StorageTreeRow[] = [], visited = new Set<string>();
  const visit = (id: string, depth: number) => {
    const node = nodes.get(id);
    if (!node || visited.has(id)) return;
    visited.add(id); result.push({ node, depth, detailOnly: node.kind === "file" });
    if (!expanded.has(id)) return;
    const children = new Set(pages.get(id)?.items.map(child => child.id) ?? []);
    for (const reveal of revealedPath) if (nodes.get(reveal)?.parent_id === id) children.add(reveal);
    for (const child of children) visit(child, depth + 1);
  };
  if (rootId) visit(rootId, 1);
  return result;
}

export function storageTreeTabStop(rows: readonly StorageTreeRow[], nodes: ReadonlyMap<string, StorageNode>, selectedId: string | null): string | undefined {
  const visible = new Set(rows.map(row => row.node.id)), visited = new Set<string>();
  let id = selectedId;
  while (id && !visited.has(id)) {
    if (visible.has(id)) return id;
    visited.add(id); id = nodes.get(id)?.parent_id ?? null;
  }
  return rows[0]?.node.id;
}
