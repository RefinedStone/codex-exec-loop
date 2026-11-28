import type { GitDiffSnapshot } from "../../shared/bridge";
import type { GitDiffSide } from "../../shared/gitDiffContext";
import type { DiffFile } from "./diff";

/** Stable file coordinates. A display row is never an attachment location. */
export interface DiffSelection {
  readonly snapshotId: string;
  readonly source: GitDiffSnapshot["source"];
  readonly path: string;
  readonly originalPath: string | null;
  readonly side: GitDiffSide;
  readonly startLine: number;
  readonly endLine: number;
}

export interface DiffCoordinate {
  readonly text: string;
  readonly oldLine: number | null;
  readonly newLine: number | null;
  readonly hunk: boolean;
}

export function diffCoordinates(file: DiffFile): readonly DiffCoordinate[] {
  let oldLine: number | null = file.kind === "delete" ? 1 : null;
  let newLine: number | null = file.kind === "add" ? 1 : null;
  return file.lines.flatMap((line): DiffCoordinate[] => {
    switch (line.kind) {
      case "meta": return [];
      case "hunk": {
        const match = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/.exec(line.text);
        if (!match) return [];
        oldLine = Number(match[1]); newLine = Number(match[3]);
        return [{ text: line.text, oldLine: match[2] === "0" ? null : oldLine,
          newLine: match[4] === "0" ? null : newLine, hunk: true }];
      }
      case "add": return [{ text: line.text, oldLine: null, newLine: newLine === null ? null : newLine++, hunk: false }];
      case "del": return [{ text: line.text, oldLine: oldLine === null ? null : oldLine++, newLine: null, hunk: false }];
      case "context": return [{ text: line.text, oldLine: oldLine === null ? null : oldLine++, newLine: newLine === null ? null : newLine++, hunk: false }];
    }
  });
}

export function selectionAt(snapshot: GitDiffSnapshot, target: { readonly side: GitDiffSide; readonly startLine: number; readonly endLine: number }): DiffSelection {
  return { snapshotId: snapshot.snapshotId, source: snapshot.source, path: snapshot.path,
    originalPath: snapshot.originalPath, ...target };
}
