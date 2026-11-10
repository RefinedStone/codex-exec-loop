// 화면 상태이다. 대화 원본은 app-server 가 가지므로 여기에는 화면에 필요한 값만 둔다 (D-05).
import { create } from "zustand";

import type { ConnectionState } from "../../shared/bridge";
import type { PendingApproval } from "../conversation/approvals";
import type { Transcript } from "../conversation/transcript";
import { EMPTY_KERNEL_VIEW, type KernelView } from "../kernel/kernelConnection";
import type { CodexVersionCheck } from "./codexVersion";

export interface GatewayInfo {
  gatewayVersion: string;
  codexVersion: string | null;
  startedAt: string;
  upstreamRestarts: number;
}

export interface WorkspaceThread {
  id: string;
  title: string;
  updatedAt: number;
}

/** 안정 API 에는 `project/list` 가 없어서 스레드의 cwd 로 작업 폴더를 묶는다 (D-16). */
export interface WorkspaceFolder {
  cwd: string;
  threads: WorkspaceThread[];
}

/** 폴더 안은 최근 스레드부터, 폴더는 가장 최근 스레드가 있는 폴더부터 놓는다. */
export function sortFolders(folders: WorkspaceFolder[]): WorkspaceFolder[] {
  for (const folder of folders) {
    folder.threads.sort((left, right) => right.updatedAt - left.updatedAt);
  }
  const latest = (folder: WorkspaceFolder) => folder.threads[0]?.updatedAt ?? 0;
  return folders.sort((left, right) => latest(right) - latest(left));
}

/** 이미 보이는 스레드는 그대로 두고, 처음 보는 스레드만 더한다. 이 창에서 만든 새 스레드도 지우지 않는다. */
export function mergeWorkspaceFolders(current: readonly WorkspaceFolder[], incoming: readonly WorkspaceFolder[]): WorkspaceFolder[] {
  const known = new Set(current.flatMap((folder) => folder.threads.map((thread) => thread.id)));
  const byCwd = new Map(current.map((folder) => [folder.cwd, { cwd: folder.cwd, threads: [...folder.threads] }]));
  for (const folder of incoming) {
    for (const thread of folder.threads) {
      if (known.has(thread.id)) continue;
      known.add(thread.id);
      const target = byCwd.get(folder.cwd) ?? { cwd: folder.cwd, threads: [] };
      target.threads.push(thread);
      byCwd.set(folder.cwd, target);
    }
  }
  return sortFolders([...byCwd.values()]);
}

export interface DesktopState {
  connection: ConnectionState | null;
  gateway: GatewayInfo | null;
  versionCheck: CodexVersionCheck | null;
  folders: WorkspaceFolder[];
  sessionError: string | null;
  /** 진행 중인 턴이 있는 스레드 id (사이드바 표시용) */
  activeThreadIds: string[];
  openThreadId: string | null;
  transcript: Transcript | null;
  transcriptError: string | null;
  /** 답을 기다리는 승인 요청이다. 모든 스레드의 것을 모으고, 화면은 스레드별로 골라 보여 준다. */
  pendingApprovals: PendingApproval[];
  setConnection(connection: ConnectionState): void;
  setSession(session: Pick<DesktopState, "gateway" | "versionCheck" | "folders">): void;
  setSessionError(message: string | null): void;
  setThreadActive(threadId: string, active: boolean): void;
  /** thread/list 의 상태로 진행 중 표시를 한 번에 채운다. */
  setActiveThreads(threadIds: string[]): void;
  setOpenThread(threadId: string | null): void;
  setTranscript(transcript: Transcript | null, error?: string | null): void;
  updateTranscript(update: (transcript: Transcript) => Transcript): void;
  addApproval(approval: PendingApproval): void;
  removeApproval(requestId: PendingApproval["requestId"]): void;
  clearApprovals(): void;
  /** 이 창에서 만든 새 스레드를 사이드바의 작업 폴더 맨 앞에 넣는다. */
  addThreadToFolder(cwd: string, thread: WorkspaceThread): void;
  /** 늦게 끝난 전체 목록에서 처음 보는 스레드만 사이드바에 더한다. */
  mergeFolders(folders: WorkspaceFolder[]): void;
  /** 그 스레드의 입력창에 미리 채울 글이다 (업무 패널에서 넘기기). 입력창이 가져가면 비운다. 보내기는 사용자가 한다. */
  composerDraft: { threadId: string; text: string } | null;
  setComposerDraft(draft: { threadId: string; text: string } | null): void;
  /** 커널 패널이 보는 저장소 커널의 상태이다 (plan 18 K3-6). */
  kernel: KernelView;
  setKernel(kernel: KernelView): void;
}

