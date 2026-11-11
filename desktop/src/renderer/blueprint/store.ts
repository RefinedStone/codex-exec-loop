// 블루프린트의 화면 상태와 mock 턴 재생이다. Gateway 에 접속하지 않고, 모델을 부르지 않는다.
// 턴 재생은 타이머로 만든 가짜 진행이다. 실제 앱에서는 `DesktopSession` 과 `desktopStore` 가 이 자리를 맡는다.
import { create } from "zustand";

import {
  EFFORTS,
  initialTurns,
  MODELS,
  PERMISSIONS,
  T2_APPROVAL,
  THREADS,
  type MockApproval,
  type MockItem,
  type MockStep,
  type MockThread,
  type MockTurn,
  type PlanStep,
  type ThreadStatus,
} from "./mockData";

export type View = "home" | "thread" | "fleet";
export type PanelId = "review" | "kernel" | "browser" | "work" | "docs";
export type Theme = "light" | "dark";

export interface ReviewNote {
  path: string;
  line: number;
  text: string;
}

export interface Toast {
  id: number;
  text: string;
}

export interface BlueprintState {
  theme: Theme;
  view: View;
  threads: MockThread[];
  turns: Record<string, MockTurn[]>;
  approvals: Record<string, MockApproval>;
  /** 턴이 진행 중일 때 보낸 메시지이다. 턴이 끝나면 차례로 보낸다. */
  queued: Record<string, string[]>;
  runStartedAt: Record<string, number>;
  openThreadId: string | null;
  homeProjectId: string;
  homeWorktree: boolean;
  sidebarOpen: boolean;
  panel: PanelId | null;
  terminalOpen: boolean;
  paletteOpen: boolean;
  shortcutsOpen: boolean;
  model: string;
  effort: string;
  permission: string;
  reviewNotes: ReviewNote[];
  /** 업무 패널이나 제안이 넘긴 글이다. 입력창이 받아서 채운다. 바로 보내지 않는다. */
  handOff: string | null;
  toasts: Toast[];
}

const TWELVE_MINUTES_MS = 12 * 60 * 1000;

