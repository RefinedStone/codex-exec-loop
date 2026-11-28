import { useState } from "react";
import { useDesktopStore } from "../session/desktopStore";
import type { EvidencePreview } from "../../shared/evidencePreview";

export function EvidencePreviewButton({ target, reviewable = false }: { readonly target: Omit<EvidencePreview, "workspace">; readonly reviewable?: boolean }) {
  const workspace = useDesktopStore((state) => state.kernel.phase === "attached" ? state.kernel.workspace : null);
  const [status, setStatus] = useState<"idle" | "pending" | "failed">("idle");
  const open = globalThis.window?.akra?.openEvidencePreview;
  if (!open || !workspace) return null;
  const show = async (review?: "pass" | "fail") => {
    setStatus("pending");
    try { await open({ workspace, ...target }, review); setStatus("idle"); }
    catch { setStatus("failed"); }
  };
  return <><button type="button" disabled={status === "pending"} onClick={() => void show()}>정제된 증거 열기</button>
    {reviewable && <><button type="button" disabled={status === "pending"} onClick={() => void show("pass")}>증거 보고 통과 확인</button>
      <button type="button" disabled={status === "pending"} onClick={() => void show("fail")}>증거 보고 실패 확인</button></>}
    {status === "failed" && <span role="status">증거를 열 수 없습니다. 현재 기준을 다시 조회하세요.</span>}</>;
}
