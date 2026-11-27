// 블로그 글의 상태와 단계 표시이다 (plan 31). 화면 저장소를 읽지 않는 함수만 둔다.
import type { ExtensionCall } from "./repositoryExtensions";

export type State = "running" | "reviewed" | "findings" | "edited" | "stopped";
export type Stage = "collect" | "write" | "review" | "revise";
export type Commit = { sha: string; time: number; subject: string };
export type Targets = { commits: Commit[]; fresh: number; selected: string[]; first: boolean; limit: number; codex: boolean };
export type Task = "generate" | "revise" | "review";
export type Summary = { id: string; title: string; state: State; stage: Stage | null; task: Task | null; updated: number | null; unread?: boolean; failed?: boolean };
export type Finding = { code: string; id?: string; reason?: string; sources?: string[]; figure?: string };
export type Article = Summary & {
  version: number; revision: number; calls: number; created?: number; draft: string; attempt?: number;
  edition: number; previous: { draft: string } | null;
  exported: { path: string; edition: number } | null;
  figures: { id: string; path: string; alt: string }[];
  claims?: { id: string; text: string }[];
  evidence: { commits: { id: string; sha: string; subject: string }[]; files: number; summarized: number } | null;
  review: { state: string; findings: Finding[] } | null;
  stopped: { reason: string; code: string; message: string } | null;
  job: { id: string; started: number } | null;
  messages: { role: string; text: string }[];
};

export const STAGES: { id: Stage; label: string }[] = [
  { id: "collect", label: "근거 모으기" }, { id: "write", label: "초안 쓰기" },
  { id: "review", label: "검수" }, { id: "revise", label: "고쳐 쓰기" },
];
/** 작업마다 보이는 단계이다. 사람이 시작한 다시 쓰기와 검수는 자기 단계만 보인다. */
export function taskStages(task: Task | null): typeof STAGES {
  if (task === "revise") return STAGES.filter(s => s.id === "revise" || s.id === "review").reverse();
  if (task === "review") return STAGES.filter(s => s.id === "review");
  return STAGES;
}

/** 다시 쓰기와 검수는 순서대로 간다. 지금 단계 앞은 끝났고 뒤는 기다린다. */
export function taskMarks(task: Task | null, stage: Stage | null, attempt: number, stopped: boolean): Mark[] {
  if (task !== "revise" && task !== "review") return stageMarks(stage, attempt, stopped);
  const steps = taskStages(task);
  const current = steps.findIndex(s => s.id === stage);
  return steps.map((_, index) => index === current ? (stopped ? "failed" : "active") : index < current || stage === null ? "done" : "wait");
}

export type Mark = "done" | "active" | "failed" | "wait";

export function stateLabel(state: State, findings: number): string {
  return { running: "만드는 중", reviewed: "검수 통과", findings: `지적 ${findings}건`, edited: "검수 안 함", stopped: "멈춤" }[state];
}

/** 단계마다 끝남, 지금, 멈춤, 기다림 중 하나다. 고쳐 쓰기 뒤의 검수는 "검수"가 다시 지금이 된다. */
export function stageMarks(stage: Stage | null, attempt: number, stopped: boolean): Mark[] {
  const order = (id: Stage) => STAGES.findIndex(s => s.id === id);
  return STAGES.map(({ id }) => {
    if (stage === null) return id === "revise" && attempt < 1 ? "wait" : "done";
    if (id === stage) return stopped ? "failed" : "active";
    if (attempt >= 1 && (id === "revise" || order(id) < order("review"))) return "done";
    return order(id) < order(stage) ? "done" : "wait";
  });
}

/** 위쪽 띠의 책 아이콘이 보이는 값이다 (plan 31 §7.7). */
export type BlogActivity = { running: number; unread: number; stopped: number; next: string | null };

/** 마지막 작업이 멈췄다. 다시 쓰기·검수가 멈춘 글은 상태가 앞의 값이므로 `failed` 로 안다. */
export const didStop = (a: Summary) => a.state === "stopped" || !!a.failed;

export function blogActivity(articles: Summary[]): BlogActivity {
  const unread = articles.filter(a => a.unread && a.state !== "running");
  return {
    running: articles.filter(a => a.state === "running").length,
    unread: unread.length,
    stopped: unread.filter(didStop).length,
    next: unread[0]?.id ?? null,
  };
}

export function activityTitle({ running, unread, stopped }: Omit<BlogActivity, "next">): string {
  if (running) return "기술 블로그 · 글을 만드는 중";
  if (stopped) return `기술 블로그 · 멈춘 글 ${stopped}개`;
  if (unread) return `기술 블로그 · 새 글 ${unread}개`;
  return "기술 블로그";
}

/** 앞의 목록에서 만드는 중이던 글 가운데, 지금 끝나서 읽지 않은 결과가 된 글이다. 사람이 취소한 글은 빠진다. */
export function finishedArticles(previous: Summary[], next: Summary[]): Summary[] {
  const wasRunning = new Set(previous.filter(a => a.state === "running").map(a => a.id));
  return next.filter(a => wasRunning.has(a.id) && a.state !== "running" && a.unread);
}

/** 공통 확장 호스트의 작업이다. 호출은 receipt를 돌려주고, 워커가 맡은 글 ID는 progress로 온다. */
export const JOBS = new Set(["generate", "resume", "revise", "review"]);
const JOB_POLL_MS = 200;
const JOB_START_LIMIT_MS = 30_000;
export interface Receipt { id: string; state: string; error?: string | null; progress?: { article_id?: string } | null; result?: { id?: string } | null }

/** 작업이 글을 맡을 때까지 기다린다. 워커가 거절하면(예: 다른 창에서 글이 바뀜) 그 오류를 던진다. */
export async function jobArticle(call: ExtensionCall, receipt: Receipt, wait = (ms: number) => new Promise(resolve => setTimeout(resolve, ms))): Promise<string> {
  let job = receipt;
  for (let waited = 0; ; waited += JOB_POLL_MS) {
    const id = job.progress?.article_id ?? job.result?.id;
    if (id) return id;
    if (["failed", "cancelled", "interrupted", "completed"].includes(job.state)) throw new Error(job.error || "작업이 글을 맡지 못했습니다.");
    if (waited >= JOB_START_LIMIT_MS) throw new Error("작업이 아직 시작되지 않았습니다. 잠시 뒤 글 목록을 확인하세요.");
    await wait(JOB_POLL_MS);
    job = await call<Receipt>("job", { job_id: job.id });
  }
}
