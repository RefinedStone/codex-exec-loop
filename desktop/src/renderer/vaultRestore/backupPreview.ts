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
  const projects = document.createElement("select"); projects.setAttribute("aria-label", "백업 원본 프로젝트"); projects.hidden = true;
  const note = document.createElement("p"); note.textContent = "v1 백업과 복구 키를 함께 가진 사람은 원래 장치 키를 얻을 수 있습니다. 파일을 검사해도 가져오거나 저장하지 않습니다.";
  const save = document.createElement("button"); save.type = "button"; save.textContent = "선택한 이슈만 v2 백업";
  const exported = document.createElement("p"); exported.setAttribute("role", "status");
  save.onclick = async () => {
    save.disabled = true;
    exported.textContent = "";
    try {
      const response = await bridge.exportBackup(generation);
      if (!section.isConnected) return;
      switch (response.status) {
        case "saved":
          exported.textContent = `복구 키는 지금 한 번만 표시합니다. 백업 파일과 별도로 보관하세요: ${response.recoveryKey}`;
          break;
        case "cancelled":
          exported.textContent = "백업이 취소되었습니다. 저장이 이미 시작되었다면 파일이 남을 수 있습니다.";
          break;
        case "rejected":
          exported.textContent = "이 이슈의 백업을 만들 수 없습니다.";
          break;
        case "save_failed":
          exported.textContent = "새 파일을 저장하지 못했습니다. 기존 파일은 덮어쓰지 않았습니다.";
          break;
      }
    } catch (error) {
      if (!(error instanceof Error)) throw error;
      exported.textContent = "이 이슈의 백업을 만들 수 없습니다.";
    } finally {
      save.disabled = false;
    }
  };
  cancel.onclick = () => { key.value = ""; file.value = ""; bridge.lock(); };
  verify.onclick = async () => {
    const chosen = file.files?.[0];
    if (!chosen || !chosen.size || chosen.size > 8*1024*1024 || !key.value) { key.value=""; report.textContent="파일과 복구 키를 확인하세요."; return; }
    verify.disabled = true;
    projects.hidden = true;
    projects.replaceChildren();
    const secret = key.value; key.value = ""; file.value = "";
    try {
      const data = new Uint8Array(await chosen.arrayBuffer());
      if (!section.isConnected) return;
      const response = await bridge.previewBackup(generation, data, secret);
      if (!section.isConnected) return;
      if (response.status === "rejected") {
        report.textContent = "파일 또는 키를 확인할 수 없습니다. 변경한 항목 없음";
        return;
      }
      report.textContent = [
        response.version === 1 ? "v1: 원래 장치 키를 포함하는 이전 형식" : "v2: 선택한 레코드 전용 키",
        "현재 게시본과 일치하는지 확인하지 않았습니다. 이 백업과 대상 프로젝트에 대한 별도 복구 승인이 없어 가져올 수 없습니다. 변경한 항목 없음",
      ].join("\n");
      const placeholder = new Option("검사할 원본 프로젝트를 선택하세요", "", true, true);
      placeholder.disabled = true;
      projects.replaceChildren(placeholder, ...response.projects.map((project) => new Option(project, project)));
      projects.hidden = response.projects.length === 0;
    } catch { report.textContent = "검사 실패. 변경한 항목 없음"; }
    finally { verify.disabled = false; }
  };
  section.append(save, exported, note, file, key, verify, cancel, report, projects); return section;
}
