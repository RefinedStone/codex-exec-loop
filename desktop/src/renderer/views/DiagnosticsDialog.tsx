import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import type { DiagnosticPreview } from "../../shared/diagnostics";
import { Icon } from "../design/icons";
import { useDesktopStore } from "../session/desktopStore";
import { useUiStore } from "../session/uiStore";
import "./diagnostics.css";

const close = () => useUiStore.setState({ overlay: null });
const LOG_LABEL = { collected: "제한된 로그 수집됨", absent: "로그 없음", empty: "해당 로그 없음", unreadable: "로그 읽기 실패" };

/** 미리보기와 실제 저장 바이트는 main이 같은 ID로 묶는다. 닫으면 미리보기 권한도 버린다. */
export function DiagnosticsDialog() {
  const [preview, setPreview] = useState<DiagnosticPreview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  useEffect(() => {
    let active = true;
    let previewId: string | null = null;
    const gateway = useDesktopStore.getState().gateway;
    void window.akra.diagnostics.preview({ gatewayVersion: gateway?.gatewayVersion, codexVersion: gateway?.codexVersion }).then(value => {
      previewId = value.id;
      if (active) setPreview(value);
      else void window.akra.diagnostics.discard(value.id).catch(() => undefined);
    }).catch(() => { if (active) setError("진단 묶음을 준비하지 못했습니다. 닫은 뒤 다시 열어 주세요."); });
    return () => {
      active = false;
      if (previewId) void window.akra.diagnostics.discard(previewId).catch(() => undefined);
    };
  }, []);

  async function save() {
    if (!preview || saving) return;
    setSaving(true); setResult(null);
    try {
      const answer = await window.akra.diagnostics.save(preview.id);
      setResult(answer.status === "saved" ? "검토한 내용 그대로 로컬 파일에 저장했습니다."
        : answer.status === "cancelled" ? "저장을 취소했습니다. 파일을 만들지 않았습니다."
        : answer.reason === "expired" ? "미리보기가 만료되었습니다. 닫은 뒤 다시 열어 주세요."
        : answer.reason === "busy" ? "이미 저장 창이 열려 있습니다."
        : "파일을 저장하지 못했습니다. 다른 위치를 골라 다시 시도하세요.");
    } catch { setResult("파일을 저장하지 못했습니다. 다른 위치를 골라 다시 시도하세요."); }
    finally { setSaving(false); }
  }

  return createPortal(
    <div className="overlay diagnostics-overlay" onClick={() => !saving && close()}>
      <section className="dialog diagnostics" role="dialog" aria-modal="true" aria-label="로컬 연결 진단" data-testid="diagnostics-dialog" onClick={event => event.stopPropagation()}>
        <header><strong>로컬 연결 진단</strong><button type="button" className="icon-button" aria-label="닫기" onClick={close} disabled={saving}><Icon name="close" /></button></header>
        <p>수집한 항목과 저장할 내용을 확인하세요. 선택한 로컬 파일에만 저장합니다.</p>
        {error && <p role="alert">{error}</p>}
        {!preview && !error && <p role="status">진단 묶음을 준비하는 중…</p>}
        {preview && <>
          <p data-testid="diagnostics-status">{preview.offline ? "Gateway 오프라인 · " : "Gateway 연결됨 · "}{LOG_LABEL[preview.logStatus]}{preview.truncated ? " · 수집 한도 적용됨" : ""}</p>
          <div className="diagnostics-inventory">
            <section><h4>수집</h4><ul>{preview.collected.map(item => <li key={item}>{item}</li>)}</ul></section>
            <section><h4>제외</h4><ul>{preview.excluded.map(item => <li key={item}>{item}</li>)}</ul></section>
            <section><h4>누락·미관측</h4><ul>{preview.missing.map(item => <li key={item}>{item}</li>)}</ul></section>
          </div>
          <label htmlFor="diagnostics-content">저장할 정제 사본</label>
          <textarea id="diagnostics-content" className="diagnostics-content" data-testid="diagnostics-content" aria-label="저장할 정제 사본" readOnly value={preview.content} />
          <p className="muted">로그는 최근 64 KiB에서 최대 40건의 시각·등급·구분·UUID 상관 ID만 포함합니다. Gateway·Codex 버전은 이 창이 마지막으로 관측한 값입니다.</p>
          <button type="button" className="chip-button" data-testid="diagnostics-save" onClick={() => void save()} disabled={saving}>{saving ? "저장하는 중…" : "검토한 내용 저장…"}</button>
        </>}
        {result && <p role="status" data-testid="diagnostics-result">{result}</p>}
      </section>
    </div>, document.body,
  );
}
