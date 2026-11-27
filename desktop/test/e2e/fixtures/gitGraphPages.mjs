// 첫 쪽은 화면보다 훨씬 길다. 패널은 읽은 줄의 끝에서 60 줄 앞에 오면 다음 쪽을 읽는다.
export const FIRST_PAGE_ROWS = 200;
export const SECOND_PAGE_ROWS = 40;
export const sha = (number) => number.toString(16).padStart(40, "0");
export const NOW = Math.floor(Date.now() / 1000);
export const BACKUP_ONLY_ROW = 6;

// 첫 쪽: 머지 커밋 하나, 두 레인, 그 뒤로 한 줄 이력이다. 번호가 클수록 새 커밋이다.
export function firstPage(cwd, repositoryChanged) {
  const total = FIRST_PAGE_ROWS + SECOND_PAGE_ROWS;
  const rows = [
    { id: sha(total), parents: [sha(total - 2), sha(total - 1)], subject: "Merge topic into main", lane: 0, lanesIn: [], lanesOut: [0, 1], lanesThrough: [] },
    { id: sha(total - 1), parents: [sha(total - 3)], subject: "Topic work", lane: 1, lanesIn: [1], lanesOut: [1], lanesThrough: [0] },
    { id: sha(total - 2), parents: [sha(total - 3)], subject: "Main work", lane: 0, lanesIn: [0], lanesOut: [1], lanesThrough: [1] },
    { id: sha(total - 3), parents: [sha(total - 4)], subject: "Fork point", lane: 1, lanesIn: [1], lanesOut: [1], lanesThrough: [] },
  ];
  for (let number = total - 4; rows.length < FIRST_PAGE_ROWS; number -= 1) {
    rows.push({ id: sha(number), parents: [sha(number - 1)], subject: `Commit ${number}`, lane: 1, lanesIn: [1], lanesOut: [1], lanesThrough: [] });
  }
  rows[BACKUP_ONLY_ROW].backupOnly = true;
  if (repositoryChanged) rows.unshift({ id: sha(total + 1), parents: [sha(total)], subject: "New commit", lane: 0, lanesIn: [], lanesOut: [0], lanesThrough: [] });
  const head = rows[0].id;
  return {
    repository: true,
    stale: false,
    head,
    refsDigest: repositoryChanged ? "d2" : "d1",
    refs: [
      { name: "main", kind: "local_branch", target: head, head: true },
      { name: "origin/main", kind: "remote_branch", target: sha(total), head: false },
      { name: "origin/topic", kind: "remote_branch", target: sha(total - 1), head: false },
      { name: "v1.0", kind: "tag", target: sha(total - 3), head: false },
      { name: "main/op-1", kind: "backup", target: rows[repositoryChanged ? BACKUP_ONLY_ROW + 1 : BACKUP_ONLY_ROW].id, head: false },
    ],
    // 열린 스레드의 폴더가 작업 트리이다. 그 작업 트리는 "Topic work" 를 체크아웃했다.
    worktrees: [{ path: String(cwd).replaceAll("\\", "/"), head: sha(total - 1), branch: "topic" }],
    rows: rows.map((row, index) => ({ ...row, authorName: "Sample Author", authorTime: NOW - index * 3600 })),
    next: { skip: FIRST_PAGE_ROWS, lanes: [null, sha(SECOND_PAGE_ROWS)], refsDigest: "sample" },
  };
}

const COMMIT_DIFF = ["diff --git a/src/topic.ts b/src/topic.ts", "index 1111111..2222222 100644", "--- a/src/topic.ts", "+++ b/src/topic.ts", "@@ -1,2 +1,3 @@", " export const topic = 1;", "+export const more = 2;", " // end", ""].join("\n");

export function commitDetail(commit) {
  const total = FIRST_PAGE_ROWS + SECOND_PAGE_ROWS;
  const topic = commit === sha(total - 1);
  return {
    commit: {
      id: commit,
      parents: topic ? [sha(total - 3)] : [],
      authorName: "Sample Author",
      authorEmail: "author@example.com",
      authorTime: NOW - 3600,
      committerName: "Sample Committer",
      committerTime: NOW - 1800,
      message: topic ? "Topic work\n\nThe body of the topic commit." : commit === sha(total - 3) ? "Fork point" : "Another commit",
      diff: topic ? COMMIT_DIFF : "",
      diffTooLarge: false,
    },
  };
}

export function secondPage() {
  const rows = [];
  for (let number = SECOND_PAGE_ROWS; number >= 1; number -= 1) {
    rows.push({ id: sha(number), parents: number > 1 ? [sha(number - 1)] : [], subject: `Commit ${number}`, lane: 1, lanesIn: [1], lanesOut: number > 1 ? [1] : [], lanesThrough: [], authorName: "Sample Author", authorTime: NOW - 90 * 86400 });
  }
  return { repository: true, stale: false, head: sha(FIRST_PAGE_ROWS + SECOND_PAGE_ROWS), refs: [], rows, next: null };
}
export function operationPlan(operation, expectation) {
  const total = FIRST_PAGE_ROWS + SECOND_PAGE_ROWS;
  return {
    operation,
    expectation,
    branch: "main",
    before: { commit: sha(total + 1), branch: "main" },
    after: { commit: operation.target, branch: "main" },
    impact: { dropped: 4, unreachable: 2, gained: 0 },
    blockers: [],
  };
}

export function operationRecord(opId, operation, undoOf, cwd) {
  const total = FIRST_PAGE_ROWS + SECOND_PAGE_ROWS;
  return {
    opId,
    operation,
    // 작업을 실행한 폴더이다. 화면은 같은 폴더의 기록에만 되돌리기를 보인다.
    worktree: cwd,
    before: { commit: sha(total + 1), branch: "main" },
    after: { commit: operation.target, branch: "main" },
    backupRef: `refs/akra/backup/main/${opId}`,
    status: "done",
    startedAt: NOW,
    finishedAt: NOW,
    error: null,
    undoOf,
    undoneBy: null,
  };
}
