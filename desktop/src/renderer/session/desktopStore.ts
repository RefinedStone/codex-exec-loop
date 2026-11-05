// 화면 상태이다. 대화 원본은 app-server 가 가지므로 여기에는 화면에 필요한 값만 둔다 (D-05).
import { create } from "zustand";

import type { ConnectionState } from "../../shared/bridge";
import type { PendingApproval } from "../conversation/approvals";
import type { Transcript } from "../conversation/transcript";
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
  setOpenThread(threadId: string | null): void;
  setTranscript(transcript: Transcript | null, error?: string | null): void;
  updateTranscript(update: (transcript: Transcript) => Transcript): void;
  addApproval(approval: PendingApproval): void;
  removeApproval(requestId: PendingApproval["requestId"]): void;
  clearApprovals(): void;
  /** 이 창에서 만든 새 스레드를 사이드바의 작업 폴더 맨 앞에 넣는다. */
  addThreadToFolder(cwd: string, thread: WorkspaceThread): void;
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
}));

export type SessionStore = Pick<
  DesktopState,
  | "setConnection"
  | "setSession"
  | "setSessionError"
  | "setThreadActive"
  | "setOpenThread"
  | "setTranscript"
  | "updateTranscript"
  | "addApproval"
  | "removeApproval"
  | "clearApprovals"
  | "addThreadToFolder"
> & { getOpenThreadId(): string | null; getTranscript(): Transcript | null };

/** zustand 저장소를 세션이 쓰는 좁은 인터페이스로 감싼다. */
export function desktopSessionStore(): SessionStore {
  const state = useDesktopStore.getState();
  return {
    setConnection: state.setConnection,
    setSession: state.setSession,
    setSessionError: state.setSessionError,
    setThreadActive: state.setThreadActive,
    setOpenThread: state.setOpenThread,
    setTranscript: state.setTranscript,
    updateTranscript: state.updateTranscript,
    addApproval: state.addApproval,
    removeApproval: state.removeApproval,
    clearApprovals: state.clearApprovals,
    addThreadToFolder: state.addThreadToFolder,
    getOpenThreadId: () => useDesktopStore.getState().openThreadId,
    getTranscript: () => useDesktopStore.getState().transcript,
  };
}
