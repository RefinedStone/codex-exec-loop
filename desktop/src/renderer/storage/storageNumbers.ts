import type { StorageNode, StorageRatio } from "./storageTypes";
export function bytes(value: string): bigint {
  if (!/^(0|[1-9][0-9]*)$/.test(value)) throw new Error("용량 값의 형식이 맞지 않습니다.");
  return BigInt(value);
}
export function compareBytes(left: string, right: string): number {
  const a = bytes(left), b = bytes(right); return a < b ? -1 : a > b ? 1 : 0;
}
export function formatBytes(value: string | null): string {
  if (value === null) return "미확인";
  const amount = bytes(value);
  const units = ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];
  let divisor = 1n, index = 0;
  while (amount >= divisor * 1024n && index < units.length - 1) { divisor *= 1024n; index++; }
  if (!index) return `${amount} B`;
  const tenths = (amount * 10n + divisor / 2n) / divisor;
  return `${tenths / 10n}.${tenths % 10n} ${units[index]}`;
}
export function ratioTenths(ratio: StorageRatio): bigint | null {
  if (ratio.numerator_bytes === null || ratio.denominator_bytes === null) return null;
  const denominator = bytes(ratio.denominator_bytes);
  return denominator === 0n ? null : (bytes(ratio.numerator_bytes) * 1000n + denominator / 2n) / denominator;
}
export function formatRatio(ratio: StorageRatio): string {
  const tenths = ratioTenths(ratio);
  if (tenths === null) return ratio.reason === "root" ? "— 보기의 루트" : ratio.reason === "zero_denominator" ? "— 부모가 0 B" : "— 비율 미확인";
  return `${tenths / 10n}.${tenths % 10n}%${ratio.provisional ? " · 잠정" : ""}`;
}
export function ratioWidth(ratio: StorageRatio): number | null {
  const tenths = ratioTenths(ratio);
  return tenths === null ? null : Number(tenths > 1000n ? 1000n : tenths) / 10;
}
export interface TreemapTile { id: string | null; name: string; amount: string; x: number; y: number; width: number; height: number; }
/** 화면 좌표 직전에만 비율을 제한된 실수로 바꾼다. 페이지의 합으로 분모를 바꾸지 않는다. */
export function treemapTiles(parent: StorageNode, children: readonly StorageNode[]): TreemapTile[] {
  const total = bytes(parent.known_allocated_bytes);
  if (total === 0n || parent.state === "pending") return [];
  const weighted = children.map(node => ({ id: node.id as string | null, name: node.name, amount: bytes(node.known_allocated_bytes) })).filter(node => node.amount > 0n);
  const shown = weighted.reduce((sum, node) => sum + node.amount, 0n);
  if (shown > total) throw new Error("자식 용량이 현재 부모 스냅숏과 맞지 않습니다.");
  if (shown < total) weighted.push({ id: null, name: "나머지 확인량", amount: total - shown });
  const output: TreemapTile[] = [];
  function layout(nodes: typeof weighted, x: number, y: number, width: number, height: number) {
    if (!nodes.length) return;
    if (nodes.length === 1) { const node = nodes[0]!; output.push({ id: node.id, name: node.name, amount: node.amount.toString(), x, y, width, height }); return; }
    const sum = nodes.reduce((value, node) => value + node.amount, 0n);
    let first = nodes[0]!.amount, split = 1;
    while (split < nodes.length - 1 && first * 2n < sum) first += nodes[split++]!.amount;
    const share = Number(first * 1_000_000n / sum) / 1_000_000;
    if (width >= height) {
      layout(nodes.slice(0, split), x, y, width * share, height);
      layout(nodes.slice(split), x + width * share, y, width * (1 - share), height);
    } else {
      layout(nodes.slice(0, split), x, y, width, height * share);
      layout(nodes.slice(split), x, y + height * share, width, height * (1 - share));
    }
  }
  layout(weighted, 0, 0, 100, 100);
  return output;
}
