# Immutable diff context

This increment is based directly on #2991 at `9ba5586001eb54b6a42bcecc1e54306c2ba0e6b6`.
It continues #2984 and #2986 for #2948. It adds a local, read-only
context contract. It does not store attachments or submit messages.

## Contract

`gitFileDiff(cwd, path, untracked, source)` keeps its existing patch response.
Its `snapshot.contents` describes the captured old and new contents:

- `head_index`: fixed HEAD blob (empty before the first commit) and index blob.
- `index_worktree`: index blob and captured working file.
- `head_worktree`: fixed HEAD blob (empty before the first commit) and captured working file.
- `untracked`: empty old content and captured working file.
- `worktree_file`: the existing #2991 full-file source remains available, with its
  repository and worktree identities. It is not an old-side diff fallback.

The snapshot includes a SHA-256 content fingerprint for each complete side,
the index entry fingerprint, canonical repository root, line counts, line endings,
and capture time. The capture checks HEAD, index entries, path metadata, status,
patch bytes, and both content reads before publishing the snapshot.

`gitDiffContext(cwd, { snapshotId, source, side, startLine, endLine })` returns
one-based inclusive file lines. It reads only the captured contents. A stale
repository observation sets `current: false`; it does not replace the original
text. A different repository or source is rejected.

Limits:

- 1 MiB for each file side and the existing patch output.
- At most 16 snapshots and 16 MiB of retained UTF-8 content.
- At most 200 lines and 64 KiB per context response.
- Binary, unsupported entries (including gitlinks and symlinks), invalid UTF-8,
  and oversized file sides return explicit unavailable reasons.
- Eviction returns `snapshot_expired`. There is no disk fallback.

The cache is process-local. #2826 owns durable selected text and surrounding
context. A consumer must preserve the source, snapshot ID, side, actual file
range, original path, and target scope when creating that attachment.

## Reproduction

From `desktop`:

```text
npm test -- --run test/gitDiffContext.test.ts test/gitStatus.test.ts
npm run check
```

From the worktree root:

```text
node scripts/agent-plan.mjs --run
```

`gitDiffContext.test.ts` uses actual Git repositories and files. It covers
combined/staged/unstaged content, unchanged surrounding lines, CRLF and Korean
paths, rename, deletion, unborn/untracked additions, gitlinks, binary and size
limits, stale content retention, scope rejection, bounded ranges, cache eviction,
and a controlled change between the two file reads.

## Windows run, 2026-11-27

- The 13 context fixtures passed. Existing Git and attachment fixtures passed
  (34 tests, one existing platform skip). Both Desktop TypeScript checks passed.
- The Desktop production build passed.
- In an isolated scratch worktree, replacing the selected side with the new side
  made all three old/new source fixtures fail with the wrong source text.
  The mutation was then restored.
- The planner selected Node surfaces. All Windows Admin checks passed. The
  planner rejected the final receipt because its build changed registered inputs.
  The separate validation repair #3000 addresses that Windows build-input issue;
  it is not included in this source increment.
- Native IPC reproduction is `test/e2e/diff-snapshot-context.mjs`. Set
  `AKRA_ELECTRON_EXECUTABLE` to a complete Electron 45.2.3 installation when the
  dependency install uses `--ignore-scripts`.
  The native run passed: old `INDEX_ONLY`, new `WORKTREE_ONLY`, unchanged index,
  and the original old text with `current: false` after a later file edit.

## Outstanding integration

| Issue | Prerequisite still required |
| --- | --- |
| #2948 | Historical turn snapshots need their own immutable source identity. Patch-only historical turns must remain partial. Remote-host capture needs an authoritative host adapter. Neither can read the current local file as historical context. |
| #2949 | The view must expose exact-context reads, navigation, selection, and bounded rendering, with native screen and timing evidence. |
| #2827 | #2826 durable draft/source/target integration and #2949 semantic selection must be connected to feedback cards. Preview, multiple-file ordering, dispatch preflight, unknown-result recovery, and explicit review state remain integration requirements. |
| #2947 | Aggregate acceptance depends on #2825 manifest integration, #2826 persistence, #2827 delivery/review state, and remaining #2948/#2949 acceptance. This source increment does not close the epic. |
