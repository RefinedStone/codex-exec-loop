import type { VaultRestoreBridge } from "../../shared/bridge";

/** This form exists only in the private vault window. Never persist a selection, key or result. */
export function archivePreview(bridge: VaultRestoreBridge, generation: number): HTMLElement {
  const section = document.createElement("section");
  section.setAttribute("aria-label", "백업 파일 검사");
  const file = document.createElement("input"); file.type = "file"; file.setAttribute("aria-label", "검사할 백업 파일");
  const key = document.createElement("input"); key.type = "password"; key.autocomplete = "off"; key.maxLength = 128; key.spellcheck = false; key.setAttribute("aria-label", "백업 복구 키");
  const verify = document.createElement("button"); verify.type = "button"; verify.textContent = "백업 파일 검사 (읽기 전용)";
  const cancel = document.createElement("button"); cancel.type = "button"; cancel.textContent = "검사 취소";
  const report = document.createElement("p"); report.setAttribute("role", "status");
  const note = document.createElement("p"); note.textContent = "v1 백업과 복구 키를 함께 가진 사람은 원래 장치 키를 얻을 수 있습니다. 파일을 검사해도 가져오거나 저장하지 않습니다.";
  cancel.onclick = () => { key.value = ""; file.value = ""; bridge.lock(); };
  verify.onclick = async () => {
    const chosen = file.files?.[0];
    if (!chosen || !chosen.size || chosen.size > 8*1024*1024 || !key.value) { key.value=""; report.textContent="파일과 복구 키를 확인하세요."; return; }
    verify.disabled = true;
    const secret = key.value; key.value = ""; file.value = "";
    try {
      const data = new Uint8Array(await chosen.arrayBuffer());
      if (!section.isConnected) return;
      const response = await bridge.previewBackup(generation, data, secret);
      if (!section.isConnected) return;
      report.textContent = response.status === "verified" ? [response.version === 1 ? "v1: 원래 장치 키를 포함하는 이전 형식" : "v2: 선택한 레코드 전용 키", ...response.projects, "변경한 항목 없음"].join("\n") : "파일 또는 키를 확인할 수 없습니다. 변경한 항목 없음";
    } catch { report.textContent = "검사 실패. 변경한 항목 없음"; }
    finally { verify.disabled = false; }
  };
  section.append(note, file, key, verify, cancel, report); return section;
}
