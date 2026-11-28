import { useEffect, useState } from "react";
import { useDesktopStore } from "../session/desktopStore";
import type { GatewayRequest } from "../work/notionHub";
import { readWorkspaceInventory, workspaceRecoveryReason, type WorkspaceInventoryPage } from "../session/workspaceInventory";

export function WorkspaceRecovery({ cwd, request }: { readonly cwd: string; readonly request: GatewayRequest }) {
  const connection = useDesktopStore(state => state.connection);
  const [opened, setOpened] = useState(false);
  const [revision, setRevision] = useState(0);
  const [after, setAfter] = useState<string | null>(null);
  const [page, setPage] = useState<WorkspaceInventoryPage | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    setPage(null);
    setError(null);
    if (!opened) return;
    let current = true;
    void readWorkspaceInventory(request, { cwd, after }).then(
      result => { if (current) setPage(result); },
      failure => { if (current) setError(failure instanceof Error ? failure.message : "격리 공간 목록을 읽지 못했습니다."); },
    );
    return () => { current = false; };
  }, [cwd, request, opened, after, revision, connection]);
  return <section data-testid="workspace-recovery">
    <button type="button" className="button" aria-expanded={opened} onClick={() => setOpened(value => !value)}>격리 공간 복구 목록</button>
    {opened && <>
      <p className="muted">저장된 준비 기록입니다. 대화 소유권과 원격 보관 상태가 확인되지 않은 공간은 그대로 보존합니다.</p>
      <button type="button" className="button" onClick={() => { setAfter(null); setRevision(value => value + 1); }}>처음부터 새로 읽기</button>
      {error && <p role="alert">{error}</p>}
      {!error && !page && <p role="status">격리 공간 확인 중…</p>}
      {page && <>
        {page.items.length === 0 && <p role="status">저장된 격리 공간이 없습니다.</p>}
        {page.items.map(item => <article key={item.record.requestId} className="fleet-card" data-testid="workspace-recovery-entry">
          <strong>{item.record.requestId}</strong>
          <span style={{ overflowWrap: "anywhere" }}>{item.record.workspaceDir}</span>
          <span style={{ overflowWrap: "anywhere" }}>{item.record.branch}</span>
          <span className="muted">준비 상태: {item.record.state} · 정리 보류</span>
          <span className="muted">현재 HEAD: {item.currentHead ?? "확인 불가"}</span>
          {item.record.reason && <p>{workspaceRecoveryReason(item.record.reason)}</p>}
          <ul>{item.reasons.map(reason => <li key={reason}>{workspaceRecoveryReason(reason)}</li>)}</ul>
        </article>)}
        {page.nextAfter && <button type="button" className="button" onClick={() => setAfter(page.nextAfter)}>다음 공간</button>}
      </>}
    </>}
  </section>;
}
