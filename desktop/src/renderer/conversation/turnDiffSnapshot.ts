import type { GitDiffContextRequest, GitDiffSide } from "../../shared/gitDiffContext";
import type { TurnDiffFileSource } from "./diff";

export interface TurnDiffScope {
  readonly hostId: string | null;
  readonly threadId: string;
  readonly turnId: string;
  readonly cwd: string;
}

interface TurnSide {
  readonly status: "complete" | "partial" | "unavailable";
  readonly firstLine: number | null;
  readonly lineCount: number | null;
  readonly lines: Readonly<Record<number, string>>;
}

export interface TurnDiffSnapshot {
  readonly source: "last_turn";
  readonly snapshotId: string;
  readonly scope: TurnDiffScope;
  readonly origin: string;
  readonly path: string;
  readonly originalPath: string | null;
  readonly old: TurnSide;
  readonly new: TurnSide;
}

export type TurnCaptureResult =
  | { readonly status: "captured"; readonly snapshot: TurnDiffSnapshot }
  | { readonly status: "unavailable"; readonly reason: "size_limit" | "binary" | "invalid_patch" };
export type TurnContextRequest = Pick<GitDiffContextRequest, "snapshotId" | "side" | "startLine" | "endLine">;
export type TurnContextResult =
  | { readonly status: "available"; readonly source: "last_turn"; readonly snapshotId: string;
      readonly side: GitDiffSide; readonly startLine: number; readonly endLine: number; readonly text: string }
  | { readonly status: "unavailable"; readonly reason: "context_not_provided" | "range_limit" | "snapshot_mismatch" };

const SOURCE_LIMIT = 1 << 20;
const encoder = new TextEncoder();

/** Only complete, count-checked hunks contribute file lines. A cut-off hunk contributes nothing. */
function patchLines(text: string): { old: Map<number, string>; new: Map<number, string> } | null {
  const rows = text.match(/[^\n]*\n|[^\n]+$/g) ?? [];
  const result = { old: new Map<number, string>(), new: new Map<number, string>() };
  for (let index = 0; index < rows.length; index += 1) {
    const header = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/.exec(rows[index] ?? "");
    if (!header) continue;
    const oldStart = Number(header[1]), oldCount = Number(header[2] ?? "1");
    const newStart = Number(header[3]), newCount = Number(header[4] ?? "1");
    if (![oldStart, oldCount, newStart, newCount, oldStart + oldCount, newStart + newCount].every(Number.isSafeInteger) ||
      (oldCount > 0 && oldStart < 1) || (newCount > 0 && newStart < 1)) return null;
    const hunk = { old: new Map<number, string>(), new: new Map<number, string>() };
    let previous: readonly GitDiffSide[] = [];
    let cursor = index + 1;
    let valid = true;
    for (; cursor < rows.length; cursor += 1) {
      const row = rows[cursor] ?? "";
      if (row.startsWith("\\ No newline at end of file")) {
        for (const side of previous) {
          const line = (side === "old" ? oldStart : newStart) + hunk[side].size - 1;
          const value = hunk[side].get(line);
          if (value !== undefined) hunk[side].set(line, value.replace(/\n$/, ""));
        }
        previous = [];
        continue;
      }
      if (hunk.old.size === oldCount && hunk.new.size === newCount) break;
      switch (row[0]) {
        case " ": previous = ["old", "new"]; break;
        case "-": previous = ["old"]; break;
        case "+": previous = ["new"]; break;
        default: previous = []; valid = false;
      }
      if (!valid || !row.endsWith("\n")) { valid = false; break; }
      for (const side of previous) {
        const start = side === "old" ? oldStart : newStart;
        hunk[side].set(start + hunk[side].size, row.slice(1));
      }
      if (hunk.old.size > oldCount || hunk.new.size > newCount) { valid = false; break; }
    }
    if (valid && hunk.old.size === oldCount && hunk.new.size === newCount) {
      for (const side of ["old", "new"] as const) {
        for (const [line, value] of hunk[side]) {
          if (result[side].has(line)) return null;
          result[side].set(line, value);
        }
      }
    }
    index = Math.max(index, cursor - 1);
  }
  return result;
}

/** The input is the turn's data, not a Git revision or a filesystem lookup instruction. */
export async function captureTurnDiff(source: TurnDiffFileSource, scope: TurnDiffScope): Promise<TurnCaptureResult> {
  const { kind, text } = source.input;
  if (text.length > SOURCE_LIMIT || encoder.encode(text).byteLength > SOURCE_LIMIT) {
    return { status: "unavailable", reason: "size_limit" };
  }
  if (text.includes("\0")) return { status: "unavailable", reason: "binary" };
  const origin = source.origin;
  const path = source.file.movePath ?? source.file.path;
  const originalPath = source.file.movePath ? source.file.path : null;
  const capturedScope = Object.freeze({
    hostId: scope.hostId, threadId: scope.threadId, turnId: scope.turnId, cwd: scope.cwd,
  });
  let sides: { old: Map<number, string>; new: Map<number, string> };
  let complete: boolean;
  switch (kind) {
    case "patch":
    case "update": {
      const parsed = patchLines(text);
      if (parsed === null) return { status: "unavailable", reason: "invalid_patch" };
      sides = parsed; complete = false; break;
    }
    case "add":
    case "delete": {
      const lines = new Map((text.match(/[^\n]*\n|[^\n]+$/g) ?? []).map((line, index) => [index + 1, line]));
      sides = kind === "add" ? { old: new Map(), new: lines } : { old: lines, new: new Map() };
      complete = true; break;
    }
  }
  const capturedSide = (lines: Map<number, string>): TurnSide => Object.freeze({
    status: complete ? "complete" : lines.size > 0 ? "partial" : "unavailable",
    firstLine: lines.keys().next().value ?? null,
    lineCount: complete ? lines.size : null,
    lines: Object.freeze(Object.fromEntries(lines)),
  });
  const old = capturedSide(sides.old), next = capturedSide(sides.new);
  const digest = await crypto.subtle.digest("SHA-256", encoder.encode(JSON.stringify([capturedScope, origin, path, originalPath, kind, text])));
  const snapshotId = Array.from(new Uint8Array(digest), byte => byte.toString(16).padStart(2, "0")).join("");
  return { status: "captured", snapshot: Object.freeze({
    source: "last_turn", snapshotId, scope: capturedScope, origin, path, originalPath, old, new: next,
  }) };
}

export function readTurnDiffContext(snapshot: TurnDiffSnapshot, request: TurnContextRequest): TurnContextResult {
  if (request.snapshotId !== snapshot.snapshotId) return { status: "unavailable", reason: "snapshot_mismatch" };
  if (!Number.isSafeInteger(request.startLine) || !Number.isSafeInteger(request.endLine) ||
    request.startLine < 1 || request.endLine < request.startLine || request.endLine - request.startLine >= 200) {
    return { status: "unavailable", reason: "range_limit" };
  }
  const lines: string[] = [];
  for (let line = request.startLine; line <= request.endLine; line += 1) {
    const text = snapshot[request.side].lines[line];
    if (text === undefined) return { status: "unavailable", reason: "context_not_provided" };
    lines.push(text);
  }
  const text = lines.join("");
  if (encoder.encode(text).byteLength > 64 << 10) return { status: "unavailable", reason: "range_limit" };
  return { status: "available", source: "last_turn", snapshotId: snapshot.snapshotId,
    side: request.side, startLine: request.startLine, endLine: request.endLine, text };
}