/** 열린 스레드의 작업 폴더이다. 사이드바 목록에서 찾으므로 기록을 다시 읽는 동안에도 바뀌지 않는다. */
export function openThreadCwd(state: Pick<DesktopState, "folders" | "openThreadId" | "transcript">): string | null {
  if (!state.openThreadId) return null;
  const folder = state.folders.find((candidate) => candidate.threads.some((thread) => thread.id === state.openThreadId));
  return folder?.cwd ?? (state.transcript?.threadId === state.openThreadId ? state.transcript.cwd : null);
}

export const useDesktopStore = create<DesktopState>((set) => ({
  connection: null,
  gateway: null,
  versionCheck: null,
  folders: [],
  sessionError: null,
  activeThreadIds: [],
  openThreadId: null,
  transcript: null,
  transcriptError: null,
  pendingApprovals: [],
  setConnection: (connection) => set({ connection }),
  setSession: (session) => set({ ...session, sessionError: null }),
  setSessionError: (sessionError) => set({ sessionError }),
  setThreadActive: (threadId, active) =>
    set((state) => {
      const others = state.activeThreadIds.filter((id) => id !== threadId);
      return { activeThreadIds: active ? [...others, threadId] : others };
    }),
  setActiveThreads: (activeThreadIds) => set({ activeThreadIds }),
  setOpenThread: (openThreadId) => set({ openThreadId, transcript: null, transcriptError: null }),
  setTranscript: (transcript, transcriptError = null) => set({ transcript, transcriptError }),
  updateTranscript: (update) =>
    set((state) => (state.transcript ? { transcript: update(state.transcript) } : {})),
  addApproval: (approval) =>
    set((state) => ({
      pendingApprovals: [
        ...state.pendingApprovals.filter((pending) => String(pending.requestId) !== String(approval.requestId)),
        approval,
      ],
    })),
  removeApproval: (requestId) =>
    set((state) => ({
      pendingApprovals: state.pendingApprovals.filter((pending) => String(pending.requestId) !== String(requestId)),
    })),
  clearApprovals: () => set({ pendingApprovals: [] }),
  addThreadToFolder: (cwd, thread) =>
    set((state) => {
      const existing = state.folders.find((folder) => folder.cwd === cwd);
      const others = state.folders.filter((folder) => folder.cwd !== cwd);
      const threads = [thread, ...(existing?.threads.filter((item) => item.id !== thread.id) ?? [])];
      return { folders: [{ cwd, threads }, ...others] };
    }),
  mergeFolders: (folders) => set((state) => ({ folders: mergeWorkspaceFolders(state.folders, folders) })),
  composerDraft: null,
  setComposerDraft: (composerDraft) => set({ composerDraft }),
  kernel: EMPTY_KERNEL_VIEW,
  setKernel: (kernel) => set({ kernel }),
}));

export type SessionStore = Pick<
  DesktopState,
  | "setConnection"
  | "setSession"
  | "setSessionError"
  | "setThreadActive"
  | "setActiveThreads"
  | "setOpenThread"
  | "setTranscript"
  | "updateTranscript"
  | "addApproval"
  | "removeApproval"
  | "clearApprovals"
  | "addThreadToFolder"
  | "mergeFolders"
  | "setKernel"
> & { getOpenThreadId(): string | null; getTranscript(): Transcript | null; getKernel(): KernelView };

/** zustand 저장소를 세션이 쓰는 좁은 인터페이스로 감싼다. */
export function desktopSessionStore(): SessionStore {
  const state = useDesktopStore.getState();
  return {
    setConnection: state.setConnection,
    setSession: state.setSession,
    setSessionError: state.setSessionError,
    setThreadActive: state.setThreadActive,
    setActiveThreads: state.setActiveThreads,
    setOpenThread: state.setOpenThread,
    setTranscript: state.setTranscript,
    updateTranscript: state.updateTranscript,
    addApproval: state.addApproval,
    removeApproval: state.removeApproval,
    clearApprovals: state.clearApprovals,
    addThreadToFolder: state.addThreadToFolder,
    mergeFolders: state.mergeFolders,
    setKernel: state.setKernel,
    getKernel: () => useDesktopStore.getState().kernel,
    getOpenThreadId: () => useDesktopStore.getState().openThreadId,
    getTranscript: () => useDesktopStore.getState().transcript,
  };
}
