// 기존 Rust trace JSONL에서 제한된 메타데이터만 읽는다. 메시지/오류 원문은 정규식 마스킹 대신 버린다.
import { constants } from "node:fs";
import { lstat, open, opendir } from "node:fs/promises";
import { join } from "node:path";

export const DIAGNOSTIC_LOG_BYTES = 64 * 1024;
export const DIAGNOSTIC_LOG_ENTRIES = 40;
const MAX_DIRECTORY_ENTRIES = 256;
const TRACE_NAME = /^akra-trace\.jsonl\.\d{4}-\d{2}-\d{2}(?:\.\d{1,4})?$/;
const LEVELS = new Set(["TRACE", "DEBUG", "INFO", "WARN", "ERROR"]);
const TARGETS = new Map([
  ["akra_ade::adapter::inbound::gateway", "gateway"],
  ["akra_ade::adapter::inbound::gateway::session", "gateway_session"],
  ["akra_ade::adapter::inbound::gateway::listener", "gateway_listener"],
  ["akra_ade::adapter::inbound::gateway::restart", "gateway_restart"],
]);
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const TIMESTAMP = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{1,9})?(?:Z|[+-]\d{2}:\d{2})$/;

export interface DiagnosticLogEntry {
  timestamp: string | null;
  level: string;
  source: string;
  correlationId: string | null;
}

export interface DiagnosticLogs {
  status: "collected" | "absent" | "empty" | "unreadable";
  entries: DiagnosticLogEntry[];
  truncated: boolean;
  omittedLines: number;
}

/** 값의 허용 목록만 복사하므로 새 로그 필드나 합성 canary도 외부 사본에 들어가지 않는다. */
export function sanitizeDiagnosticLogLines(text: string, partialFirstLine = false): DiagnosticLogs {
  const lines = text.split(/\r?\n/);
  if (partialFirstLine) lines.shift();
  const entries: DiagnosticLogEntry[] = [];
  let omittedLines = 0;
  for (const line of lines) {
    if (!line.trim()) continue;
    let row: Record<string, unknown>;
    try {
      const value: unknown = JSON.parse(line);
      if (!value || typeof value !== "object" || Array.isArray(value)) { omittedLines++; continue; }
      row = value as Record<string, unknown>;
    } catch { omittedLines++; continue; }
    const source = typeof row.target === "string" ? TARGETS.get(row.target) : undefined;
    if (!source || typeof row.level !== "string" || !LEVELS.has(row.level)) { omittedLines++; continue; }
    const correlation = row.correlation_id ?? row.op_id;
    entries.push({
      timestamp: typeof row.timestamp === "string" && TIMESTAMP.test(row.timestamp) ? row.timestamp : null,
      level: row.level,
      source,
      correlationId: typeof correlation === "string" && UUID.test(correlation) ? correlation : null,
    });
  }
  return {
    status: entries.length ? "collected" : "empty",
    entries: entries.slice(-DIAGNOSTIC_LOG_ENTRIES),
    truncated: partialFirstLine || entries.length > DIAGNOSTIC_LOG_ENTRIES,
    omittedLines,
  };
}

/** 환경에 명시된 trace 파일 또는 Desktop 시작 폴더의 기존 기본 trace만 읽는다. 재귀 탐색은 없다. */
export async function readDiagnosticLogs(cwd: string, exactPath?: string): Promise<DiagnosticLogs> {
  const empty = (status: DiagnosticLogs["status"]): DiagnosticLogs => ({ status, entries: [], truncated: false, omittedLines: 0 });
  try {
    let path = exactPath;
    if (path === "") return empty("absent");
    if (!path) {
      const directoryPath = join(cwd, ".akra", "runtime", "log");
      const directoryInfo = await lstat(directoryPath);
      if (!directoryInfo.isDirectory() || directoryInfo.isSymbolicLink()) return empty("unreadable");
      const directory = await opendir(directoryPath);
      let count = 0;
      let newest: { path: string; time: number } | undefined;
      for await (const entry of directory) {
        if (++count > MAX_DIRECTORY_ENTRIES) return empty("unreadable");
        if (!entry.isFile() || !TRACE_NAME.test(entry.name)) continue;
        const candidate = join(directoryPath, entry.name);
        const info = await lstat(candidate);
        if (info.isFile() && !info.isSymbolicLink() && (!newest || info.mtimeMs > newest.time)) newest = { path: candidate, time: info.mtimeMs };
      }
      path = newest?.path;
      if (!path) return empty("absent");
    }
    const before = await lstat(path);
    if (!before.isFile() || before.isSymbolicLink() || before.nlink !== 1) return empty("unreadable");
    const file = await open(path, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
    try {
      const info = await file.stat();
      if (!info.isFile() || info.nlink !== 1 || info.dev !== before.dev || info.ino !== before.ino) return empty("unreadable");
      const start = Math.max(0, info.size - DIAGNOSTIC_LOG_BYTES);
      const buffer = Buffer.alloc(Math.min(info.size, DIAGNOSTIC_LOG_BYTES));
      const { bytesRead } = await file.read(buffer, 0, buffer.length, start);
      return sanitizeDiagnosticLogLines(buffer.subarray(0, bytesRead).toString("utf8"), start > 0);
    } finally { await file.close(); }
  } catch (error) {
    return empty((error as NodeJS.ErrnoException).code === "ENOENT" ? "absent" : "unreadable");
  }
}
