import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { writeFile } from "node:fs/promises";
import { dialog, type BrowserWindow } from "electron";
import type { VaultBackupExport, VaultRestoreTarget } from "../shared/vaultRestore";

const MAX_ARCHIVE_BYTES = 8 * 1024 * 1024;
const MAX_REPLY_BYTES = 3 * MAX_ARCHIVE_BYTES;
type ExportedArchive = { readonly archive: string; readonly recoveryKey: string };
export type BackupExportSource = (target: VaultRestoreTarget, signal: AbortSignal) => Promise<ExportedArchive | null>;

export async function saveVaultBackup(
  request: { readonly owner: BrowserWindow; readonly target: VaultRestoreTarget; readonly signal: AbortSignal },
  source: BackupExportSource,
  active: () => boolean,
): Promise<VaultBackupExport> {
  try {
    const choice = await dialog.showSaveDialog(request.owner, {
      title: "선택한 이슈의 Vault 백업 저장",
      defaultPath: "akra-vault-v2.json",
      filters: [{ name: "Vault backup", extensions: ["json"] }],
    });
    if (choice.canceled || !choice.filePath || !active()) return { status: "cancelled" };
    const result = await source(request.target, request.signal);
    if (!active()) return { status: "cancelled" };
    if (!result) return { status: "rejected" };
    try {
      await writeFile(choice.filePath, result.archive, { flag: "wx", mode: 0o600 });
    } catch (error) {
      if (!(error instanceof Error)) throw error;
      return { status: "save_failed" };
    }
    return active() ? { status: "saved", recoveryKey: result.recoveryKey } : { status: "cancelled" };
  } catch (error) {
    if (!(error instanceof Error)) throw error;
    return { status: "rejected" };
  }
}

function parseReply(value: unknown): ExportedArchive | null {
  if (typeof value !== "object" || !value || !("status" in value) || value.status !== "exported" ||
      !("archive" in value) || typeof value.archive !== "string" || Buffer.byteLength(value.archive) > MAX_ARCHIVE_BYTES ||
      !("recovery_key" in value) || typeof value.recovery_key !== "string" || !/^[a-f0-9]{8}(?:-[a-f0-9]{8}){7}$/i.test(value.recovery_key)) return null;
  // An old/incompatible helper must not publish an archive using the device key.
  const archive: unknown = JSON.parse(value.archive);
  if (typeof archive !== "object" || !archive || !("formatVersion" in archive) || archive.formatVersion !== 2) return null;
  return { archive: value.archive, recoveryKey: value.recovery_key };
}

export function exportBackup(executable: string, target: VaultRestoreTarget, signal: AbortSignal): Promise<ExportedArchive | null> {
  if (signal.aborted) return Promise.resolve(null);
  return new Promise((resolve) => {
    const child = spawn(executable, ["desktop-backup-export"], { stdio: ["pipe", "pipe", "ignore"], windowsHide: true });
    const chunks: Buffer[] = [];
    let size = 0;
    let complete = false;
    let closed = false;
    const stop = () => {
      child.stdin.destroy(); // EOF cancels the native confirmation.
      settle(null);
    };
    const settle = (result: ExportedArchive | null) => {
      if (closed) return;
      closed = true;
      clearTimeout(timer);
      signal.removeEventListener("abort", stop);
      for (const chunk of chunks) chunk.fill(0);
      chunks.length = 0;
      resolve(result);
    };
    const timer = setTimeout(stop, 130_000);
    signal.addEventListener("abort", stop, { once: true });
    child.once("error", stop);
    child.stdin.once("error", stop);
    child.stdout.once("error", stop);
    child.stdout.on("data", (chunk: Buffer) => {
      size += chunk.length;
      if (closed || signal.aborted || complete || size > MAX_REPLY_BYTES) {
        chunk.fill(0);
        stop();
        return;
      }
      chunks.push(chunk);
      const newline = chunk.indexOf(10);
      if (newline < 0) return;
      if (newline !== chunk.length - 1) return stop();
      complete = true;
      child.stdin.end();
    });
    child.once("close", (code) => {
      if (code !== 0 || !complete || signal.aborted || chunks.length === 0) return settle(null);
      const bytes = Buffer.concat(chunks, size);
      try {
        const result: unknown = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes));
        settle(parseReply(result));
      } catch (error) {
        if (!(error instanceof SyntaxError || error instanceof TypeError)) throw error;
        settle(null);
      } finally {
        bytes.fill(0);
      }
    });
    child.stdin.write(JSON.stringify({ version: 2, id: randomUUID(), command: "prepare", issue_url: target.issueUrl, issue: target.issue }) + "\n");
  });
}
