import { useLayoutEffect, useMemo, useRef, useState } from "react";
import type { StorageNode } from "./storageTypes";
import { formatBytes, treemapTiles } from "./storageNumbers";
export function StorageTreemap({ parent, nodes, selectedId, onSelect }: { parent: StorageNode; nodes: StorageNode[]; selectedId: string | null; onSelect(node: StorageNode): void }) {
  const result = useMemo(() => { try { return { tiles: treemapTiles(parent, nodes), error: "" }; } catch (error) { return { tiles: [], error: String(error) }; } }, [parent, nodes]);
  const container = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ width: 0, height: 0 });
  const hasTiles = result.tiles.length > 0;
  useLayoutEffect(() => {
    const element = container.current;
    if (!element) return;
    const update = (width: number, height: number) => setSize(old => old.width === width && old.height === height ? old : { width, height });
    update(element.clientWidth, element.clientHeight);
    const observer = new ResizeObserver(entries => {
      const rect = entries[0]?.contentRect;
      if (rect) update(rect.width, rect.height);
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, [hasTiles]);
  if (result.error) return <p role="status">{result.error}</p>;
  if (!hasTiles) return <p className="muted">면적을 표시할 확인 용량이 없습니다. 표에서 모든 항목을 확인할 수 있습니다.</p>;
  return <section aria-label="선택 폴더 트리맵">
    <p className="muted">{parent.name} · {formatBytes(parent.known_allocated_bytes)} 확인됨{parent.state !== "complete" ? " · 일부 확인량의 잠정 면적" : ""}. 나머지는 아직 표시하지 않은 확인량입니다. 아주 작은 항목은 표에서 확인하세요.</p>
    <div className="storage-treemap" ref={container}>
      {result.tiles.map(tile => {
        const node = nodes.find(item => item.id === tile.id);
        const style = { left: `${tile.x}%`, top: `${tile.y}%`, width: `${tile.width}%`, height: `${tile.height}%` };
        const visible = tile.width * size.width / 100 >= 1 && tile.height * size.height / 100 >= 1;
        if (!visible) return <div key={tile.id ?? "remainder"} style={style} className="storage-treemap-tiny" aria-hidden="true" />;
        return node ? <button type="button" key={tile.id} style={style} className={selectedId === tile.id ? "selected" : ""} aria-label={`${tile.name}, ${formatBytes(tile.amount)}, 표에서 선택`} title={`${tile.name} · ${formatBytes(tile.amount)}`} onClick={() => onSelect(node)}>{tile.name}<small>{formatBytes(tile.amount)}</small></button>
          : <div key="remainder" style={style} title={`나머지 확인량 · ${formatBytes(tile.amount)}`}>나머지 확인량<small>{formatBytes(tile.amount)}</small></div>;
      })}
    </div>
  </section>;
}
