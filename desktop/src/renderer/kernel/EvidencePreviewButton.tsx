import { useState } from "react";
import { useDesktopStore } from "../session/desktopStore";
import type { EvidencePreview } from "../../shared/evidencePreview";

export function EvidencePreviewButton({ target }: { readonly target: Omit<EvidencePreview, "workspace"> }) {
  const workspace = useDesktopStore((state) => state.kernel.phase === "attached" ? state.kernel.workspace : null);
  const [status, setStatus] = useState<"idle" | "pending" | "failed">("idle");
  const open = window.akra.openEvidencePreview;
  if (!open || !workspace) return null;
  return <><button type="button" disabled={status === "pending"} onClick={async () => {
    setStatus("pending");
    try { await open({ workspace, ...target }); setStatus("idle"); }
    catch { setStatus("failed"); }
  }}>정제된 증거 열기</button>
    {status === "failed" && <span role="status">증거를 열 수 없습니다. 현재 기준을 다시 조회하세요.</span>}</>;
}
