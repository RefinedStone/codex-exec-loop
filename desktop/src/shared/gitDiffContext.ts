import type { GitDiffSource } from "./bridge";

export type GitDiffSide = "old" | "new";
export type GitContentLimit = "size_limit" | "binary" | "unsupported_entry" | "invalid_utf8";

export type GitSnapshotContent =
  | { readonly status: "complete"; readonly hash: string; readonly lineCount: number; readonly lineEnding: "lf" | "crlf" | "mixed" | "none" }
  | { readonly status: "unavailable"; readonly reason: GitContentLimit };

export interface GitSnapshotContents {
  readonly capturedAt: string;
  readonly root: string;
  readonly indexFingerprint: string;
  readonly old: GitSnapshotContent;
  readonly new: GitSnapshotContent;
}

/** File lines are one-based and inclusive. One request returns at most 200 lines / 64 KiB. */
export interface GitDiffContextRequest {
  readonly snapshotId: string;
  readonly source: GitDiffSource;
  readonly side: GitDiffSide;
  readonly startLine: number;
  readonly endLine: number;
}

export type GitDiffContextResult =
  | { readonly status: "available"; readonly snapshotId: string; readonly source: GitDiffSource;
      readonly side: GitDiffSide; readonly startLine: number; readonly endLine: number;
      readonly text: string; readonly current: boolean }
  | { readonly status: "unavailable"; readonly reason: GitContentLimit | "snapshot_expired" | "scope_mismatch" | "range_limit" };
