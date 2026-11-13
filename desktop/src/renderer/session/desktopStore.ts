// 화면 상태이다. 대화 원본은 app-server 가 가지므로 여기에는 화면에 필요한 값만 둔다 (D-05).
import { createContext, useContext } from "react";
import { create } from "zustand";

import { type ConnectionState, isRemoteHostId } from "../../shared/bridge";
import type { PendingApproval } from "../conversation/approvals";
import type { Transcript } from "../conversation/transcript";
import { EMPTY_KERNEL_VIEW, type KernelView } from "../kernel/kernelConnection";
import type { CodexVersionCheck } from "./codexVersion";

export interface GatewayInfo {
  gatewayVersion: string;
  codexVersion: string | null;
  startedAt: string;
  upstreamRestarts: number;
  /** Gateway 가 중계하는 호스트 목록이다 (plan 25). 이전 Gateway 는 보내지 않는다. */
  hosts?: { id: string }[];
}

/** 클라우드 에이전트(dot) 하나이다. Gateway 의 `akra/dots/list` 가 준다 (D-19). */
export interface DotSummary {
  id: string;
  displayName: string;
  status: string;
  kind: string;
  paused: boolean;
  /** dot 의 현재 작업 thread 이다. 없으면 열 수 없다. */
  rootThreadId: string | null;
  /** dot 과 대화하는 메시지 방이다. 없으면 대화를 읽을 수 없다. */
  messagingRoomId: string | null;
}

/** 메시지 방의 실시간 상태이다. Gateway 의 `akra/dots/live` 알림이 준다. */
export interface DotLiveState {
  roomId: string;
  /** 실시간 통로가 붙어 있다. false 이면 나머지 값은 믿을 수 없다. */
  connected: boolean;
  /** dot 이 지금 답을 만들고 있다. */
  working: boolean;
  activityLabel: string | null;
  usingComputer: boolean;
}

export type DotLive = Omit<DotLiveState, "roomId">;

export interface DotLiveSlice {
  /** 방 id → 실시간 상태 */
  dotLive: Record<string, DotLive>;
  /** 방 id → dot 이 답을 끝낸 횟수. 늘면 그 방의 대화를 다시 읽는다. */
  dotReplies: Record<string, number>;
  /** 사용자가 아직 보지 않은 답이 있는 방 id */
  dotUnreadRooms: string[];
}

/**
 * 실시간 상태 하나를 반영한다. "일하는 중"이던 방이 "끝남"이 되면 dot 이 답을 끝낸 것이다: 횟수를 늘리고 그 방을
 * 안 읽음으로 둔다. 통로가 끊겼다는 알림은 답이 아니다.
 */
export function reduceDotLive(previous: DotLiveSlice, next: DotLiveState): DotLiveSlice {
  const { roomId, ...live } = next;
  const finished = previous.dotLive[roomId]?.working === true && live.connected && !live.working;
  return {
    dotLive: { ...previous.dotLive, [roomId]: live },
    dotReplies: finished ? { ...previous.dotReplies, [roomId]: (previous.dotReplies[roomId] ?? 0) + 1 } : previous.dotReplies,
    dotUnreadRooms: finished && !previous.dotUnreadRooms.includes(roomId) ? [...previous.dotUnreadRooms, roomId] : previous.dotUnreadRooms,
  };
}

/** 메시지 방의 메시지 하나이다. Gateway 의 `akra/dots/messages` 가 준다. */
export interface DotMessage {
  id: string;
  createdAt: string;
  /** 보낸 쪽의 이름이다. 모르면 빈 글자이다. */
  author: string;
  fromDot: boolean;
  text: string;
}

/** `local` 이 아닌 호스트 id 를 쉼표로 이은 값이다. 문자열이므로 목록을 다시 읽어도 값이 같으면 화면이 다시 그리지 않는다. */
export function remoteHostKey(state: Pick<DesktopState, "gateway">): string {
  return (state.gateway?.hosts ?? [])
    .map((host) => host.id)
    .filter(isRemoteHostId)
    .join(",");
}

