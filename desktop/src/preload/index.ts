// renderer 에 노출하는 것은 AkraBridge 의 함수뿐이다. Node API 와 ipcRenderer 자체는 노출하지 않는다.
import { contextBridge, ipcRenderer, type IpcRendererEvent } from "electron";
import type { GitHunkAction, GitHunkDiff } from "../shared/gitHunks";

import {
  type AkraBridge,
  type BrowserBounds,
  type BrowserCommand,
  type BrowserSettings,
  type BrowserState,
  type ConnectionState,
  type GitHubProject,
  type GitHubSummary,
  type GitActionResult,
  type GitBranchSummary,
  type GitCommitDiff,
  type GitPublishResult,
  type PullRequestDraft,
  type GitFileAction,
  type GitFileDiff,
  type GitStatusResult,
  IPC_CHANNELS,
  type JiraConnectRequest,
  type JiraStatus,
  type JiraSummary,
  type TerminalOpenRequest,
  type TerminalOpenResult,
} from "../shared/bridge";

function listen<T extends unknown[]>(channel: string, listener: (...args: T) => void): () => void {
  const handler = (_event: IpcRendererEvent, ...args: unknown[]) => listener(...(args as T));
  ipcRenderer.on(channel, handler);
  return () => ipcRenderer.removeListener(channel, handler);
}

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
  remote: {
    open: (host) => ipcRenderer.send(IPC_CHANNELS.remoteOpen, host),
    close: (host) => ipcRenderer.send(IPC_CHANNELS.remoteClose, host),
    send: (host, text) => ipcRenderer.send(IPC_CHANNELS.remoteSend, host, text),
    onMessage: (listener) => listen<[string, string]>(IPC_CHANNELS.remoteMessage, listener),
    onConnectionState: (listener) => listen<[string, ConnectionState]>(IPC_CHANNELS.remoteState, listener),
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
  gitFileAction(cwd: string, action: GitFileAction, path: string): Promise<GitActionResult> {
    return ipcRenderer.invoke(IPC_CHANNELS.gitFileAction, cwd, action, path) as Promise<GitActionResult>;
  },
  gitHunks(cwd: string, path: string): Promise<GitHunkDiff> {
    return ipcRenderer.invoke(IPC_CHANNELS.gitHunks, cwd, path);
  },
  gitHunkAction(cwd: string, action: GitHunkAction, path: string, hunkId: string): Promise<GitActionResult> {
    return ipcRenderer.invoke(IPC_CHANNELS.gitHunkAction, cwd, action, path, hunkId);
  },
  gitOpenFile(cwd: string, path: string): Promise<void> {
    return ipcRenderer.invoke(IPC_CHANNELS.gitOpenFile, cwd, path) as Promise<void>;
  },
  gitBranchSummary(cwd: string, fresh: boolean): Promise<GitBranchSummary> {
    return ipcRenderer.invoke(IPC_CHANNELS.gitBranchSummary, cwd, fresh) as Promise<GitBranchSummary>;
  },
  gitCommitDiff(cwd: string, sha: string): Promise<GitCommitDiff> {
    return ipcRenderer.invoke(IPC_CHANNELS.gitCommitDiff, cwd, sha) as Promise<GitCommitDiff>;
  },
  gitPush(cwd: string): Promise<GitPublishResult> {
    return ipcRenderer.invoke(IPC_CHANNELS.gitPush, cwd) as Promise<GitPublishResult>;
  },
  gitCreatePullRequest(cwd: string, draft: PullRequestDraft): Promise<GitPublishResult> {
    return ipcRenderer.invoke(IPC_CHANNELS.gitCreatePullRequest, cwd, draft) as Promise<GitPublishResult>;
  },
  terminal: {
    open(request: TerminalOpenRequest): Promise<TerminalOpenResult> {
      return ipcRenderer.invoke(IPC_CHANNELS.terminalOpen, request) as Promise<TerminalOpenResult>;
    },
    write(id: number, data: string): void {
      ipcRenderer.send(IPC_CHANNELS.terminalWrite, id, data);
    },
    resize(id: number, cols: number, rows: number): void {
      ipcRenderer.send(IPC_CHANNELS.terminalResize, id, cols, rows);
    },
    close(id: number): void {
      ipcRenderer.send(IPC_CHANNELS.terminalClose, id);
    },
    onData: (listener) => listen<[number, string]>(IPC_CHANNELS.terminalData, listener),
    onExit: (listener) => listen<[number, number]>(IPC_CHANNELS.terminalExit, listener),
  },
  browser: {
    show(profileId: string, bounds: BrowserBounds, url: string | null): Promise<BrowserState> {
      return ipcRenderer.invoke(IPC_CHANNELS.browserShow, profileId, bounds, url) as Promise<BrowserState>;
    },
    setBounds(bounds: BrowserBounds): void {
      ipcRenderer.send(IPC_CHANNELS.browserBounds, bounds);
    },
    hide(): void {
      ipcRenderer.send(IPC_CHANNELS.browserHide);
    },
    navigate(input: string): Promise<boolean> {
      return ipcRenderer.invoke(IPC_CHANNELS.browserNavigate, input) as Promise<boolean>;
    },
    command(command: BrowserCommand): void {
      ipcRenderer.send(IPC_CHANNELS.browserCommand, command);
    },
    onState: (listener) => listen<[BrowserState]>(IPC_CHANNELS.browserState, listener),
    loadSettings(): Promise<BrowserSettings> {
      return ipcRenderer.invoke(IPC_CHANNELS.browserLoadSettings) as Promise<BrowserSettings>;
    },
    saveSettings(settings: BrowserSettings): Promise<BrowserSettings> {
      return ipcRenderer.invoke(IPC_CHANNELS.browserSaveSettings, settings) as Promise<BrowserSettings>;
    },
  },
  github: {
    summary(): Promise<GitHubSummary> {
      return ipcRenderer.invoke(IPC_CHANNELS.githubSummary) as Promise<GitHubSummary>;
    },
    reviewPrompt(url: string): Promise<string> {
      return ipcRenderer.invoke(IPC_CHANNELS.githubReviewPrompt, url) as Promise<string>;
    },
    project(project: string): Promise<GitHubProject> {
      return ipcRenderer.invoke(IPC_CHANNELS.githubProject, project) as Promise<GitHubProject>;
    },
    issuePrompt(url: string): Promise<string> {
      return ipcRenderer.invoke(IPC_CHANNELS.githubIssuePrompt, url) as Promise<string>;
    },
    repository(cwd: string): Promise<string | null> {
      return ipcRenderer.invoke(IPC_CHANNELS.githubRepository, cwd) as Promise<string | null>;
    },
    openExternal(url: string): void {
      ipcRenderer.send(IPC_CHANNELS.openExternal, url);
    },
  },
  jira: {
    status(): Promise<JiraStatus> {
      return ipcRenderer.invoke(IPC_CHANNELS.jiraStatus) as Promise<JiraStatus>;
    },
    connect(request: JiraConnectRequest): Promise<JiraStatus> {
      return ipcRenderer.invoke(IPC_CHANNELS.jiraConnect, request) as Promise<JiraStatus>;
    },
    disconnect(): Promise<JiraStatus> {
      return ipcRenderer.invoke(IPC_CHANNELS.jiraDisconnect) as Promise<JiraStatus>;
    },
    summary(): Promise<JiraSummary> {
      return ipcRenderer.invoke(IPC_CHANNELS.jiraSummary) as Promise<JiraSummary>;
    },
    issuePrompt(key: string): Promise<string> {
      return ipcRenderer.invoke(IPC_CHANNELS.jiraIssuePrompt, key) as Promise<string>;
    },
    openExternal(url: string): void {
      ipcRenderer.send(IPC_CHANNELS.openExternal, url);
    },
  },
  notion: {
    openExternal(url: string): void {
      ipcRenderer.send(IPC_CHANNELS.openExternal, url);
    },
  },
  dots: {
    getEnabled(): Promise<boolean> {
      return ipcRenderer.invoke(IPC_CHANNELS.dotsGetEnabled) as Promise<boolean>;
    },
    setEnabled(enabled: boolean): Promise<boolean> {
      return ipcRenderer.invoke(IPC_CHANNELS.dotsSetEnabled, enabled) as Promise<boolean>;
    },
    openWindow(host: string, dotId: string, draft?: string): void {
      ipcRenderer.send(IPC_CHANNELS.dotsOpenWindow, host, dotId, draft);
    },
    takeDraft(host: string, dotId: string): Promise<string | null> {
      return ipcRenderer.invoke(IPC_CHANNELS.dotsTakeDraft, host, dotId) as Promise<string | null>;
    },
    hasWindow(host: string, dotId: string): Promise<boolean> {
      return ipcRenderer.invoke(IPC_CHANNELS.dotsHasWindow, host, dotId) as Promise<boolean>;
    },
    dock(host: string, dotId: string, draft: string): void {
      ipcRenderer.send(IPC_CHANNELS.dotsDock, host, dotId, draft);
    },
    onDock(listener: (host: string, dotId: string, draft: string) => void): () => void {
      const handler = (_event: IpcRendererEvent, host: string, dotId: string, draft: string) => listener(host, dotId, draft);
      ipcRenderer.on(IPC_CHANNELS.dotsDocked, handler);
      return () => ipcRenderer.removeListener(IPC_CHANNELS.dotsDocked, handler);
    },
    isSendAllowed(dotId: string): Promise<boolean> {
      return ipcRenderer.invoke(IPC_CHANNELS.dotsSendAllowed, dotId) as Promise<boolean>;
    },
    allowSend(dotId: string): void {
      ipcRenderer.send(IPC_CHANNELS.dotsAllowSend, dotId);
    },
  },
};

contextBridge.exposeInMainWorld("akra", bridge);
