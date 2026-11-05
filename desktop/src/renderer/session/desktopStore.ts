// 화면 상태이다. 대화 원본은 app-server 가 가지므로 여기에는 화면에 필요한 값만 둔다 (D-05).
import { create } from "zustand";

import type { ConnectionState } from "../../shared/bridge";
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

/** 안정 API 에는 `project/list` 가 없어서 스레드의 cwd 로 작업 폴더를 묶는다. */
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
  setConnection(connection: ConnectionState): void;
  setSession(session: Pick<DesktopState, "gateway" | "versionCheck" | "folders">): void;
  setSessionError(message: string | null): void;
}

export const useDesktopStore = create<DesktopState>((set) => ({
  connection: null,
  gateway: null,
  versionCheck: null,
  folders: [],
  sessionError: null,
  setConnection: (connection) => set({ connection }),
  setSession: (session) => set({ ...session, sessionError: null }),
  setSessionError: (sessionError) => set({ sessionError }),
}));
