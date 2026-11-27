// renderer 에 노출하는 것은 AkraBridge 의 함수뿐이다. Node API 와 ipcRenderer 자체는 노출하지 않는다.
import { contextBridge, ipcRenderer, type IpcRendererEvent } from "electron";
import type { GitHunkAction, GitHunkDiff } from "../shared/gitHunks";
import type { BrowserAgentState } from "../shared/browserAgent";
import type { ZoomDirection, ZoomFactors, ZoomScope } from "../shared/zoom";

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
  type GitDiffSource,
  type GitStatusResult,
  type IssueAutomationPolicyFileRequest,
  IPC_CHANNELS,
  type TerminalOpenRequest,
  type TerminalOpenResult,
} from "../shared/bridge";

function listen<T extends unknown[]>(channel: string, listener: (...args: T) => void): () => void {
  const handler = (_event: IpcRendererEvent, ...args: unknown[]) => listener(...(args as T));
  ipcRenderer.on(channel, handler);
  return () => ipcRenderer.removeListener(channel, handler);
}

const bridge: AkraBridge = {
  diagnostics: {
    preview: (versions) => ipcRenderer.invoke(IPC_CHANNELS.diagnosticsPreview, versions),
    save: (id) => ipcRenderer.invoke(IPC_CHANNELS.diagnosticsSave, id),
    discard: (id) => ipcRenderer.invoke(IPC_CHANNELS.diagnosticsDiscard, id),
  },
  revealStoragePath: (request) => ipcRenderer.invoke(IPC_CHANNELS.storageReveal, request),
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
  async openFolder(path: string): Promise<boolean> {
    return (await ipcRenderer.invoke(IPC_CHANNELS.openFolder, path)) === true;
  },
  gitStatus(cwd: string): Promise<GitStatusResult> {
    return ipcRenderer.invoke(IPC_CHANNELS.gitStatus, cwd) as Promise<GitStatusResult>;
  },
  gitFileDiff(cwd: string, path: string, untracked: boolean, source?: GitDiffSource): Promise<GitFileDiff> {
    return ipcRenderer.invoke(IPC_CHANNELS.gitFileDiff, cwd, path, untracked, source) as Promise<GitFileDiff>;
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
  openIssueAutomationPolicyFile(request: IssueAutomationPolicyFileRequest): Promise<void> {
    return ipcRenderer.invoke(IPC_CHANNELS.issueAutomationPolicyOpenFile, request) as Promise<void>;
  },
  confirmIssueDecision: (request) => ipcRenderer.invoke(IPC_CHANNELS.confirmIssueDecision, request),
  confirmIssueAutomationMode: (request) => ipcRenderer.invoke(IPC_CHANNELS.confirmIssueAutomationMode, request),
  confirmIssueAutonomy: (request) => ipcRenderer.invoke(IPC_CHANNELS.confirmIssueAutonomy, request),
  confirmIssueBindingResume: (request) => ipcRenderer.invoke(IPC_CHANNELS.confirmIssueBindingResume, request),
  confirmTaskCriterionWaiver: (request) => ipcRenderer.invoke(IPC_CHANNELS.confirmTaskCriterionWaiver, request),
  gitBranchSummary(cwd: string, fresh: boolean): Promise<GitBranchSummary> {
    return ipcRenderer.invoke(IPC_CHANNELS.gitBranchSummary, cwd, fresh) as Promise<GitBranchSummary>;
  },
  gitCommitDiff(cwd: string, sha: string): Promise<GitCommitDiff> {
    return ipcRenderer.invoke(IPC_CHANNELS.gitCommitDiff, cwd, sha) as Promise<GitCommitDiff>;
  },
  gitOperator(cwd, action, branch) { return ipcRenderer.invoke(IPC_CHANNELS.gitOperator, cwd, action, branch); },
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
    agent: {
      prepare: () => ipcRenderer.invoke(IPC_CHANNELS.browserAgentPrepare),
      bind: (id, thread) => ipcRenderer.invoke(IPC_CHANNELS.browserAgentBind, id, thread),
      release: (id) => ipcRenderer.invoke(IPC_CHANNELS.browserAgentRelease, id),
      setActiveThread: (thread) => ipcRenderer.invoke(IPC_CHANNELS.browserAgentActiveThread, thread),
      pause: () => ipcRenderer.invoke(IPC_CHANNELS.browserAgentPause),
      resume: () => ipcRenderer.invoke(IPC_CHANNELS.browserAgentResume),
      status: () => ipcRenderer.invoke(IPC_CHANNELS.browserAgentStatus),
      onState: (listener) => listen<[BrowserAgentState]>(IPC_CHANNELS.browserAgentState, listener),
    },
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
    repository(cwd: string): Promise<string | null> {
      return ipcRenderer.invoke(IPC_CHANNELS.githubRepository, cwd) as Promise<string | null>;
    },
    openExternal(url: string): void {
      ipcRenderer.send(IPC_CHANNELS.openExternal, url);
    },
  },
  providerCredentials: {
    legacyStatus: provider => ipcRenderer.invoke(IPC_CHANNELS.providerLegacyStatus, provider),
    migrateLegacy: target => ipcRenderer.invoke(IPC_CHANNELS.providerMigrateLegacy, target),
    prepareLegacyDisconnect: request => ipcRenderer.invoke(IPC_CHANNELS.providerPrepareDisconnect, request),
    cleanupLegacy: request => ipcRenderer.invoke(IPC_CHANNELS.providerCleanupLegacy, request),
    openExternal: request => ipcRenderer.invoke(IPC_CHANNELS.providerOpenExternal, request),
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
    openWindow(host: string, dotId: string, draft?: string): Promise<boolean> {
      return ipcRenderer.invoke(IPC_CHANNELS.dotsOpenWindow, host, dotId, draft) as Promise<boolean>;
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
    reportSeen(host: string, dotId: string): void {
      ipcRenderer.send(IPC_CHANNELS.dotsReportSeen, host, dotId);
    },
    onSeen(listener: (host: string, dotId: string) => void): () => void {
      const handler = (_event: IpcRendererEvent, host: string, dotId: string) => listener(host, dotId);
      ipcRenderer.on(IPC_CHANNELS.dotsSeen, handler);
      return () => ipcRenderer.removeListener(IPC_CHANNELS.dotsSeen, handler);
    },
    isSendAllowed(dotId: string): Promise<boolean> {
      return ipcRenderer.invoke(IPC_CHANNELS.dotsSendAllowed, dotId) as Promise<boolean>;
    },
    allowSend(dotId: string): void {
      ipcRenderer.send(IPC_CHANNELS.dotsAllowSend, dotId);
    },
  },
  isAppFocused: () => ipcRenderer.invoke(IPC_CHANNELS.appFocused) as Promise<boolean>,
  zoom: {
    get(): Promise<ZoomFactors> {
      return ipcRenderer.invoke(IPC_CHANNELS.zoomGet) as Promise<ZoomFactors>;
    },
    step: (scope, direction) => ipcRenderer.send(IPC_CHANNELS.zoomStep, scope, direction),
    onChanged: (listener) => listen<[ZoomFactors, ZoomScope, boolean]>(IPC_CHANNELS.zoomChanged, listener),
    onCommand: (listener) => listen<[ZoomDirection]>(IPC_CHANNELS.zoomCommand, listener),
  },
  settings: {
    get: () => ipcRenderer.invoke(IPC_CHANNELS.settingsGet),
    set: (settings) => ipcRenderer.invoke(IPC_CHANNELS.settingsSet, settings),
    onChanged: (listener) => listen(IPC_CHANNELS.settingsChanged, listener),
    onOpen: (listener) => listen(IPC_CHANNELS.settingsOpen, listener),
    reportWorking: (working) => ipcRenderer.send(IPC_CHANNELS.reportWorking, working),
  },
  vaultRestore: {
    open: (target) => ipcRenderer.invoke(IPC_CHANNELS.vaultRestoreOpen, target),
    projectChanged: () => ipcRenderer.send(IPC_CHANNELS.vaultRestoreProjectChanged),
  },
  delegatedChat: {
    enabled: () => ipcRenderer.invoke(IPC_CHANNELS.delegatedChatEnabled),
    open: (conversationUrl) => ipcRenderer.invoke(IPC_CHANNELS.delegatedChatOpen, conversationUrl),
  },
  skills: {
    list: (request) => ipcRenderer.invoke(IPC_CHANNELS.skillsList, request),
    link: (identity, source) => ipcRenderer.invoke(IPC_CHANNELS.skillsLink, identity, source),
    check: (identity) => ipcRenderer.invoke(IPC_CHANNELS.skillsCheck, identity),
    preview: (identity) => ipcRenderer.invoke(IPC_CHANNELS.skillsPreview, identity),
    apply: (identity, approval) => ipcRenderer.invoke(IPC_CHANNELS.skillsApply, identity, approval),
    rollback: (identity, localHash) => ipcRenderer.invoke(IPC_CHANNELS.skillsRollback, identity, localHash),
  },
};

contextBridge.exposeInMainWorld("akra", bridge);
