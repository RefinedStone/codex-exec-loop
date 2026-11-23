import "./storage.css";
import { useEffect, useRef, useState } from "react";
import type { RegisteredPanelProps } from "../extensions/registry";
import { setDraft, toast, useUiStore } from "../session/uiStore";
import { clearStorageEvidence, useStorageEvidence } from "./storageEvidence";
import { formatBytes, formatRatio } from "./storageNumbers";
import { completenessLabels, isActiveJob, jobLabels, type SharePreview, type StorageEvidence, type StorageGrant, type StorageHotspot, type StorageNode, type StoragePage } from "./storageTypes";
import { sameSnapshot, useStorageExplorer } from "./useStorageExplorer";
import { StorageTree } from "./StorageTree";
import { StorageLiveTree } from "./StorageLiveTree";
import { StorageTreemap } from "./StorageTreemap";
import { StorageProtection } from "./StorageProtection";
import { prepareStorageNavigation, type StorageNavigation } from "./storageNavigation";

export default function StoragePanel({ call, epoch, scopeKey, threadId, enabled }: RegisteredPanelProps) {
  const model = useStorageExplorer(call, epoch, scopeKey, enabled);
  const selected = model.selectedId ? model.nodes.get(model.selectedId) ?? null : null;
  const job = model.jobs[0] ?? null;
  const progress = job?.progress && typeof job.progress === "object" ? job.progress as { files_scanned?: number; directories_scanned?: number; volume_id?: string } : null;
  const [mode, setMode] = useState<"tree" | "summary">("tree");
  const [showMap, setShowMap] = useState(false);
  const [files, setFiles] = useState<StoragePage | null>(null);
  const [summary, setSummary] = useState<StoragePage<StorageHotspot> | null>(null);
  const [nameFilter, setNameFilter] = useState("");
  const [extensionFilter, setExtensionFilter] = useState("");
  const [recursive, setRecursive] = useState(false);
  const [preview, setPreview] = useState<SharePreview | null>(null);
  const [grant, setGrant] = useState<StorageGrant | null>(null);
  const [sharing, setSharing] = useState(false);
  const [focusRequest, setFocusRequest] = useState(0);
  const evidence = useStorageEvidence();
  const handledEvidence = useRef(0);
  const navigationRequest = useRef(0);
  const navigationRevision = useUiStore(state => state.navigationRevision);
  const snapshotKey = model.snapshot ? model.snapshot.id + ":" + model.snapshot.revision : "";
  const shareLifetime = useRef({ scopeKey, threadId, selectedId: model.selectedId, snapshotKey, navigationRevision });
  if (shareLifetime.current.scopeKey !== scopeKey || shareLifetime.current.threadId !== threadId || shareLifetime.current.selectedId !== model.selectedId
    || shareLifetime.current.snapshotKey !== snapshotKey || shareLifetime.current.navigationRevision !== navigationRevision) {
    shareLifetime.current = { scopeKey, threadId, selectedId: model.selectedId, snapshotKey, navigationRevision };
  }
  useEffect(() => () => { shareLifetime.current = { ...shareLifetime.current }; }, []);
  const fileSequence = useRef(0);
  const summarySequence = useRef(0);

  useEffect(() => { setPreview(null); setSharing(false); }, [scopeKey, threadId, model.selectedId, snapshotKey, navigationRevision]);
  useEffect(() => { setGrant(null); }, [scopeKey, threadId]);
  useEffect(() => {
    if (!selected || !model.snapshot || selected.kind === "file") { setFiles(null); return; }
    const sequence = ++fileSequence.current, basis = model.snapshot, life = model.scope.current;
    setFiles(null);
    void call<StoragePage>("list_files", { snapshot_id: basis.id, node_id: selected.id, limit: 100, sort: "allocated_desc", name_filter: nameFilter || null, extension_filter: extensionFilter || null, recursive }).then(page => {
      if (sequence === fileSequence.current && model.scope.current === life && sameSnapshot(page, basis)) setFiles(page);
    }).catch(error => { if (sequence === fileSequence.current && model.scope.current === life) model.setError(String(error)); });
    return () => { fileSequence.current++; };
  }, [call, model.snapshot, selected?.id, nameFilter, extensionFilter, recursive]);
  useEffect(() => {
    if (selected && selected.kind !== "file" && !model.pages.has(selected.id)) void model.loadChildren(selected.id);
  }, [selected?.id, model.snapshot, model.loadChildren]);
  useEffect(() => {
    if (mode !== "summary" || !model.snapshot) return;
    const sequence = ++summarySequence.current, basis = model.snapshot, life = model.scope.current;
    setSummary(null);
    void call<StoragePage<StorageHotspot>>("find_hotspots", { snapshot_id: basis.id, node_id: basis.root_node_id, limit: 100,
      thresholds: { absolute_bytes: "1073741824", relative_min_bytes: "268435456", parent_percent: 20 } }).then(page => {
      if (sequence === summarySequence.current && model.scope.current === life && sameSnapshot(page, basis)) setSummary(page);
    }).catch(error => { if (sequence === summarySequence.current && model.scope.current === life) model.setError(String(error)); });
    return () => { summarySequence.current++; };
  }, [mode, model.snapshot, call]);

  const moreFiles = async () => {
    const basis = model.snapshot, life = model.scope.current, current = files, id = selected?.id, sequence = fileSequence.current;
    if (!basis || !current?.next_cursor || !id) return;
    try {
      const page = await call<StoragePage>("list_files", { snapshot_id: basis.id, node_id: id, cursor: current.next_cursor, limit: 100, sort: "allocated_desc", name_filter: nameFilter || null, extension_filter: extensionFilter || null, recursive });
      if (sequence === fileSequence.current && model.scope.current === life && sameSnapshot(page, basis)) setFiles(previous => previous?.next_cursor === current.next_cursor ? { ...page, items: [...current.items, ...page.items.filter(item => !current.items.some(old => old.id === item.id))] } : previous);
    } catch (error) { if (sequence === fileSequence.current && model.scope.current === life) model.setError(String(error)); }
  };
  const moreSummary = async () => {
    const basis = model.snapshot, life = model.scope.current, current = summary, sequence = summarySequence.current;
    if (!basis || !current?.next_cursor) return;
    try {
      const page = await call<StoragePage<StorageHotspot>>("find_hotspots", { snapshot_id: basis.id, node_id: basis.root_node_id, cursor: current.next_cursor, limit: 100,
        thresholds: { absolute_bytes: "1073741824", relative_min_bytes: "268435456", parent_percent: 20 } });
      if (sequence === summarySequence.current && model.scope.current === life && sameSnapshot(page, basis)) setSummary(previous => previous?.next_cursor === current.next_cursor ? { ...page, items: [...current.items, ...page.items.filter(item => !current.items.some(old => old.node.id === item.node.id && old.role === item.role))] } : previous);
    } catch (error) { if (sequence === summarySequence.current && model.scope.current === life) model.setError(String(error)); }
  };

  const applyNavigation = ({ chain, pages }: StorageNavigation) => {
    model.setNodes(previous => {
      const next = new Map(previous);
      for (const node of chain) next.set(node.id, node);
      for (const page of pages.values()) for (const node of page.items) next.set(node.id, node);
      return next;
    });
    model.setPages(previous => {
      const next = new Map(previous);
      for (const [id, page] of pages) {
        const old = next.get(id);
        const extra = old?.snapshot_id === page.snapshot_id && old.revision === page.revision ? old.items.filter(node => !page.items.some(item => item.id === node.id)) : [];
        next.set(id, { ...page, items: [...page.items, ...extra], next_cursor: extra.length ? old!.next_cursor : page.next_cursor });
      }
      return next;
    });
    model.setRevealedPath(chain.map(node => node.id));
    model.setExpanded(previous => new Set([...previous, ...chain.filter(node => node.kind !== "file").map(node => node.id)]));
    model.setSelectedId(chain.at(-1)!.id);
    setMode("tree"); setFocusRequest(value => value + 1); model.setError("");
  };
  const selectNode = async (node: StorageNode) => {
    const basis = model.snapshot, life = model.scope.current;
    if (!basis || !sameSnapshot(node, basis)) return;
    const request = ++navigationRequest.current, interaction = ++model.interaction.current;
    const valid = () => navigationRequest.current === request && model.scope.current === life && model.interaction.current === interaction
      && model.currentSnapshot.current?.id === basis.id && model.currentSnapshot.current.revision === basis.revision;
    try {
      const navigation = await prepareStorageNavigation(call, basis, node.id, valid);
      if (navigation && valid()) applyNavigation(navigation);
    } catch (error) { if (valid()) model.setError(String(error)); }
  };

  const previewShare = async () => {
    const life = shareLifetime.current, basis = model.snapshot;
    if (!selected || !threadId || !basis) return;
    setSharing(true); model.setError("");
    try {
      const value = await call<SharePreview>("share_preview", { snapshot_id: basis.id, node_id: selected.id, thread_id: threadId });
      if (shareLifetime.current === life && value.thread_id === threadId && value.node_id === selected.id && sameSnapshot(value, basis)) setPreview(value);
    } catch (error) { if (shareLifetime.current === life) model.setError(String(error)); }
    finally { if (shareLifetime.current === life) setSharing(false); }
  };
  const createShare = async () => {
    const life = shareLifetime.current;
    if (!preview || !threadId || preview.thread_id !== threadId) return;
    setSharing(true);
    try {
      const value = await call<StorageGrant>("share_create", { preview_id: preview.id, thread_id: threadId });
      if (shareLifetime.current !== life || value.thread_id !== threadId) {
        void call("share_revoke", { grant_id: value.id }).catch(() => undefined);
        return;
      }
      const existing = useUiStore.getState().drafts[threadId] ?? "";
      setDraft(threadId, `${existing}${existing ? "\n\n" : ""}${value.prompt}`);
      setGrant(value); setPreview(null);
      toast("선택한 대화의 입력창에 공유 참조를 넣었습니다. 내용을 확인하고 보내세요.");
    } catch (error) { if (shareLifetime.current === life) model.setError(String(error)); }
    finally { if (shareLifetime.current === life) setSharing(false); }
  };
  const revokeShare = async () => {
    const current = grant, life = shareLifetime.current;
    if (!current) return;
    setSharing(true);
    try { await call("share_revoke", { grant_id: current.id }); if (shareLifetime.current === life) { setGrant(null); toast("이 참조의 새 조회를 막았습니다. 이미 보낸 대화는 유지됩니다."); } }
    catch (error) { if (shareLifetime.current === life) model.setError(String(error)); }
    finally { if (shareLifetime.current === life) setSharing(false); }
  };

  useEffect(() => {
    if (!evidence || evidence.id === handledEvidence.current || !model.snapshot) return;
    handledEvidence.current = evidence.id;
    clearStorageEvidence(evidence.id);
    const request = ++navigationRequest.current;
    const life = model.scope.current, interaction = model.interaction.current, basis = model.snapshot;
    const valid = () => navigationRequest.current === request && handledEvidence.current === evidence.id && model.scope.current === life && model.interaction.current === interaction
      && model.currentSnapshot.current?.id === basis.id && model.currentSnapshot.current.revision === basis.revision
      && useUiStore.getState().navigationRevision === evidence.navigationRevision
      && shareLifetime.current.threadId === evidence.threadId && shareLifetime.current.scopeKey === scopeKey;
    if (threadId !== evidence.threadId || !valid()) return;
    void (async () => {
      const result = await call<StorageEvidence>("resolve_evidence", { reference: evidence.reference, thread_id: threadId });
      if (!valid()) return;
      if (!sameSnapshot(result, basis)) throw new Error("이 근거는 이전 분석 결과입니다. 현재 결과를 다시 공유하세요.");
      const navigation = await prepareStorageNavigation(call, basis, result.node_id, valid);
      if (!navigation || !valid()) return;
      // 준비하는 동안 철회되었거나 참조의 근거가 바뀌었으면 선택을 옮기지 않는다.
      const checked = await call<StorageEvidence>("resolve_evidence", { reference: evidence.reference, thread_id: threadId });
      if (!valid()) return;
      if (!sameSnapshot(checked, basis) || checked.node_id !== result.node_id) throw new Error("근거의 분석 결과가 바뀌었습니다.");
      applyNavigation(navigation);
    })().catch(error => { if (valid()) model.setError(String(error)); });
  }, [evidence, model.snapshot, call, scopeKey, threadId]);

  const reveal = async () => {
    const life = shareLifetime.current, basis = model.snapshot;
    if (!selected || !basis) return;
    try {
      const result = await call<{ path: string; host_id: string; snapshot_id: string; node_id: string }>("reveal_path", { snapshot_id: basis.id, node_id: selected.id });
      if (shareLifetime.current !== life) return;
      if (result.snapshot_id !== basis.id || result.node_id !== selected.id) throw new Error("탐색기로 열 항목이 현재 선택과 다릅니다.");
      await window.akra.revealStoragePath({ path: result.path, host: "local" });
    } catch (error) { if (shareLifetime.current === life) model.setError(String(error)); }
  };

  return <section className="storage-panel" aria-label="저장 공간 분석기">
    <header><h2>저장 공간</h2><p className="muted">이 PC · 현재 사용자. 분석 결과만 읽고 파일을 변경하지 않습니다.</p></header>
    {!enabled && <p role="status">확장이 꺼졌거나 끄는 중입니다. 설정에서 현재 상태를 확인하세요.</p>}
    {model.error && <div className="storage-error" role="alert">{model.error}<button type="button" onClick={() => { model.setError(""); void model.refresh(); }}>상태 다시 확인</button></div>}
    <div className="storage-controls">
      <label>드라이브<select aria-label="분석할 드라이브" value={model.volumeId} onChange={event => model.setVolumeId(event.target.value)} disabled={!enabled || model.busy || isActiveJob(job)}>
        {!model.volumes.length && <option value="">드라이브 확인 중</option>}
        {model.volumes.map(volume => <option key={volume.id} value={volume.id} disabled={!volume.supported}>{volume.label} · {volume.root_path}{!volume.supported ? " · 지원 안 함" : ""}</option>)}
      </select></label>
      <button type="button" onClick={() => void model.scan()} disabled={!enabled || model.busy || !model.volumeId || isActiveJob(job)}>{model.busy ? "확인 중…" : model.snapshot ? "다시 분석" : "분석"}</button>
      {isActiveJob(job) && <button type="button" disabled={model.busy || job?.state === "cancelling"} onClick={() => void model.cancel()}>공유 분석 취소</button>}
    </div>
    <div className="storage-progress" aria-live="polite" role="status">
      <strong>{job ? jobLabels[job.state] : "분석을 누르면 시작합니다."}</strong>
      {model.snapshot && <span> · {completenessLabels[model.snapshot.state]} · 파일 {model.snapshot.files_scanned.toLocaleString()}개 · 폴더 {model.snapshot.directories_scanned.toLocaleString()}개</span>}
      {!model.snapshot && progress && <span> · 확인한 파일 {progress.files_scanned ?? 0}개 · 폴더 {progress.directories_scanned ?? 0}개</span>}
      {job?.error && <p>{job.error}</p>}
      {job?.state === "interrupted" && <p>이전 분석이 중단됐습니다. 다시 분석을 눌러 새 작업을 시작할 수 있습니다.</p>}
      {isActiveJob(job) && <p className="muted">다른 창도 같은 분석을 봅니다. 패널을 닫아도 분석은 계속되며, 취소하면 모든 창에 적용됩니다.</p>}
      {model.snapshot?.issues.map(issue => <p key={issue.code} className="muted">{issue.message} · {issue.count}개</p>)}
    </div>
    {!model.snapshot && job && isActiveJob(job) && <StorageLiveTree key={scopeKey + ":" + job.id} call={call} epoch={epoch} jobId={job.id}
      rootName={model.volumes.find(volume => volume.id === progress?.volume_id)?.label ?? "분석 중인 드라이브"} />}
    {model.snapshot && <>
      <nav className="segmented" aria-label="저장 공간 보기">
        <button type="button" aria-pressed={mode === "tree"} onClick={() => { model.interaction.current++; setMode("tree"); }}>폴더 점유율 표</button>
        <button type="button" aria-pressed={mode === "summary"} onClick={() => { model.interaction.current++; setMode("summary"); }}>큰 폴더 요약</button>
      </nav>
      {mode === "tree" ? <StorageTree model={model} focusRequest={focusRequest} /> : <div aria-label="큰 폴더 요약">
        {summary?.items.map((item, index) => <button type="button" key={`${item.node.id}:${item.role}:${index}`} className="storage-summary-row" onClick={() => void selectNode(item.node)}>
          <span>{item.node.name}{item.role === "remainder" ? " · 이 폴더의 나머지" : ""}</span><strong>{formatBytes(item.included_bytes)}{item.provisional ? " · 잠정" : ""}</strong><small>{item.reason}</small>
        </button>)}
        {summary?.next_cursor && <button type="button" onClick={() => void moreSummary()}>요약 다음 항목</button>}
      </div>}
      {selected && <section className="storage-details" aria-label="선택 항목 상세">
        <h3>{selected.name}</h3>
        <dl><dt>확인 할당량</dt><dd>{formatBytes(selected.known_allocated_bytes)} · {selected.known_allocated_bytes} B</dd>
          <dt>논리 크기</dt><dd>{formatBytes(selected.logical_bytes)} · {selected.logical_bytes} B</dd>
          <dt>직속 부모 대비</dt><dd>{formatRatio(selected.ratio)}{selected.ratio.denominator_bytes !== null && ` (분모 ${selected.ratio.denominator_bytes} B)`}</dd>
          <dt>집계 상태</dt><dd>{completenessLabels[selected.state]}</dd></dl>
        {selected.issues.map(issue => <p key={issue.code}>{issue.message} · {issue.count}개</p>)}
        {selected.protection.map((reason, index) => <StorageProtection key={index} reason={reason} />)}
        {selected.classification.map((item, index) => <p key={index} className="muted">{item.kind} · {item.evidence}</p>)}
        <div className="storage-controls">
          <button type="button" onClick={() => void reveal()} disabled={!enabled}>탐색기에서 보기</button>
          <button type="button" onClick={() => void previewShare()} disabled={!enabled || !threadId || sharing || selected.kind === "file"}>이 폴더를 AI에 묻기</button>
        </div>
        {!threadId && <p className="muted">대화를 연 뒤 선택한 폴더를 공유할 수 있습니다.</p>}
        {preview && <section className="storage-share" aria-label="폴더 공유 미리보기">
          <h4>선택한 대화에 공유</h4><p>대화: {preview.thread_id}</p><p>{preview.root.name} 아래의 이름과 아래 항목을 공유합니다. 상위 경로와 파일 본문은 보내지 않습니다.</p>
          <ul>{preview.shared_fields.map(field => <li key={field}>{field}</li>)}</ul>
          <details><summary>전송할 폴더 정보 전체 확인</summary><pre>{JSON.stringify(preview.root, null, 2)}</pre></details>
          <p className="muted">공유를 해제하면 새 조회를 막습니다. 이미 보낸 대화 내용은 회수되지 않습니다.</p>
          <button type="button" disabled={sharing} onClick={() => void createShare()}>대화 입력창에 공유</button>
          <button type="button" disabled={sharing} onClick={() => setPreview(null)}>취소</button>
        </section>}
        {grant && <p role="status">대화 입력창에 공유 참조를 넣었습니다. <button type="button" disabled={sharing} onClick={() => void revokeShare()}>공유 해제</button></p>}
        {selected.kind !== "file" && <>
          <label className="storage-map-toggle"><input type="checkbox" checked={showMap} onChange={event => setShowMap(event.target.checked)} />선택 폴더 트리맵 표시</label>
          {showMap && <StorageTreemap parent={selected} nodes={model.pages.get(selected.id)?.items ?? []} selectedId={model.selectedId} onSelect={node => void selectNode(node)} />}
          <h4>파일</h4><div className="storage-filters">
            <label>이름<input value={nameFilter} onChange={event => { model.interaction.current++; setNameFilter(event.target.value); }} aria-label="파일 이름 필터" /></label>
            <label>확장자<input value={extensionFilter} onChange={event => { model.interaction.current++; setExtensionFilter(event.target.value); }} aria-label="파일 확장자 필터" /></label>
            <label><input type="checkbox" checked={recursive} onChange={event => { model.interaction.current++; setRecursive(event.target.checked); }} />하위 파일 포함</label>
          </div>
          <table className="storage-files"><caption>선택 폴더의 파일 · {files?.total_count === null || files === null ? "전체 개수 미확인" : `${files.total_count}개`}</caption>
            <thead><tr><th>이름</th><th>할당량</th><th>논리 크기</th></tr></thead>
            <tbody>{files?.items.map(node => <tr key={node.id}><td><button type="button" onClick={() => void selectNode(node)}>{node.name}</button></td><td>{formatBytes(node.allocated_bytes)}</td><td>{formatBytes(node.logical_bytes)}</td></tr>)}</tbody>
          </table>
          {files?.next_cursor && <button type="button" onClick={() => void moreFiles()}>파일 다음 항목</button>}
        </>}
      </section>}
    </>}
  </section>;
}
