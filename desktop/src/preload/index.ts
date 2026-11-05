// renderer 에 노출하는 것은 AkraBridge 의 함수뿐이다. Node API 와 ipcRenderer 자체는 노출하지 않는다.
import { contextBridge, ipcRenderer, type IpcRendererEvent } from "electron";

import { type AkraBridge, type ConnectionState, type GitFileDiff, type GitStatusResult, IPC_CHANNELS } from "../shared/bridge";

const bridge: AkraBridge = {
  send(text: string): void {
    ipcRenderer.send(IPC_CHANNELS.send, text);
  },
  onMessage(listener: (text: string) => void): () => void {
    const handler = (_event: IpcRendererEvent, text: string) => listener(text);
    ipcRenderer.on(IPC_CHANNELS.message, handler);
    return () => ipcRenderer.removeListener(IPC_CHANNELS.message, handler);
  },
  onConnectionState(listener: (state: ConnectionState) => void): () => void {
    const handler = (_event: IpcRendererEvent, state: ConnectionState) => listener(state);
    ipcRenderer.on(IPC_CHANNELS.connectionState, handler);
    return () => ipcRenderer.removeListener(IPC_CHANNELS.connectionState, handler);
  },
  ready(): void {
    ipcRenderer.send(IPC_CHANNELS.ready);
  },
  async chooseFolder(): Promise<string | null> {
    const folder: unknown = await ipcRenderer.invoke(IPC_CHANNELS.chooseFolder);
    return typeof folder === "string" ? folder : null;
  },
  gitStatus(cwd: string): Promise<GitStatusResult> {
    return ipcRenderer.invoke(IPC_CHANNELS.gitStatus, cwd) as Promise<GitStatusResult>;
  },
  gitFileDiff(cwd: string, path: string, untracked: boolean): Promise<GitFileDiff> {
    return ipcRenderer.invoke(IPC_CHANNELS.gitFileDiff, cwd, path, untracked) as Promise<GitFileDiff>;
  },
};

contextBridge.exposeInMainWorld("akra", bridge);
