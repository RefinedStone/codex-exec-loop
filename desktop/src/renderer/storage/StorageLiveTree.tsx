import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import type { ExtensionCall } from "../extensions/repositoryExtensions";

interface LiveDirectory { id: string; parent_id: string | null; name: string; enumeration_complete: boolean; }
interface LivePage {
  job_id: string; revision: 0; root_node_id: string; node_id: string; items: LiveDirectory[];
  total_count: number | null; next_cursor: string | null; enumeration_complete: boolean;
}
const ROOT_ID = "n1";

/** 진행 중 발견 목록은 확정 snapshot과 별도이다. 용량·비율·공유를 만들지 않는다. */
export function StorageLiveTree({ call, epoch, jobId, rootName }: {
  call: ExtensionCall; epoch: number; jobId: string; rootName: string;
}) {
  const [pages, setPages] = useState(new Map<string, LivePage>());
  const [expanded, setExpanded] = useState(new Set([ROOT_ID]));
  const [selectedId, setSelectedId] = useState(ROOT_ID);
  const [error, setError] = useState("");
  const [prioritizing, setPrioritizing] = useState(new Set<string>());
  const alive = useRef(true);
  const sequence = useRef(new Map<string, number>());
  const expansion = useRef(expanded);
  expansion.current = expanded;
  const elements = useRef(new Map<string, HTMLDivElement>());

  useEffect(() => { alive.current = true; return () => { alive.current = false; }; }, []);
  const load = useCallback(async (nodeId: string, cursor: string | null = null) => {
    const request = (sequence.current.get(nodeId) ?? 0) + 1;
    sequence.current.set(nodeId, request);
    try {
      const result = await call<LivePage>("live_children", { job_id: jobId, node_id: nodeId, cursor, limit: 100 });
      if (!alive.current || sequence.current.get(nodeId) !== request) return;
      if (result.job_id !== jobId || result.revision !== 0 || result.node_id !== nodeId || result.root_node_id !== ROOT_ID) {
        throw new Error("진행 중 폴더 목록의 작업이 일치하지 않습니다.");
      }
      setPages(previous => {
        const next = new Map(previous), old = next.get(nodeId);
        const items = cursor && old ? [...old.items, ...result.items.filter(item => !old.items.some(value => value.id === item.id))]
          : [...result.items, ...(old?.items ?? []).filter(item => !result.items.some(value => value.id === item.id))];
        // 새 발견 알림으로 첫 페이지를 다시 읽어도 이미 연 뒤쪽 항목과 선택은 유지한다.
        const nextCursor = !cursor && old && old.items.length > result.items.length ? old.next_cursor ?? result.next_cursor : result.next_cursor;
        next.set(nodeId, { ...result, items, next_cursor: nextCursor });
        return next;
      });
    } catch (failure) {
      if (alive.current && sequence.current.get(nodeId) === request) setError(String(failure));
    }
  }, [call, jobId]);

  useEffect(() => {
    for (const id of expansion.current) void load(id);
  }, [load, epoch]);

  const rows = useMemo(() => {
    const nodes = new Map<string, LiveDirectory>([[ROOT_ID, {
      id: ROOT_ID, parent_id: null, name: rootName, enumeration_complete: pages.get(ROOT_ID)?.enumeration_complete ?? false,
    }]]);
    for (const page of pages.values()) for (const node of page.items) nodes.set(node.id, {
      ...node, enumeration_complete: pages.get(node.id)?.enumeration_complete ?? node.enumeration_complete,
    });
    const result: { node: LiveDirectory; depth: number }[] = [], visited = new Set<string>();
    const visit = (id: string, depth: number) => {
      const node = nodes.get(id);
      if (!node || visited.has(id)) return;
      visited.add(id); result.push({ node, depth });
      if (expanded.has(id)) for (const child of pages.get(id)?.items ?? []) visit(child.id, depth + 1);
    };
    visit(ROOT_ID, 1);
    return result;
  }, [pages, expanded, rootName]);

  const toggle = (id: string) => {
    if (expanded.has(id)) { setExpanded(previous => { const next = new Set(previous); next.delete(id); return next; }); return; }
    setError("");
    setExpanded(previous => new Set(previous).add(id));
    void load(id);
    setPrioritizing(previous => new Set(previous).add(id));
    void call<{ job_id: string; node_id: string; queued: boolean; queue_length: number }>("prioritize", { job_id: jobId, node_id: id })
      .catch(failure => { if (alive.current) setError(String(failure)); })
      .finally(() => { if (alive.current) setPrioritizing(previous => { const next = new Set(previous); next.delete(id); return next; }); });
  };
  const focus = (id: string) => { setSelectedId(id); elements.current.get(id)?.focus(); };
  const keyDown = (event: KeyboardEvent, index: number) => {
    const node = rows[index]!.node;
    if (!["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
    event.preventDefault();
    if (event.key === "ArrowDown" && rows[index + 1]) focus(rows[index + 1]!.node.id);
    if (event.key === "ArrowUp" && rows[index - 1]) focus(rows[index - 1]!.node.id);
    if (event.key === "Home") focus(ROOT_ID);
    if (event.key === "End" && rows.at(-1)) focus(rows.at(-1)!.node.id);
    if (event.key === "ArrowLeft") {
      if (expanded.has(node.id)) toggle(node.id);
      else if (node.parent_id) focus(node.parent_id);
    }
    if (event.key === "ArrowRight") {
      if (!expanded.has(node.id)) toggle(node.id);
      else if (rows[index + 1]?.depth === rows[index]!.depth + 1) focus(rows[index + 1]!.node.id);
    }
  };

  return <section aria-label="분석 중 폴더 탐색">
    <h3>발견한 폴더</h3>
    <p className="muted">폴더를 펼치면 먼저 분석하도록 요청합니다. 분석이 끝나면 확정 용량 표로 바뀝니다.</p>
    <p className="muted">아래 목록은 아직 집계 중이며 용량·비율·AI 공유를 제공하지 않습니다.</p>
    {error && <p role="alert">{error}</p>}
    <div className="storage-tree storage-live-tree" role="treegrid" aria-label="분석 중 발견한 폴더" aria-colcount={2}>
      <div role="row" className="storage-tree-header"><span role="columnheader">폴더</span><span role="columnheader">발견 상태</span></div>
      {rows.map(({ node, depth }, index) => <div key={node.id} role="row" aria-level={depth} aria-expanded={expanded.has(node.id)}
        aria-selected={selectedId === node.id} tabIndex={selectedId === node.id ? 0 : -1} data-live-node-id={node.id}
        className={selectedId === node.id ? "storage-tree-row selected" : "storage-tree-row"}
        ref={element => { if (element) elements.current.set(node.id, element); else elements.current.delete(node.id); }}
        onFocus={() => setSelectedId(node.id)} onClick={() => focus(node.id)} onKeyDown={event => keyDown(event, index)}>
        <span role="gridcell" style={{ paddingInlineStart: (depth - 1) * 18 }}>
          <button type="button" tabIndex={-1} aria-label={node.name + (expanded.has(node.id) ? " 접기" : " 펼치기")}
            onClick={event => { event.stopPropagation(); toggle(node.id); }}>{expanded.has(node.id) ? "▾" : "▸"}</button>
          <span title={node.name}>{node.name}</span>
        </span>
        <span role="gridcell">{prioritizing.has(node.id) ? "우선 분석 요청 중" : node.enumeration_complete ? "직속 폴더 발견 완료 · 집계 중" : "폴더 발견 중 · 집계 중"}</span>
      </div>)}
    </div>
    {[...expanded].map(id => {
      const page = pages.get(id), name = rows.find(row => row.node.id === id)?.node.name;
      return page?.next_cursor ? <button type="button" key={id} onClick={() => void load(id, page.next_cursor)}>{name ?? "폴더"}의 다음 발견 항목</button> : null;
    })}
  </section>;
}
