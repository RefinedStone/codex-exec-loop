// 블로그 글의 상태와 단계 표시이다 (plan 31). 화면 저장소를 읽지 않는 순수 함수만 둔다.
export type State = "running" | "reviewed" | "findings" | "edited" | "stopped";
export type Stage = "collect" | "write" | "review" | "revise";
export type Commit = { sha: string; time: number; subject: string };
export type Targets = { commits: Commit[]; fresh: number; selected: string[]; first: boolean; limit: number; codex: boolean };
export type Task = "generate" | "revise" | "review";
export type Summary = { id: string; title: string; state: State; stage: Stage | null; task: Task | null; updated: number | null };
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
