import { createHash } from "node:crypto";
import type { GitFileDiff, GitDiffSnapshot } from "../shared/bridge";
import { isConflictStatus } from "../shared/gitChange";
import { readUntrackedFile, FILE_DIFF_LIMIT } from "./gitFileContent";
import { repositoryFile, runGit, failureMessage, currentChange, anyContentFilter, snapshotGitFilePlan, FILTER_MESSAGE, CHANGED_SINCE_CONFIRMATION, type RepositoryFile } from "./gitStatus";
import { captureBlob, captureWorkingFile, capturedText } from "./gitSnapshotContent";
import { retainDiffSnapshot } from "./gitDiffContext";
const DIFF_PATH_OPTIONS = ["--src-prefix=a/", "--dst-prefix=b/", "--no-relative"];

/** 파일 하나의 변경이다. 추적 중인 파일은 HEAD 대비 git diff, 새 파일은 내용이다. */
export async function readGitFileDiff(cwdInput: unknown, path: unknown, untracked: unknown, source?: unknown): Promise<GitFileDiff> {
  const file = await repositoryFile(cwdInput, path);
  if (source !== undefined) return readSourceDiff(file, source);
  const { cwd, root, target, pathspec } = file;
  if (untracked === true) {
    return readUntrackedFile(root, target);
  }
  // An unborn repository has no HEAD. Git's cached diff compares the index with the empty tree.
  const head = await runGit(cwd, ["rev-parse", "--verify", "-q", "HEAD"], 4096);
  const base = head.code === 0 ? ["HEAD"] : ["--cached"];
  const output = await runGit(cwd, ["diff", ...base, "--no-color", "--no-ext-diff", "--no-textconv", ...DIFF_PATH_OPTIONS, "--", pathspec], FILE_DIFF_LIMIT);
  if (output.code !== 0) {
    throw new Error(failureMessage(output, "git diff 가 실패했습니다."));
  }
  return { kind: "gitDiff", text: output.stdout, truncated: output.truncated };
}

/** Explicitly distinguish HEAD, index and working-copy comparisons. Never reconstruct full files from a patch. */
async function readSourceDiff(file: RepositoryFile, sourceInput: unknown): Promise<GitFileDiff> {
  if (sourceInput !== "head_worktree" && sourceInput !== "head_index" && sourceInput !== "index_worktree" && sourceInput !== "untracked" && sourceInput !== "worktree_file") {
    throw new Error("지원하지 않는 비교 기준입니다.");
  }
  const source = sourceInput;
  const change = await currentChange(file);
  if (change?.directory || (change && isConflictStatus(change.status)) ||
      (source !== "worktree_file" && (!change || change.untracked !== (source === "untracked")))) {
    throw new Error("파일 상태가 변경되었거나 선택한 비교 기준을 사용할 수 없습니다.");
  }
  const original = change?.originalPath ? await repositoryFile(file.root, change.originalPath) : null;
  if (await anyContentFilter(file.root, [file.relativePath, ...(original ? [original.relativePath] : [])])) {
    throw new Error(FILTER_MESSAGE);
  }
  // A staged rename already has its new path in the index. Only the worktree status
  // column identifies a rename between the index and the working copy.
  const comparisonOriginal = source === "index_worktree" && !/[RC]/.test(change?.status[1] ?? "") ? null : original;
  const before = await snapshotGitFilePlan(file, original);
  if (source === "worktree_file" && !before.indexEntries && !change?.untracked) {
    throw new Error("추적하거나 변경 목록에 있는 파일을 선택하십시오.");
  }
  const capture = async () => {
    const old = source === "untracked" || source === "worktree_file" ? capturedText("")
      : await captureBlob(comparisonOriginal ?? file, source === "index_worktree" ? "index" : before.headOid);
    const stagedDeletion = change?.status[0] === "D" && !before.indexEntries;
    const next = source === "head_index" ? await captureBlob(file, "index")
      : stagedDeletion && (source === "head_worktree" || source === "index_worktree") ? capturedText("")
      : await captureWorkingFile(file);
    return { old, new: next };
  };
  const contents = await capture();
  const emptyBase = source === "head_worktree" && before.headOid === null
    ? await runGit(file.root, ["hash-object", "-t", "tree", "--stdin"], 256) : null;
  if (emptyBase && (emptyBase.code !== 0 || emptyBase.truncated || emptyBase.timedOut)) {
    throw new Error(failureMessage(emptyBase, "Cannot resolve empty tree."));
  }
  const read = async (): Promise<GitFileDiff> => {
    if (source === "untracked" || source === "worktree_file") return readUntrackedFile(file.root, file.target);
    const args = source === "head_index" ? ["--cached", ...(before.headOid ? [before.headOid] : [])]
      : source === "head_worktree" ? [before.headOid ?? emptyBase?.stdout.trim() ?? ""] : [];
    const result = await runGit(file.root, ["diff", ...args, "--no-color", "--no-ext-diff", "--no-textconv", "--find-renames", ...DIFF_PATH_OPTIONS, "--", file.pathspec,
      ...(comparisonOriginal ? [comparisonOriginal.pathspec] : [])], FILE_DIFF_LIMIT);
    if (result.code !== 0 || result.timedOut) throw new Error(failureMessage(result, "git diff 가 실패했습니다."));
    return { kind: "gitDiff", text: result.stdout, truncated: result.truncated };
  };
  const result = await read();
  const repeat = await read();
  const repeatedContents = await capture();
  const after = await snapshotGitFilePlan(file, original);
  const updated = await currentChange(file);
  if (JSON.stringify(before) !== JSON.stringify(after) || JSON.stringify(result) !== JSON.stringify(repeat) ||
      JSON.stringify(contents) !== JSON.stringify(repeatedContents) ||
      updated?.status !== change?.status || updated?.originalPath !== change?.originalPath) {
    throw new Error(CHANGED_SINCE_CONFIRMATION);
  }
  const snapshotId = createHash("sha256").update(JSON.stringify([file.root, file.relativePath, change?.originalPath ?? null, source, before, result, contents.old.info, contents.new.info])).digest("hex");
  const common = await runGit(file.root, ["rev-parse", "--path-format=absolute", "--git-common-dir"], 4096);
  if (common.code !== 0 || common.truncated || common.timedOut) throw new Error("저장소 출처를 확인할 수 없습니다.");
  const binary = result.kind === "binary" || (result.kind === "gitDiff" && /^(Binary files .* differ|GIT binary patch)\r?$/m.test(result.text));
  const snapshot: GitDiffSnapshot = { source, snapshotId, headOid: before.headOid, path: file.relativePath,
    worktreeId: file.root, repositoryId: common.stdout.trim(),
    originalPath: comparisonOriginal?.relativePath ?? null,
    contentStatus: binary ? "binary" : result.truncated ? "truncated" : result.kind === "content" ? "complete" : "patch_only",
    contents: { capturedAt: new Date().toISOString(), root: file.root,
      indexFingerprint: createHash("sha256").update(before.indexEntries).digest("hex"),
      old: contents.old.info, new: contents.new.info } };
  retainDiffSnapshot({ snapshot, file, original, guard: before, ...contents });
  return { ...result, snapshot };
}