function initialTheme(): Theme {
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

export const useBlueprint = create<BlueprintState>(() => ({
  theme: initialTheme(),
  view: "thread",
  threads: THREADS,
  turns: Object.fromEntries(THREADS.map((thread) => [thread.id, initialTurns(thread)])),
  approvals: { t2: T2_APPROVAL },
  queued: {},
  runStartedAt: { t2: Date.now() - 3 * 60 * 1000, t3: Date.now() - TWELVE_MINUTES_MS },
  openThreadId: "t1",
  homeProjectId: "akra",
  homeWorktree: true,
  sidebarOpen: true,
  panel: null,
  terminalOpen: false,
  paletteOpen: false,
  shortcutsOpen: false,
  model: MODELS[0],
  effort: EFFORTS[1],
  permission: PERMISSIONS[1],
  reviewNotes: [],
  handOff: null,
  toasts: [],
}));

const set = useBlueprint.setState;
const get = useBlueprint.getState;

// ---- 화면 동작 ----

let toastCounter = 0;

export function toast(text: string): void {
  toastCounter += 1;
  const id = toastCounter;
  set((state) => ({ toasts: [...state.toasts, { id, text }] }));
  window.setTimeout(() => set((state) => ({ toasts: state.toasts.filter((item) => item.id !== id) })), 2800);
}

/** 아직 연결하지 않은 기능의 버튼이 쓴다. */
export function notConnected(feature: string): void {
  toast(`${feature}: 블루프린트에서는 동작하지 않습니다 (mock)`);
}

function setThreadStatus(threadId: string, status: ThreadStatus): void {
  set((state) => ({ threads: state.threads.map((thread) => (thread.id === threadId ? { ...thread, status, updated: "방금" } : thread)) }));
}

export function openThread(threadId: string): void {
  set((state) => ({
    view: "thread",
    openThreadId: threadId,
    paletteOpen: false,
    // 스레드를 보면 읽지 않음 상태가 없어진다.
    threads: state.threads.map((thread) => (thread.id === threadId && thread.status === "unread" ? { ...thread, status: "idle" } : thread)),
  }));
}

export function showView(view: View): void {
  set({ view, paletteOpen: false });
}

export function togglePanel(panel: PanelId): void {
  set((state) => ({ panel: state.panel === panel ? null : panel, paletteOpen: false }));
}

export function toggleTheme(): void {
  set((state) => ({ theme: state.theme === "dark" ? "light" : "dark" }));
}

export function togglePinned(threadId: string): void {
  set((state) => ({ threads: state.threads.map((thread) => (thread.id === threadId ? { ...thread, pinned: !thread.pinned } : thread)) }));
}

/** 글을 새 스레드 화면의 입력창에 채운다. */
export function handOffToNewThread(text: string): void {
  set({ view: "home", handOff: text, paletteOpen: false });
}

function updateThread(threadId: string, update: Partial<MockThread>): void {
  set((state) => ({ threads: state.threads.map((thread) => (thread.id === threadId ? { ...thread, ...update } : thread)) }));
}

/** 끝난 스레드를 목록에서 뺀다. 진행 중인 스레드는 정리하지 않는다. */
export function markDone(threadId: string): void {
  if (isRunning(threadId)) {
    toast("진행 중인 스레드는 완료로 정리할 수 없습니다");
    return;
  }
  updateThread(threadId, { done: true, snoozedUntil: null, pinned: false });
  toast("완료로 정리했습니다");
}

/** 스레드를 정해진 때까지 목록에서 뺀다 (mock: 때는 고정 값이다). */
export function snooze(threadId: string): void {
  updateThread(threadId, { snoozedUntil: "내일 오전 9시", pinned: false });
  toast("내일 오전 9시까지 미뤘습니다");
}

/** 완료나 미루기를 취소하고 목록에 되돌린다. */
export function restoreThread(threadId: string): void {
  updateThread(threadId, { done: false, snoozedUntil: null });
}

export function editQueued(threadId: string, index: number, text: string): void {
  set((state) => ({
    queued: { ...state.queued, [threadId]: (state.queued[threadId] ?? []).map((current, position) => (position === index ? text : current)) },
  }));
}

export function addReviewNote(note: ReviewNote): void {
  set((state) => ({ reviewNotes: [...state.reviewNotes, note] }));
}

export function removeQueued(threadId: string, index: number): void {
  set((state) => ({ queued: { ...state.queued, [threadId]: (state.queued[threadId] ?? []).filter((_, position) => position !== index) } }));
}

// ---- mock 턴 재생 ----

interface TurnRef {
  threadId: string;
  turnId: string;
}

interface Run {
  cancelled: boolean;
  answerApproval: ((accepted: boolean) => void) | null;
}

const runs = new Map<string, Run>();
const CANCELLED = Symbol("cancelled");
let idCounter = 0;

function nextId(prefix: string): string {
  idCounter += 1;
  return `${prefix}-${idCounter}`;
}

function updateTurn(ref: TurnRef, update: (turn: MockTurn) => MockTurn): void {
  set((state) => ({
    turns: { ...state.turns, [ref.threadId]: (state.turns[ref.threadId] ?? []).map((turn) => (turn.id === ref.turnId ? update(turn) : turn)) },
  }));
}

function pushItem(ref: TurnRef, item: MockItem): void {
  updateTurn(ref, (turn) => ({ ...turn, items: [...turn.items, item] }));
}

function updateItem(ref: TurnRef, itemId: string, update: (item: MockItem) => MockItem): void {
  updateTurn(ref, (turn) => ({ ...turn, items: turn.items.map((item) => (item.id === itemId ? update(item) : item)) }));
}

function pushStep(ref: TurnRef, activityId: string, step: MockStep): void {
  updateItem(ref, activityId, (item) => (item.type === "activity" ? { ...item, steps: [...item.steps, step] } : item));
}

function finishStep(ref: TurnRef, activityId: string, step: MockStep): void {
  updateItem(ref, activityId, (item) =>
    item.type === "activity" ? { ...item, steps: item.steps.map((current) => (current.id === step.id ? step : current)) } : item,
  );
}

function pause(run: Run, ms: number): Promise<void> {
  return new Promise((resolve, reject) => {
    window.setTimeout(() => (run.cancelled ? reject(CANCELLED) : resolve()), ms);
  });
}

async function streamAgentText(ref: TurnRef, run: Run, text: string): Promise<void> {
  const id = nextId("m");
  pushItem(ref, { type: "agent", id, text: "" });
  for (let end = 4; end < text.length + 4; end += 4) {
    await pause(run, 18);
    updateItem(ref, id, (item) => (item.type === "agent" ? { ...item, text: text.slice(0, end) } : item));
  }
}

function waitForApproval(threadId: string, run: Run, approval: MockApproval): Promise<boolean> {
  set((state) => ({ approvals: { ...state.approvals, [threadId]: approval } }));
  setThreadStatus(threadId, "approval");
  return new Promise((resolve, reject) => {
    run.answerApproval = (accepted) => {
      run.answerApproval = null;
      set((state) => {
        const { [threadId]: _answered, ...approvals } = state.approvals;
        return { approvals };
      });
      if (run.cancelled) {
        reject(CANCELLED);
      } else {
        setThreadStatus(threadId, "running");
        resolve(accepted);
      }
    };
  });
}

/** 승인 요청부터 턴 끝까지이다. 처음부터 승인 대기인 mock 스레드도 이 부분을 쓴다. */
async function playPushAndReport(ref: TurnRef, run: Run, approval: MockApproval): Promise<void> {
  const accepted = await waitForApproval(ref.threadId, run, approval);
  if (accepted) {
    const activityId = nextId("a");
    const step: MockStep = { id: nextId("s"), kind: "command", label: approval.command, status: "running", duration: null, output: null };
    pushItem(ref, { type: "activity", id: activityId, steps: [step] });
    await pause(run, 1400);
    finishStep(ref, activityId, { ...step, status: "ok", duration: "1.4초", output: "remote: Create a pull request by visiting the repository page.\n * [new branch]  HEAD -> branch" });
    await pause(run, 400);
    await streamAgentText(ref, run, "변경을 push 했습니다.\n\n- 검사는 통과했습니다.\n- PR 은 아직 만들지 않았습니다. `변경` 패널의 **PR 만들기** 로 만들 수 있습니다.");
  } else {
    await pause(run, 400);
    await streamAgentText(ref, run, "push 를 하지 않았습니다. 변경은 worktree 에 그대로 있습니다. 다음 지시를 기다립니다.");
  }
}

const PLAN_TEXTS = ["관련 파일을 읽는다", "파일을 수정한다", "검사를 실행하고 push 한다"];

/** `activeIndex` 앞의 단계는 끝난 것으로, 그 단계는 진행 중으로 표시한다. */
function planAt(activeIndex: number): PlanStep[] {
  return PLAN_TEXTS.map((text, index) => ({ text, state: index < activeIndex ? "done" : index === activeIndex ? "active" : "todo" }));
}

function setPlan(ref: TurnRef, planId: string, activeIndex: number): void {
  updateItem(ref, planId, (item) => (item.type === "plan" ? { ...item, steps: planAt(activeIndex) } : item));
}

/** 새 메시지에 대한 mock 진행이다: 생각, 읽기, 수정, 검사, 승인 요청, 보고. */
async function playReply(ref: TurnRef, run: Run): Promise<void> {
  await pause(run, 900);
  pushItem(ref, { type: "reasoning", id: nextId("r"), seconds: 3, text: "요청과 관련된 파일을 먼저 읽고, 가장 작은 변경으로 끝낸다." });
  const planId = nextId("p");
  pushItem(ref, { type: "plan", id: planId, steps: planAt(0) });
  await pause(run, 700);
  const activityId = nextId("a");
  pushItem(ref, { type: "activity", id: activityId, steps: [{ id: nextId("s"), kind: "read", label: "src/renderer/sidebar/Sidebar.tsx", status: "ok", duration: null, output: null }] });
  await pause(run, 700);
  pushStep(ref, activityId, { id: nextId("s"), kind: "search", label: "ThreadStatus", status: "ok", duration: null, output: "src/renderer/session/desktopStore.ts:24" });
  await pause(run, 800);
  setPlan(ref, planId, 1);
  pushStep(ref, activityId, { id: nextId("s"), kind: "edit", label: "src/renderer/sidebar/Sidebar.tsx", status: "ok", duration: null, output: null });
  await pause(run, 600);
  setPlan(ref, planId, 2);
  const check: MockStep = { id: nextId("s"), kind: "command", label: "npm run check", status: "running", duration: null, output: null };
  pushStep(ref, activityId, check);
  await pause(run, 1800);
  finishStep(ref, activityId, { ...check, status: "ok", duration: "1.8초", output: "> tsc --noEmit -p tsconfig.node.json && tsc --noEmit -p tsconfig.web.json" });
  pushItem(ref, { type: "changes", id: nextId("c"), files: [{ path: "src/renderer/sidebar/Sidebar.tsx", added: 12, removed: 3 }] });
  await pause(run, 500);
  await playPushAndReport(ref, run, {
    command: "git push origin HEAD",
    cwd: "~/dev/akra-worktrees/current",
    reason: "네트워크에 쓰는 명령입니다. 현재 권한(승인 후 실행)에서는 허용이 필요합니다.",
  });
  setPlan(ref, planId, PLAN_TEXTS.length);
}

function finishRun(ref: TurnRef, status: MockTurn["status"]): void {
  runs.delete(ref.threadId);
  updateTurn(ref, (turn) => ({
    ...turn,
    status,
    summary: status === "done" ? "12초 · 2.3k 토큰 (mock)" : null,
    items: turn.items.map((item) =>
      item.type === "activity" ? { ...item, steps: item.steps.map((step) => (step.status === "running" ? { ...step, status: "failed" as const } : step)) } : item,
    ),
  }));
  set((state) => {
    const { [ref.threadId]: _started, ...runStartedAt } = state.runStartedAt;
    const { [ref.threadId]: _approval, ...approvals } = state.approvals;
    return { runStartedAt, approvals };
  });
  const watching = get().view === "thread" && get().openThreadId === ref.threadId;
  setThreadStatus(ref.threadId, watching ? "idle" : "unread");
  const [next, ...rest] = get().queued[ref.threadId] ?? [];
  if (next !== undefined && status === "done") {
    set((state) => ({ queued: { ...state.queued, [ref.threadId]: rest } }));
    startTurn(ref.threadId, next);
  }
}

function startRun(ref: TurnRef, play: (run: Run) => Promise<void>): void {
  const run: Run = { cancelled: false, answerApproval: null };
  runs.set(ref.threadId, run);
  play(run).then(
    () => finishRun(ref, "done"),
    (cause: unknown) => {
      if (cause !== CANCELLED) throw cause;
      finishRun(ref, "interrupted");
    },
  );
}

function startTurn(threadId: string, text: string): void {
  const ref: TurnRef = { threadId, turnId: nextId("turn") };
  const turn: MockTurn = { id: ref.turnId, user: text, attachments: [], items: [], status: "running", summary: null };
  set((state) => ({
    turns: { ...state.turns, [threadId]: [...(state.turns[threadId] ?? []), turn] },
    runStartedAt: { ...state.runStartedAt, [threadId]: Date.now() },
  }));
  setThreadStatus(threadId, "running");
  startRun(ref, (run) => playReply(ref, run));
}

export function isRunning(threadId: string): boolean {
  const status = get().threads.find((thread) => thread.id === threadId)?.status;
  return status === "running" || status === "approval";
}

/** 턴이 진행 중이면 대기 목록에 넣고, 아니면 새 턴을 시작한다. */
export function sendMessage(threadId: string, text: string): void {
  restoreThread(threadId);
  if (isRunning(threadId)) {
    set((state) => ({ queued: { ...state.queued, [threadId]: [...(state.queued[threadId] ?? []), text] } }));
  } else {
    startTurn(threadId, text);
  }
}

/** 홈 화면에서 보낸 첫 메시지로 새 스레드를 만든다. */
export function startThread(text: string): void {
  const { homeProjectId, homeWorktree } = get();
  const id = nextId("new");
  const thread: MockThread = {
    id,
    projectId: homeProjectId,
    title: text.length > 36 ? `${text.slice(0, 36)}…` : text,
    status: "idle",
    updated: "방금",
    branch: homeWorktree ? `akra/${id}` : "main",
    worktree: homeWorktree,
    added: 0,
    removed: 0,
    pullRequest: null,
    pinned: false,
    done: false,
    snoozedUntil: null,
  };
  set((state) => ({ threads: [thread, ...state.threads], turns: { ...state.turns, [id]: [] } }));
  openThread(id);
  startTurn(id, text);
}

export function interrupt(threadId: string): void {
  const run = runs.get(threadId);
  if (!run) {
    notConnected("중단");
    return;
  }
  run.cancelled = true;
  // 승인을 기다리는 중이면 타이머가 없으므로 여기에서 끝낸다.
  run.answerApproval?.(false);
}

export function answerApproval(threadId: string, accepted: boolean): void {
  runs.get(threadId)?.answerApproval?.(accepted);
}

// 처음부터 승인 대기인 mock 스레드(t2)는 승인 요청에서 멈춘 상태로 시작한다.
startRun({ threadId: "t2", turnId: "t2-turn-1" }, (run) => playPushAndReport({ threadId: "t2", turnId: "t2-turn-1" }, run, T2_APPROVAL));
