import { useMemo } from "react";
import type { StorageNode } from "./storageTypes";
import { formatBytes, treemapTiles } from "./storageNumbers";
export function StorageTreemap({ parent, nodes, selectedId, onSelect }: { parent: StorageNode; nodes: StorageNode[]; selectedId: string | null; onSelect(node: StorageNode): void }) {
  const result = useMemo(() => { try { return { tiles: treemapTiles(parent, nodes), error: "" }; } catch (error) { return { tiles: [], error: String(error) }; } }, [parent, nodes]);
  if (result.error) return <p role="status">{result.error}</p>;
  if (!result.tiles.length) return <p className="muted">면적을 표시할 확인 용량이 없습니다. 표에서 모든 항목을 확인할 수 있습니다.</p>;
  return <section aria-label="선택 폴더 트리맵">
    <p className="muted">{parent.name} · {formatBytes(parent.known_allocated_bytes)} 확인됨{parent.state !== "complete" ? " · 일부 확인량의 잠정 면적" : ""}. 나머지는 아직 표시하지 않은 확인량입니다.</p>
    <div className="storage-treemap">
      {result.tiles.map(tile => {
        const node = nodes.find(item => item.id === tile.id);
        const style = { left: `${tile.x}%`, top: `${tile.y}%`, width: `${tile.width}%`, height: `${tile.height}%` };
        return node ? <button type="button" key={tile.id} style={style} className={selectedId === tile.id ? "selected" : ""} aria-label={`${tile.name}, ${formatBytes(tile.amount)}, 표에서 선택`} title={`${tile.name} · ${formatBytes(tile.amount)}`} onClick={() => onSelect(node)}>{tile.name}<small>{formatBytes(tile.amount)}</small></button>
          : <div key="remainder" style={style} title={`나머지 확인량 · ${formatBytes(tile.amount)}`}>나머지 확인량<small>{formatBytes(tile.amount)}</small></div>;
      })}
    </div>
  </section>;
}
