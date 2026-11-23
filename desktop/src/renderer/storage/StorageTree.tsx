import { useEffect, useMemo, useRef, type KeyboardEvent } from "react";
import type { StorageExplorer } from "./useStorageExplorer";
import type { StorageNode } from "./storageTypes";
import { completenessLabels } from "./storageTypes";
import { formatBytes, formatRatio, ratioWidth } from "./storageNumbers";

export function StorageTree({ model, focusRequest }: { model: StorageExplorer; focusRequest: number }) {
  const elements = useRef(new Map<string, HTMLDivElement>());
  const rows = useMemo(() => {
    const result: { node: StorageNode; depth: number }[] = [];
    const visited = new Set<string>();
    function visit(id: string, depth: number) {
      if (visited.has(id)) return;
      visited.add(id);
      const node = model.nodes.get(id);
      if (!node) return;
      result.push({ node, depth });
      if (model.expanded.has(id)) for (const child of model.pages.get(id)?.items ?? []) visit(child.id, depth + 1);
    }
    if (model.snapshot) visit(model.snapshot.root_node_id, 1);
    return result;
  }, [model.nodes, model.pages, model.expanded, model.snapshot]);
  const focus = (id: string) => { model.select(id); elements.current.get(id)?.focus(); };
  useEffect(() => {
    if (focusRequest && model.selectedId) elements.current.get(model.selectedId)?.focus();
  }, [focusRequest, model.selectedId, rows]);
  const keyDown = (event: KeyboardEvent, node: StorageNode, index: number) => {
    if (!["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
    event.preventDefault();
    if (event.key === "ArrowDown" && rows[index + 1]) focus(rows[index + 1]!.node.id);
    if (event.key === "ArrowUp" && rows[index - 1]) focus(rows[index - 1]!.node.id);
    if (event.key === "Home" && rows[0]) focus(rows[0].node.id);
    if (event.key === "End" && rows.at(-1)) focus(rows.at(-1)!.node.id);
    if (event.key === "ArrowLeft") {
      if (model.expanded.has(node.id)) model.toggle(node.id);
      else if (node.parent_id && elements.current.has(node.parent_id)) focus(node.parent_id);
    }
    if (event.key === "ArrowRight" && node.kind !== "file") {
      if (!model.expanded.has(node.id)) model.toggle(node.id);
      else if (rows[index + 1]?.depth === rows[index]!.depth + 1) focus(rows[index + 1]!.node.id);
    }
  };
  return <div className="storage-tree" role="treegrid" aria-label="폴더 점유율 표" aria-colcount={4}>
    <div role="row" className="storage-tree-header"><span role="columnheader">이름</span><span role="columnheader">할당량</span><span role="columnheader">부모 대비</span><span role="columnheader">상태</span></div>
    {rows.map(({ node, depth }, index) => {
      const width = ratioWidth(node.ratio), canExpand = node.kind !== "file";
      return <div key={node.id} role="row" aria-level={depth} aria-selected={model.selectedId === node.id}
        aria-expanded={canExpand ? model.expanded.has(node.id) : undefined}
        tabIndex={model.selectedId === node.id ? 0 : -1} data-node-id={node.id}
        className={model.selectedId === node.id ? "storage-tree-row selected" : "storage-tree-row"}
        ref={element => { if (element) elements.current.set(node.id, element); else elements.current.delete(node.id); }}
        onFocus={() => { if (model.selectedId !== node.id) model.select(node.id); }}
        onClick={() => focus(node.id)} onKeyDown={event => keyDown(event, node, index)}>
        <span role="gridcell" style={{ paddingInlineStart: `${(depth - 1) * 18}px` }}>
          {canExpand && <button type="button" tabIndex={-1} aria-label={`${node.name} ${model.expanded.has(node.id) ? "접기" : "펼치기"}`} onClick={event => { event.stopPropagation(); model.toggle(node.id); }}>{model.expanded.has(node.id) ? "▾" : "▸"}</button>}
          <span title={node.name}>{node.name}</span>
        </span>
        <span role="gridcell" title={`확인 할당량 ${node.known_allocated_bytes} B · 논리 크기 ${node.logical_bytes} B`}>{formatBytes(node.allocated_bytes ?? node.known_allocated_bytes)}{node.allocated_bytes === null ? " + 미확인" : ""}</span>
        <span role="gridcell" className="storage-ratio">{width !== null && <span className="storage-ratio-bar" style={{ width: `${width}%` }} aria-hidden="true" />}<span>{formatRatio(node.ratio)}</span></span>
        <span role="gridcell">{completenessLabels[node.state]}</span>
      </div>;
    })}
    {[...model.expanded].map(id => {
      const page = model.pages.get(id), parent = model.nodes.get(id);
      return page?.next_cursor && parent ? <button type="button" className="ghost" key={id} onClick={() => void model.loadChildren(id, page.next_cursor)}>{parent.name}의 다음 항목</button> : null;
    })}
  </div>;
}