export interface WorkspaceThread {
  id: string;
  title: string;
  /** Unix 초이다. */
  updatedAt: number;
  /** 스레드를 만들 때의 git 브랜치이다. 없으면 null 이다. */
  branch: string | null;
  /** app-server 가 이 스레드를 `systemError` 상태로 알렸다. */
  failed: boolean;
  /** 클라우드 에이전트(dot)가 만든 작업 스레드이다 (`threadSource` 가 `aeon` 으로 시작). 사용자가 만든 클라우드 스레드와 따로 보인다. */
  agentThread?: boolean;
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
  /** 열려 있지 않은 동안 턴이 끝난 스레드 id 이다. 이 창의 메모리에만 있다. 스레드를 열면 빠진다. */
  unreadThreadIds: string[];
  /** 열린 스레드의 대화가 화면에 있다. 새 스레드 화면이나 현황판을 보는 동안은 false 이다. */
  threadVisible: boolean;
  openThreadId: string | null;
  transcript: Transcript | null;
  transcriptError: string | null;
  /** 답을 기다리는 승인 요청이다. 모든 스레드의 것을 모으고, 화면은 스레드별로 골라 보여 준다. */
  pendingApprovals: PendingApproval[];
  setConnection(connection: ConnectionState): void;
  setSession(session: Pick<DesktopState, "gateway" | "versionCheck" | "folders">): void;
  setSessionError(message: string | null): void;
  setThreadActive(threadId: string, active: boolean): void;
  /** `thread/status/changed` 의 `systemError` 여부를 사이드바의 스레드에 반영한다. */
  setThreadFailed(threadId: string, failed: boolean): void;
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
  /** 커널 패널이 보는 저장소 커널의 상태이다 (plan 18 K3-6). */
  kernel: KernelView;
  setKernel(kernel: KernelView): void;
  /** 원격 호스트의 dot 목록이다. `local` 과 dots 가 없는 호스트에서는 비어 있다. */
  dots: DotSummary[];
  setDots(dots: DotSummary[]): void;
  dotLive: DotLiveSlice["dotLive"];
  dotReplies: DotLiveSlice["dotReplies"];
  dotUnreadRooms: DotLiveSlice["dotUnreadRooms"];
  applyDotLive(state: DotLiveState): void;
  /** 사용자가 그 방의 대화를 봤다. */
  markDotRoomRead(roomId: string): void;
}

/** 열린 스레드의 작업 폴더이다. 사이드바 목록에서 찾으므로 기록을 다시 읽는 동안에도 바뀌지 않는다. */
export function openThreadCwd(state: Pick<DesktopState, "folders" | "openThreadId" | "transcript">): string | null {
  if (!state.openThreadId) return null;
  const folder = state.folders.find((candidate) => candidate.threads.some((thread) => thread.id === state.openThreadId));
  return folder?.cwd ?? (state.transcript?.threadId === state.openThreadId ? state.transcript.cwd : null);
}

