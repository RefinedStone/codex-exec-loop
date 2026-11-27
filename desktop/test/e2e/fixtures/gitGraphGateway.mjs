import { FIRST_PAGE_ROWS, SECOND_PAGE_ROWS, sha, commitDetail, firstPage, operationPlan, operationRecord, secondPage } from "./gitGraphPages.mjs";

export function graphFixture() {
  const state = {
    unborn: false, repositoryChanged: false, worktreeProgress: { operationInProgress: null, conflictedFiles: 0 },
    initialStalePages: 1, holdFirstPagesStale: false, staleFirstPages: 0, onStaleFirstPage: () => {}, refsRequests: 0,
  };
  let staleAnswers = 1;
  const planRequests = [], executeRequests = [], undoRequests = [], journal = [], graphRequests = [], commitRequests = [];
  const handlers = {
    gitGraph(params) {
      if (params.filter?.author === "needle" && params.filter?.hide) {
        graphRequests.push(params);
        const page = params.cursor ? secondPage() : firstPage(params.cwd, state.repositoryChanged);
        return { ...page, rows: [page.rows[0]], next: params.cursor ? null : page.next };
      }
      if (state.unborn) return { repository: true, stale: false, head: null, refsDigest: "unborn", refs: [], worktrees: [], rows: [], next: null };
      if (!params.cursor && (state.initialStalePages > 0 || state.holdFirstPagesStale)) {
        state.initialStalePages = Math.max(0, state.initialStalePages - 1);
        state.staleFirstPages += 1;
        state.onStaleFirstPage();
        return { repository: true, stale: true, head: sha(FIRST_PAGE_ROWS + SECOND_PAGE_ROWS + 1), refs: [], rows: [], next: null };
      }
      graphRequests.push(params);
      return params.cursor ? secondPage() : firstPage(params.cwd, state.repositoryChanged);
    },
    gitRefs() {
      state.refsRequests += 1;
      return { repository: true, refsDigest: state.unborn ? "unborn" : state.repositoryChanged ? "d2" : "d1", ...state.worktreeProgress };
    },
    gitCommit(params) {
      commitRequests.push(params.commit);
      return commitDetail(params.commit);
    },
    gitPlan(params) {
      planRequests.push(params);
      if (params.operation.kind === "cherry_pick") return { ...operationPlan(params.operation, "seen-at-plan"), after: null, picks: params.operation.commits.map((commit, i) => ({ commit, prediction: i === 0 ? { status: "clean" } : { status: "conflict", paths: ["a.txt"] } })) };
      return operationPlan(params.operation, "seen-at-plan");
    },
    gitExecute(params) {
      executeRequests.push(params);
      if (staleAnswers > 0) {
        staleAnswers -= 1;
        return { status: "stale", plan: operationPlan(params.operation, "seen-after-change") };
      }
      const record = operationRecord("20261119T010203Z-0000002a", params.operation, null, params.cwd);
      journal.unshift(record);
      return { status: "done", record };
    },
    gitOperations() {
      return { operations: journal };
    },
    gitUndo(params) {
      undoRequests.push(params.opId);
      const undone = journal.find((record) => record.opId === params.opId);
      const record = operationRecord("20261119T010300Z-0000002b", { kind: "reset", target: undone.before.commit }, undone.opId, params.cwd);
      undone.undoneBy = record.opId;
      journal.unshift(record);
      return { status: "done", record };
    },
  };
  return { handlers, state, planRequests, executeRequests, undoRequests, journal, graphRequests, commitRequests };
}
