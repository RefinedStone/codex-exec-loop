import { execFile } from "node:child_process";
import { isAbsolute } from "node:path";
import { type BrowserWindow, dialog } from "electron";
import { isEvidencePreview } from "../shared/evidencePreview";

/** Private pipe, then a modal. No artifact bytes return to renderer or diagnostics. */
export async function openEvidencePreview(owner: BrowserWindow, executable: string, input: unknown): Promise<void> {
  if (!isEvidencePreview(input) || !isAbsolute(input.workspace) || !isAbsolute(executable)) throw unavailable();
  const { workspace, ...target } = input;
  const text = await new Promise<string>((resolve, reject) => {
    const child = execFile(executable, ["desktop-evidence-preview"], {
      cwd: workspace, encoding: "utf8", timeout: 10_000, maxBuffer: 64 * 1024, windowsHide: true,
    }, (error, stdout) => error ? reject(unavailable()) : resolve(stdout));
    child.stdin?.on("error", () => reject(unavailable()));
    child.stdin?.end(JSON.stringify(target));
  });
  if (owner.isDestroyed() || !owner.isVisible()) throw unavailable();
  await dialog.showMessageBox(owner, { type: "info", title: "완료 기준 증거", message: "정제된 증거 (읽기 전용)",
    detail: text, buttons: ["닫기"], noLink: true });
}

function unavailable(): Error { return new Error("정제된 증거를 열 수 없습니다."); }