const createDesktopStore = () => create<DesktopState>((set) => ({
  connection: null,
  gateway: null,
  versionCheck: null,
  folders: [],
  sessionError: null,
  activeThreadIds: [],
  unreadThreadIds: [],
  threadVisible: true,
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
      const watching = state.threadVisible && state.openThreadId === threadId;
      const finishedUnseen = !active && others.length < state.activeThreadIds.length && !watching;
      const unread = state.unreadThreadIds.filter((id) => id !== threadId);
      return { activeThreadIds: active ? [...others, threadId] : others, unreadThreadIds: finishedUnseen ? [...unread, threadId] : unread };
    }),
  setActiveThreads: (activeThreadIds) => set({ activeThreadIds }),
  setThreadFailed: (threadId, failed) =>
    set((state) => {
      const changed = state.folders.some((folder) => folder.threads.some((thread) => thread.id === threadId && thread.failed !== failed));
      if (!changed) return {};
      return { folders: state.folders.map((folder) => ({ ...folder, threads: folder.threads.map((thread) => (thread.id === threadId ? { ...thread, failed } : thread)) })) };
    }),
  setOpenThread: (openThreadId) =>
    set((state) => ({ openThreadId, transcript: null, transcriptError: null, unreadThreadIds: state.unreadThreadIds.filter((id) => id !== openThreadId) })),
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
  kernel: EMPTY_KERNEL_VIEW,
  setKernel: (kernel) => set({ kernel }),
  dots: [],
  setDots: (dots) => set({ dots }),
  dotLive: {},
  dotReplies: {},
  dotUnreadRooms: [],
  applyDotLive: (next) => set((state) => reduceDotLive(state, next)),
  markDotRoomRead: (roomId) => set((state) => (state.dotUnreadRooms.includes(roomId) ? { dotUnreadRooms: state.dotUnreadRooms.filter((id) => id !== roomId) } : {})),
}));

/** `local` 호스트의 상태이다. */
export const useDesktopStore = createDesktopStore();
export type DesktopStore = typeof useDesktopStore;

const remoteHostStores = new Map<string, DesktopStore>();

/**
 * 원격 호스트(plan 25)의 상태이다. 호스트마다 저장소가 따로이다. 그래서 스레드는 `(호스트, 스레드 id)` 로 구분되고,
 * 원격 대화는 `local` 의 패널(변경, 커널, 문서, 터미널)에 닿지 않는다. 메모리에만 있다 (D-05).
 */
export function remoteHostStore(host: string): DesktopStore {
  const store = remoteHostStores.get(host) ?? createDesktopStore();
  remoteHostStores.set(host, store);
  return store;
}

/** 대화 화면이 읽는 저장소이다. 원격 대화를 그릴 때만 그 호스트의 저장소로 바꾼다. */
export const ConversationStoreContext = createContext<DesktopStore>(useDesktopStore);

export function useConversationStore<T>(selector: (state: DesktopState) => T): T {
  return useContext(ConversationStoreContext)(selector);
}

export type SessionStore = Pick<
  DesktopState,
  | "setConnection"
  | "setSession"
  | "setSessionError"
  | "setThreadActive"
  | "setActiveThreads"
  | "setThreadFailed"
  | "setOpenThread"
  | "setTranscript"
  | "updateTranscript"
  | "addApproval"
  | "removeApproval"
  | "clearApprovals"
  | "addThreadToFolder"
  | "mergeFolders"
  | "setKernel"
  | "setDots"
  | "applyDotLive"
> & { getOpenThreadId(): string | null; getTranscript(): Transcript | null; getKernel(): KernelView };

/** zustand 저장소를 세션이 쓰는 좁은 인터페이스로 감싼다. */
export function desktopSessionStore(store: DesktopStore = useDesktopStore): SessionStore {
  const state = store.getState();
  return {
    setConnection: state.setConnection,
    setSession: state.setSession,
    setSessionError: state.setSessionError,
    setThreadActive: state.setThreadActive,
    setActiveThreads: state.setActiveThreads,
    setThreadFailed: state.setThreadFailed,
    setOpenThread: state.setOpenThread,
    setTranscript: state.setTranscript,
    updateTranscript: state.updateTranscript,
    addApproval: state.addApproval,
    removeApproval: state.removeApproval,
    clearApprovals: state.clearApprovals,
    addThreadToFolder: state.addThreadToFolder,
    mergeFolders: state.mergeFolders,
    setKernel: state.setKernel,
    setDots: state.setDots,
    applyDotLive: state.applyDotLive,
    getKernel: () => store.getState().kernel,
    getOpenThreadId: () => store.getState().openThreadId,
    getTranscript: () => store.getState().transcript,
  };
}
