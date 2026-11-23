import { useCallback, useEffect, useRef, useState } from "react";
import type { ExtensionCall } from "../extensions/repositoryExtensions";
import { STORAGE_EXTENSION, isActiveJob, type ExtensionJob, type StorageNode, type StoragePage, type StorageSnapshot, type StorageVolume } from "./storageTypes";

export function sameSnapshot(value: { snapshot_id: string; revision: number }, snapshot: StorageSnapshot): boolean {
  return value.snapshot_id === snapshot.id && value.revision === snapshot.revision;
}

export function useStorageExplorer(call: ExtensionCall, epoch: number, scopeKey: string, enabled: boolean) {
  const [volumes, setVolumes] = useState<StorageVolume[]>([]);
  const [jobs, setJobs] = useState<ExtensionJob[]>([]);
  const [snapshot, setSnapshot] = useState<StorageSnapshot | null>(null);
  const [nodes, setNodes] = useState(new Map<string, StorageNode>());
  const [pages, setPages] = useState(new Map<string, StoragePage>());
  const [expanded, setExpanded] = useState(new Set<string>());
  const [revealedPath, setRevealedPath] = useState<string[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [volumeId, setVolumeId] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(true);
  const restoring = loading && !snapshot;
  const scope = useRef({ key: scopeKey, enabled, epoch: 0 });
  if (scope.current.key !== scopeKey || scope.current.enabled !== enabled) scope.current = { key: scopeKey, enabled, epoch: scope.current.epoch + 1 };
  const currentSnapshot = useRef(snapshot);
  currentSnapshot.current = snapshot;
  const selection = useRef(selectedId);
  selection.current = selectedId;
  const currentJobId = useRef<string | null>(null);
  const expansion = useRef(expanded);
  expansion.current = expanded;
  const childRequests = useRef(new Map<string, number>());
  const refreshSequence = useRef(0);
  const interaction = useRef(0);

  const acceptPage = useCallback((page: StoragePage, basis: StorageSnapshot, append: boolean) => {
    if (!sameSnapshot(page, basis) || currentSnapshot.current?.id !== basis.id || currentSnapshot.current.revision !== basis.revision) return false;
    setNodes(previous => { const next = new Map(previous); for (const item of page.items) if (sameSnapshot(item, basis)) next.set(item.id, item); return next; });
    const parent = page.items[0]?.parent_id;
    if (parent) setPages(previous => {
      const next = new Map(previous), old = next.get(parent);
      const items = append && old ? [...old.items, ...page.items.filter(item => !old.items.some(existing => existing.id === item.id))] : page.items;
      next.set(parent, { ...page, items }); return next;
    });
    return true;
  }, []);

  const loadChildren = useCallback(async (nodeId: string, cursor?: string | null) => {
    const basis = currentSnapshot.current, life = scope.current;
    if (!basis || !life.enabled) return;
    const request = (childRequests.current.get(nodeId) ?? 0) + 1;
    childRequests.current.set(nodeId, request);
    try {
      const page = await call<StoragePage>("list_children", { snapshot_id: basis.id, node_id: nodeId, limit: 100, cursor: cursor ?? null, sort: "allocated_desc" });
      if (scope.current !== life || childRequests.current.get(nodeId) !== request || !sameSnapshot(page, basis) || currentSnapshot.current?.id !== basis.id || currentSnapshot.current.revision !== basis.revision) return;
      acceptPage(page, basis, !!cursor);
      setPages(previous => {
        const next = new Map(previous), old = previous.get(nodeId);
        next.set(nodeId, { ...page, items: cursor && old ? [...old.items, ...page.items.filter(item => !old.items.some(existing => existing.id === item.id))] : page.items });
        return next;
      });
    } catch (failure) {
      if (scope.current === life && childRequests.current.get(nodeId) === request && currentSnapshot.current?.id === basis.id && currentSnapshot.current.revision === basis.revision) setError(String(failure));
    }
  }, [call, acceptPage]);

  const refresh = useCallback(async () => {
    const life = scope.current, sequence = ++refreshSequence.current;
    if (!enabled) return;
    setLoading(true);
    try {
      const [listed, result] = await Promise.all([
        call<StorageVolume[] | { volumes: StorageVolume[] }>("volumes"),
        call<{ jobs: ExtensionJob[] }>("jobs"),
      ]);
      if (scope.current !== life || sequence !== refreshSequence.current) return;
      const available = Array.isArray(listed) ? listed : listed.volumes;
      setVolumes(available);
      setVolumeId(previous => previous || available.find(item => item.supported)?.id || "");
      const currentJobs = result.jobs.filter(job => job.extension === STORAGE_EXTENSION && job.action === "scan").sort((a, b) => b.created_at_ms - a.created_at_ms || b.sequence - a.sequence);
      setJobs(currentJobs);
      if (!currentJobs.length) {
        currentJobId.current = null; currentSnapshot.current = null;
        setSnapshot(null); setNodes(new Map()); setPages(new Map()); setExpanded(new Set()); setSelectedId(null); setRevealedPath([]); return;
      }
      const latest = currentJobs[0]!;
      const progress = latest.progress && typeof latest.progress === "object" ? latest.progress as { volume_id?: unknown } : null;
      if (isActiveJob(latest) && typeof progress?.volume_id === "string" && progress.volume_id) setVolumeId(progress.volume_id);
      if (currentJobId.current !== latest.id) {
        currentJobId.current = latest.id; currentSnapshot.current = null; interaction.current++;
        if (latest.result) setVolumeId(latest.result.volume_id);
        setSnapshot(null); setNodes(new Map()); setPages(new Map()); setExpanded(new Set()); setSelectedId(null); setRevealedPath([]);
      }
      if (!latest.result) {
        currentSnapshot.current = null; setSnapshot(null); setNodes(new Map()); setPages(new Map()); setExpanded(new Set()); setSelectedId(null); setRevealedPath([]); return;
      }
      const next = await call<StorageSnapshot | null>("snapshot", { snapshot_id: latest.result.id })
        .catch(error => { if (String(error).includes("snapshot_pending")) return null; throw error; });
      if (scope.current !== life || sequence !== refreshSequence.current || !next) return;
      const old = currentSnapshot.current;
      const changed = old?.id !== next.id || old.revision !== next.revision;
      if (!changed) return;
      const previousSelection = old?.id === next.id ? selection.current : null;
      if (old?.id !== next.id) setRevealedPath([]);
      const nextExpanded = old?.id === next.id ? new Set([...expansion.current, next.root_node_id]) : new Set([next.root_node_id]);
      const [root, selected] = await Promise.all([
        call<StorageNode>("get_node", { snapshot_id: next.id, node_id: next.root_node_id }),
        previousSelection && previousSelection !== next.root_node_id
          ? call<StorageNode>("get_node", { snapshot_id: next.id, node_id: previousSelection }).catch(() => null) : Promise.resolve(null),
      ]);
      if (scope.current !== life || sequence !== refreshSequence.current || !sameSnapshot(root, next)) return;
      // Bind the initial/restored result once. Later refreshes keep the user's next-scan choice.
      if (old?.id !== next.id) setVolumeId(next.volume_id);
      currentSnapshot.current = next;
      setSnapshot(next);
      const nextNodes = new Map([[root.id, root]]);
      if (selected && sameSnapshot(selected, next)) nextNodes.set(selected.id, selected);
      setNodes(nextNodes); setPages(new Map());
      setSelectedId(selected?.id ?? root.id);
      setExpanded(nextExpanded);
      for (const id of nextExpanded) void loadChildren(id);
    } catch (failure) { if (scope.current === life && sequence === refreshSequence.current) setError(String(failure)); }
    finally { if (scope.current === life && sequence === refreshSequence.current) setLoading(false); }
  }, [call, enabled, loadChildren]);

  useEffect(() => {
    setVolumes([]); setVolumeId(""); setJobs([]); setSnapshot(null); currentSnapshot.current = null; currentJobId.current = null; setBusy(false); setLoading(true);
    setNodes(new Map()); setPages(new Map()); setExpanded(new Set()); setSelectedId(null); setRevealedPath([]); setError("");
    interaction.current++;
  }, [scopeKey]);
  useEffect(() => { void refresh(); return () => { refreshSequence.current++; }; }, [refresh, epoch]);
  useEffect(() => () => { scope.current = { ...scope.current, enabled: false, epoch: scope.current.epoch + 1 }; }, []);

  const scan = async () => {
    const life = scope.current;
    if (!enabled || busy || restoring || !volumes.some(volume => volume.id === volumeId && volume.supported)) return;
    setBusy(true); setError(""); interaction.current++;
    try {
      await call<ExtensionJob>("scan", { volume_id: volumeId, options: {} }, crypto.randomUUID());
      if (scope.current === life) await refresh();
    } catch (failure) {
      if (scope.current === life) { setError(`분석 접수 결과를 확인하세요: ${String(failure)}`); await refresh(); }
    } finally { if (scope.current === life) setBusy(false); }
  };
  const cancel = async () => {
    const life = scope.current, job = jobs[0];
    if (!job || busy) return;
    setBusy(true); setError("");
    try { await call("cancel", { job_id: job.id }); if (scope.current === life) await refresh(); }
    catch (failure) { if (scope.current === life) setError(String(failure)); }
    finally { if (scope.current === life) setBusy(false); }
  };
  const select = (id: string) => { interaction.current++; setSelectedId(id); };
  const toggle = (id: string) => {
    interaction.current++;
    setExpanded(previous => {
      const next = new Set(previous);
      if (next.has(id)) next.delete(id); else { next.add(id); if (!pages.has(id)) void loadChildren(id); }
      return next;
    });
  };
  return { call, scope, interaction, volumes, jobs, snapshot, nodes, pages, expanded, selectedId, volumeId,
    setVolumeId, error, setError, busy, loading, restoring, scan, cancel, refresh, loadChildren, select, toggle,
    setNodes, setPages, setExpanded, setSelectedId, revealedPath, setRevealedPath, currentSnapshot };
}
export type StorageExplorer = ReturnType<typeof useStorageExplorer>;
