import { spawn } from "node:child_process";
import type { VaultBackupPreview } from "../shared/vaultRestore";

/** No paths or keys in argv. Never relay helper stderr or invalid output to a client. */
export function inspectBackup(executable: string, archive: Uint8Array, secret: string, signal: AbortSignal): Promise<VaultBackupPreview> {
  if (signal.aborted || archive.byteLength === 0 || archive.byteLength > 8 * 1024 * 1024 || secret.length > 128 || !/^[0-9a-fA-F -]+$/.test(secret)) return Promise.resolve({status:"rejected"});
  return new Promise(resolve => {
    const child = spawn(executable, ["desktop-backup-preview"], {stdio:["pipe","pipe","ignore"],windowsHide:true});
    const chunks: Buffer[] = [];
    let size = 0;
    const stop = () => { child.kill(); resolve({status:"rejected"}); };
    const timer = setTimeout(stop, 15_000);
    signal.addEventListener("abort", stop, {once:true});
    child.once("error", stop);
    child.stdin.once("error", stop);
    child.stdout.once("error", stop);
    child.stdout.on("data", (chunk: Buffer) => { size += chunk.length; if (size > 256 * 1024) stop(); else chunks.push(chunk); });
    child.once("close", code => {
      clearTimeout(timer);
      signal.removeEventListener("abort",stop);
      if (code !== 0 || signal.aborted || size > 256 * 1024) return resolve({status:"rejected"});
      try {
        const value: unknown = JSON.parse(Buffer.concat(chunks).toString("utf8"));
        if (value && typeof value === "object" && "status" in value && value.status === "verified" && "version" in value && (value.version === 1 || value.version === 2) && "projects" in value && Array.isArray(value.projects) && value.projects.length <= 4096 && value.projects.every((p: unknown) => typeof p === "string" && p.length <= 4096)) {
          resolve({status:"verified", version:value.version, projects:value.projects});
        } else resolve({status:"rejected"});
      } catch { resolve({status:"rejected"}); }
    });
    const key = Buffer.from(secret + "\n", "utf8");
    child.stdin.write(key, () => key.fill(0));
    child.stdin.end(archive);
  });
}
