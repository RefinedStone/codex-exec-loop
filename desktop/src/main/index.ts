import { revealStoragePath } from "./storageReveal";
// Electron main: Gateway 문자열 중계와 native UI를 소유한다. 확인 프로토콜은 별도 utility process에서 처리한다.
import { spawn } from "node:child_process";
import { lstat } from "node:fs/promises";
import { homedir } from "node:os";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

import { app, BrowserWindow, clipboard, dialog, ipcMain, Menu, type MenuItemConstructorOptions, net, powerMonitor, powerSaveBlocker, safeStorage, screen, shell, type IpcMainInvokeEvent, type WebContents, utilityProcess } from "electron";

import { type BrowserBounds, IPC_CHANNELS, isDotId, isRemoteHostId, type TerminalOpenRequest } from "../shared/bridge";
import { isNotionPageUrl } from "../shared/notionLinks";
import { mergeSettings, shouldKeepAwake, validateSettings } from "../shared/settings";
import { isVaultRestoreTarget, vaultRestoreProfileHold } from "../shared/vaultRestore";
import { isZoomDirection, nextZoom, type ZoomDirection } from "../shared/zoom";
import { GatewayConnection } from "./gatewayConnection";
import { resolveGatewayEndpoint, resolveGatewayExecutable } from "./gatewayEndpoint";
import { spawn as spawnPty } from "node-pty";

import { loadAppSettings, saveAppSettings } from "./appSettings";
import { BrowserPanelService } from "./browserPanel";
import { BrowserAgentService } from "./browserAgentService";
import { BrowserAgentTransport } from "./browserAgentTransport";
import { BrowserAutomation } from "./browserAgent/automation";
import { loadSettings, saveSettings } from "./browserSettings";
import { contextMenuItems } from "./contextMenuItems";
import { DelegatedChatDriver, delegatedChatHome } from "./delegatedChatDriver";
import { chatAccountStatus } from "./delegatedChatAccount";
import { delegatedTunnelStatus } from "./delegatedTunnelStatus";
import { createDelegatedChatPage } from "./delegatedChatPage";
import { startDelegatedChatProcess } from "./delegatedChatProcess";
import { DelegatedChatWindow } from "./delegatedChatWindow";
import { CHAT_CONVERSATION } from "../shared/delegatedChat";
import { isDotsCloudEnabled, setDotsCloudEnabled } from "./dotsSettings";
import { readBranchSummary, readCommitDiff, readGitHubRepository } from "./gitBranch";
import { executePullRequest, executePush, planPullRequest, planPush } from "./gitPublish";
import { executeGitHunkPlan, planGitHunkAction, readGitHunks } from "./gitHunks";
import { editorUrl, executeGitFilePlan, planGitFileAction, readGitFileDiff, readGitStatus, resolveGitFile } from "./gitStatus";
import { readGitHubSummary, readProject, readReviewPrompt } from "./githubService";
import { ProviderCredentialMigration } from "./providerCredentialMigration";
import { LegacyCredentialFiles } from "./providerLegacy";
import { createProviderMigrationSource } from "./providerMigrationSource";
import { openIssueAutomationPolicyFile } from "./issueAutomationPolicyFile";
import { IssueConfirmationQueue, IssueConfirmationService } from "./issueConfirmationService";
import { isLinearIssueUrl } from "../shared/providerLinks";
import { SkillService } from "./skillService";
import { TerminalService } from "./terminalService";
import { VaultRestoreWindows } from "./vaultRestoreWindow";
import { createVaultRestoreSource } from "./vaultRestoreSource";
import { DEFAULT_WINDOW_STATE, loadWindowState, saveWindowState, visibleBounds } from "./windowState";

// 시험용이다. 브라우저 프로필의 쿠키와 설정이 사용자의 실제 앱 데이터 폴더에 쓰이지 않게 한다. ready 전에 정해야 한다.
if (process.env.AKRA_DESKTOP_USER_DATA) {
  app.setPath("userData", process.env.AKRA_DESKTOP_USER_DATA);
}

const connections = new Map<number, GatewayConnection>();
const providerOperations = new Map<number, Set<AbortController>>();
function invalidateProviderOperations(owner: number): void {
  for (const operation of providerOperations.get(owner) ?? []) operation.abort();
  providerOperations.delete(owner);
}
const issueConfirmations = new Map<number, IssueConfirmationService>();
const issueConfirmationQueue = new IssueConfirmationQueue();
/** 원격 호스트(plan 25)의 접속이다. 키는 `창 id:호스트 id` 이다. renderer 가 열라고 한 것만 있다. */
const remoteConnections = new Map<string, GatewayConnection>();
const remoteKey = (contentsId: number, host: string) => `${contentsId}:${host}`;
const terminals = new TerminalService(spawnPty);
const browsers = new BrowserPanelService(installContextMenu);
const browserAgents = new BrowserAgentService({
  command: process.execPath,
  args: [join(__dirname, "browserMcp.js")],
  view: (owner) => {
    const view = browsers.forAgent(owner);
    return view ? { generation: view.generation, visible: view.visible, profileId: view.profileId, url: view.contents.getURL(), title: view.contents.getTitle() } : null;
  },
  automation: (owner, onPause) => {
    const view = browsers.forAgent(owner);
    if (!view?.visible) throw new Error("브라우저 패널을 먼저 여세요.");
    return new BrowserAutomation(view.contents, onPause);
  },
  onViewChange: (listener) => browsers.onViewChange(listener),
  notify: (owner, state) => {
    const window = BrowserWindow.getAllWindows().find((window) => window.webContents.id === owner);
    if (window && !window.webContents.isDestroyed()) window.webContents.send(IPC_CHANNELS.browserAgentState, state);
  },
}, (callbacks) => new BrowserAgentTransport({ command: resolveGatewayExecutable(repositoryRoot()), ...callbacks }));
const browserSettingsPath = () => join(app.getPath("userData"), "browser-settings.json");
// Legacy ciphertext is read only by main after a private helper prepares its exact Gateway profile.
const providerCredentials = new ProviderCredentialMigration({
  files: new LegacyCredentialFiles(app.getPath("userData")),
  secrets: safeStorage,
  source: createProviderMigrationSource({ executable: () => resolveGatewayExecutable(repositoryRoot()) }),
  openExternal: url => shell.openExternal(url),
});
// 스킬 출처 장부, 스냅샷, 복구 저널은 userData 에 둔다 (#2383). 원본은 인증 없이 공개 GitHub 에서만 읽는다.
const skills = new SkillService({
  dataDir: join(app.getPath("userData"), "skill-sources"),
  home: homedir(),
  fetcher: (url, init) => net.fetch(url, { headers: init.headers, signal: init.signal, redirect: init.redirect, credentials: "omit" }),
});
// 지난 실행이 교체하다 멈췄으면 지금 이전 상태로 되돌린다.
skills.recover();

/**
 * 화면 시험용이다. 창을 보이지 않게 만들어 사용자가 쓰는 다른 앱 위로 올라오지 않게 한다.
 * 보이지 않는 창에서도 requestAnimationFrame 과 타이머가 돌도록 background throttling 을 끈다.
 */
const HIDDEN_WINDOWS = process.env.AKRA_DESKTOP_HIDDEN === "1";

/**
 * 창의 자리와 확대 값이다 (plan 26 W1, U-3). 화면 시험이 사용자의 실제 값을 읽거나 바꾸지 않게 한다: 숨긴 창은
 * 시험용 userData 를 정했을 때만 파일을 쓴다.
 */
const windowStatePath = join(app.getPath("userData"), "window-state.json");
const PERSIST_WINDOW_STATE = !HIDDEN_WINDOWS || Boolean(process.env.AKRA_DESKTOP_USER_DATA);
const windowState = PERSIST_WINDOW_STATE ? loadWindowState(windowStatePath) : { ...DEFAULT_WINDOW_STATE };

function persistWindowState(): void {
  if (PERSIST_WINDOW_STATE) saveWindowState(windowStatePath, windowState);
}

/** 앱 설정이다 (plan 26 U2). 창 상태와 같은 규칙으로 저장한다. */
const appSettingsPath = join(app.getPath("userData"), "app-settings.json");
let appSettings = PERSIST_WINDOW_STATE ? loadAppSettings(appSettingsPath) : validateSettings(null);

/** 실행 중인 턴이 있다고 알린 창(webContents id)이다. */
const workingWindows = new Set<number>();
let keepAwakeId: number | null = null;

/** 설정과 창들의 상태에 맞춰 잠자기 방지를 켜거나 끈다 (plan 26 W5). 화면은 꺼질 수 있고, 시스템만 깨어 있다. */
function updateKeepAwake(): void {
  const wanted = shouldKeepAwake(appSettings.keepAwake, workingWindows.size > 0);
  if (wanted && keepAwakeId === null) {
    keepAwakeId = powerSaveBlocker.start("prevent-app-suspension");
  } else if (!wanted && keepAwakeId !== null) {
    powerSaveBlocker.stop(keepAwakeId);
    keepAwakeId = null;
  }
}

/** 화면 위 막대의 높이(CSS 픽셀, `app.css` 의 `.top-bar`)와 macOS 창 단추의 높이이다. */
const TOP_BAR_HEIGHT = 44;
const WINDOW_BUTTON_HEIGHT = 16;

/** macOS 창 단추를 위 막대의 가운데 높이에 둔다. 위 막대는 확대 값만큼 커진다 (plan 26 Z3). */
function windowButtonPosition(zoom: number): { x: number; y: number } {
  return { x: 14, y: Math.max(2, Math.round((TOP_BAR_HEIGHT * zoom - WINDOW_BUTTON_HEIGHT) / 2)) };
}

/** 제목 막대를 화면 위 막대와 합친 창이다 (macOS 의 본 창과 블루프린트 창). dot 창은 기본 제목 막대를 쓴다. */
const insetWindows = new WeakSet<BrowserWindow>();

function applyZoom(window: BrowserWindow): void {
  if (window.isDestroyed()) return;
  window.webContents.setZoomFactor(windowState.zoom);
  if (insetWindows.has(window)) window.setWindowButtonPosition(windowButtonPosition(windowState.zoom));
}

/** 확대 값을 한 단계 바꾼다. 모든 창에 적용하고 저장한다. 창은 배율을 잠깐 보인다 (plan 26 Z1–Z2). */
function stepZoom(direction: ZoomDirection): void {
  windowState.zoom = nextZoom(windowState.zoom, direction);
  for (const window of BrowserWindow.getAllWindows()) {
    applyZoom(window);
    window.webContents.send(IPC_CHANNELS.zoomChanged, windowState.zoom);
  }
  persistWindowState();
}

/** 오른쪽 클릭 메뉴이다 (plan 26 C1). 앱 창과 브라우저 패널의 페이지에 건다. */
function installContextMenu(contents: WebContents): void {
  contents.on("context-menu", (_event, params) => {
    const template = contextMenuItems(params).map<MenuItemConstructorOptions>((item) => {
      switch (item.kind) {
        case "separator":
          return { type: "separator" };
        case "suggestion":
          return { label: item.word, click: () => contents.replaceMisspelling(item.word) };
        case "learn":
          return { label: "사전에 추가", click: () => contents.session.addWordToSpellCheckerDictionary(item.word) };
        case "copyLink":
          return { label: "링크 복사", click: () => clipboard.writeText(item.url) };
        case "edit":
          return { role: item.role, label: item.label, enabled: item.enabled };
      }
    });
    if (template.length > 0) {
      Menu.buildFromTemplate(template).popup({ window: BrowserWindow.fromWebContents(contents) ?? undefined });
    }
  });
}

function repositoryRoot(): string {
  // 개발 중에는 desktop/ 의 부모가 Akra 저장소 루트이다. 패키징 경로는 R8 에서 정한다.
  return resolve(app.getAppPath(), "..");
}

function startGatewayDaemon(): void {
  const executable = resolveGatewayExecutable(repositoryRoot());
  try {
    const child = spawn(executable, [], { detached: true, stdio: "ignore", windowsHide: true });
    child.on("error", (error) => console.error(`akra-gateway could not start: ${error.message}`));
    child.unref();
  } catch (error) {
    console.error(`akra-gateway could not start: ${String(error)}`);
  }
}

function closeRemoteConnections(contentsId: number): void {
  for (const [key, connection] of remoteConnections) {
    if (key.startsWith(`${contentsId}:`)) {
      connection.close();
      remoteConnections.delete(key);
    }
  }
}

/** `host` 가 없으면 창의 `local` 접속이다. 있으면 그 호스트의 접속이고, 메시지와 상태 앞에 호스트 id 를 붙여 보낸다. */
function attachGatewayConnection(contents: WebContents, host?: string): void {
  const messageChannel = host ? IPC_CHANNELS.remoteMessage : IPC_CHANNELS.message;
  const stateChannel = host ? IPC_CHANNELS.remoteState : IPC_CHANNELS.connectionState;
  const prefix = host ? [host] : [];
  if (host) {
    remoteConnections.get(remoteKey(contents.id, host))?.close();
  } else {
    // 화면을 다시 불러오면 renderer 가 initialize 를 처음부터 하므로 접속도 새로 연다. 원격 접속은 renderer 가 다시 연다.
    connections.get(contents.id)?.close();
    closeRemoteConnections(contents.id);
  }
  let endpoint: string;
  try {
    endpoint = resolveGatewayEndpoint();
  } catch (error) {
    // 주소를 정하지 못해도 main 이 멈추지 않게 화면에 이유를 보여 준다.
    const detail = error instanceof Error ? error.message : String(error);
    contents.send(stateChannel, ...prefix, { phase: "disconnected", endpoint: "", detail });
    return;
  }
  const connection = new GatewayConnection({
    endpoint,
    onMessage: (text) => {
      if (!contents.isDestroyed()) {
        contents.send(messageChannel, ...prefix, text);
      }
    },
    onState: (state) => {
      if (!host) {
        issueConfirmations.get(contents.id)?.invalidate();
        invalidateProviderOperations(contents.id);
      }
      if (!contents.isDestroyed()) {
        contents.send(stateChannel, ...prefix, state);
      }
    },
    // 데몬은 `local` 접속이 시작한다.
    ...(host ? { host } : { startDaemon: startGatewayDaemon }),
  });
  if (host) {
    remoteConnections.set(remoteKey(contents.id, host), connection);
  } else {
    connections.set(contents.id, connection);
  }
  connection.start();
}

function isAppUrl(url: string, contents: WebContents): boolean {
  const devServer = process.env.ELECTRON_RENDERER_URL;
  if (devServer && url.startsWith(devServer)) {
    return true;
  }
  return url.startsWith("file://") && url.split("#")[0] === contents.getURL().split("#")[0];
}

/**
 * UX 블루프린트 화면을 연다 (`npm run blueprint`). mock 데이터만 쓰는 화면이고 Gateway 에 접속하지 않는다.
 * 블루프린트는 `ready` 를 보내지 않으므로 Gateway 접속과 데몬 시작이 일어나지 않는다.
 */
const BLUEPRINT = process.env.AKRA_DESKTOP_BLUEPRINT === "1";
const RENDERER_PAGE = BLUEPRINT ? "blueprint.html" : "index.html";

/** dot 창이다. 키는 `호스트:dot id` 이다. dot 하나에 창 하나만 둔다. `opener` 는 그 창을 연 본 창이다. */
const dotWindows = new Map<string, { window: BrowserWindow; opener: BrowserWindow | null; draft: string | null }>();

/** 창 사이로 옮기는 쓰던 글이다. 길이는 자르지 않는다: 옮기다가 사용자의 글을 잃지 않는다. */
function dotDraftOf(value: unknown): string {
  return typeof value === "string" ? value : "";
}

/** 이 앱을 켠 동안 사용자가 보내기를 허락한 dot 이다 ("세션 한 번 허용"). 본 창과 dot 창이 함께 쓴다. */
const dotSendAllowed = new Set<string>();

function openDotWindowOf(host: string, dotId: string): BrowserWindow | null {
  const entry = dotWindows.get(`${host}:${dotId}`);
  return entry && !entry.window.isDestroyed() ? entry.window : null;
}

/** dot 하나와의 대화만 보이는 작은 창을 연다 (D-19). 같은 renderer 를 `?dotWindow=` 로 연다. */
function openDotWindow(host: string, dotId: string, opener: BrowserWindow | null, draft: string): boolean {
  const key = `${host}:${dotId}`;
  const existing = openDotWindowOf(host, dotId);
  if (existing) {
    if (existing.isMinimized()) existing.restore();
    existing.focus();
    return false;
  }
  const window = createWindow({ dotWindow: host, dot: dotId });
  // 쓰던 글은 새 창이 한 번 가져간다 (`dotsTakeDraft`).
  dotWindows.set(key, { window, opener, draft });
  window.once("closed", () => dotWindows.delete(key));
  return true;
}

/**
 * dot 창의 대화를 본 창의 옆 칸으로 옮긴다. 그 창을 연 본 창으로 보낸다. 그 창이 닫혔으면 다른 본 창으로 보낸다.
 * 본 창이 없으면 아무것도 하지 않는다: dot 창이 그대로 남는다.
 */
function dockDotWindow(host: string, dotId: string, sender: BrowserWindow | null, draft: string): void {
  const entry = dotWindows.get(`${host}:${dotId}`);
  if (!entry || entry.window !== sender) return;
  const isDotWindow = (candidate: BrowserWindow) => [...dotWindows.values()].some((other) => other.window === candidate);
  const target = entry.opener && !entry.opener.isDestroyed() ? entry.opener : (BrowserWindow.getAllWindows().find((candidate) => !isDotWindow(candidate)) ?? null);
  if (!target) return;
  target.webContents.send(IPC_CHANNELS.dotsDocked, host, dotId, draft);
  if (target.isMinimized()) target.restore();
  target.focus();
  entry.window.close();
}

const WINDOW_STATE_SAVE_DELAY_MS = 400;

/** 열려 있는 본 창이다. dot 창과 블루프린트 창은 넣지 않는다. */
const mainWindows = new Set<BrowserWindow>();
const delegatedChatWindow = new DelegatedChatWindow();
let closeDelegatedChatProcess: (() => Promise<void>) | undefined;
let delegatedChatDriver: DelegatedChatDriver | undefined;
const mainRendererPath = join(__dirname, "../renderer/index.html");
function delegatedChatEnabled(): boolean {
  return process.env.AKRA_DESKTOP_DELEGATED_CHAT === "1"
    && !BLUEPRINT && !HIDDEN_WINDOWS && !process.env.ELECTRON_RENDERER_URL
    && !app.commandLine.hasSwitch("remote-debugging-port")
    && !app.commandLine.hasSwitch("remote-debugging-pipe")
    && ![...process.argv, ...process.execArgv].some((arg) => /^--(?:inspect|remote-debugging)/.test(arg))
    && !/(?:^|\s)--inspect/.test(process.env.NODE_OPTIONS ?? "");
}

function isDelegatedChatSender(event: IpcMainInvokeEvent): boolean {
  const owner = BrowserWindow.fromWebContents(event.sender);
  const frame = event.senderFrame;
  return Boolean(owner && mainWindows.has(owner) && frame && frame === event.sender.mainFrame
    && frame.url.split("#")[0] === pathToFileURL(mainRendererPath).href);
}

ipcMain.handle(IPC_CHANNELS.delegatedChatEnabled, (event) => isDelegatedChatSender(event) && delegatedChatEnabled());
ipcMain.handle(IPC_CHANNELS.delegatedChatOpen, async (event, url: unknown) => {
  if (!isDelegatedChatSender(event) || !delegatedChatEnabled()) return false;
  if (url !== undefined && (typeof url !== "string" || !CHAT_CONVERSATION.test(url))) return false;
  try {
    await delegatedChatWindow.openForManualUse(url);
    return true;
  } catch {
    return false;
  }
});

/** `dotQuery` 가 있으면 dot 창이다: 작고, renderer 가 그 dot 의 대화만 그린다. */
function createWindow(dotQuery?: { dotWindow: string; dot: string }): BrowserWindow {
  // 본 창은 지난번의 자리에 연다 (plan 26 W1). 본 창이 이미 있으면 겹치지 않게 기본 자리에 연다.
  const restores = !dotQuery && !BLUEPRINT && mainWindows.size === 0;
  const restored = restores ? visibleBounds(windowState.bounds, screen.getAllDisplays().map((display) => display.workArea)) : null;
  // 창 제목 막대를 화면 위 막대와 합친다 (macOS, plan 26 W2). dot 창은 위 막대가 없어서 기본 제목 막대를 쓴다.
  const inset = !dotQuery && process.platform === "darwin";
  const window = new BrowserWindow({
    width: dotQuery ? 440 : BLUEPRINT ? 1440 : 1200,
    height: dotQuery ? 680 : BLUEPRINT ? 900 : 800,
    ...(restored ?? {}),
    ...(dotQuery ? { minWidth: 340, minHeight: 420 } : {}),
    title: BLUEPRINT ? "Akra Desktop Blueprint" : "Akra Desktop",
    show: false,
    ...(inset ? { titleBarStyle: "hiddenInset" as const, trafficLightPosition: windowButtonPosition(windowState.zoom) } : {}),
    webPreferences: {
      preload: join(__dirname, "../preload/index.js"),
      sandbox: true,
      contextIsolation: true,
      nodeIntegration: false,
      backgroundThrottling: !HIDDEN_WINDOWS,
    },
  });
  const contents = window.webContents;
  if (inset) insetWindows.add(window);
  if (!dotQuery && !BLUEPRINT) {
    mainWindows.add(window);
    // `maximize()` 는 창을 보이게 한다. 그래서 숨긴 창에는 쓰지 않고, 화면을 그린 뒤에 한다.
    if (restored && windowState.maximized && !HIDDEN_WINDOWS) window.once("ready-to-show", () => window.maximize());
    // 마지막으로 옮기거나 닫은 본 창의 자리를 다음 실행에 쓴다. 옮기는 동안에는 멈춘 뒤에 한 번만 쓴다.
    const remember = () => {
      if (window.isDestroyed()) return;
      windowState.bounds = window.getNormalBounds();
      windowState.maximized = window.isMaximized();
      persistWindowState();
    };
    let rememberTimer: NodeJS.Timeout | undefined;
    const rememberSoon = () => {
      clearTimeout(rememberTimer);
      rememberTimer = setTimeout(remember, WINDOW_STATE_SAVE_DELAY_MS);
    };
    window.on("resize", rememberSoon);
    window.on("move", rememberSoon);
    window.on("close", () => {
      clearTimeout(rememberTimer);
      remember();
    });
    window.once("closed", () => {
      mainWindows.delete(window);
      // A hidden chat host must not keep Desktop alive after the last application window closes.
      if (!BrowserWindow.getAllWindows().length && process.platform !== "darwin") app.quit();
    });
  }
  // 확대 값은 페이지를 읽을 때마다 다시 적용한다.
  contents.on("did-finish-load", () => applyZoom(window));
  installContextMenu(contents);

  // 앱 화면 밖으로 이동하지 않는다. http(s) 링크는 기본 브라우저로 연다.
  contents.on("will-navigate", (event, url) => {
    if (!isAppUrl(url, contents)) {
      event.preventDefault();
    }
  });
  contents.setWindowOpenHandler(({ url }) => {
    if (url.startsWith("https://") || url.startsWith("http://")) {
      void shell.openExternal(url);
    }
    return { action: "deny" };
  });
  // 접속은 renderer 가 수신 함수를 등록하고 `ready` 를 보낸 뒤에 연다 (아래 ipcMain 처리).
  const contentsId = contents.id;
  window.on("close", () => {
    issueConfirmations.get(contentsId)?.invalidate();
    vaultRestores.lockOpenedBy(contentsId, "window_closed");
    invalidateProviderOperations(contentsId);
  });
  contents.on("did-start-navigation", (_event, _url, isInPlace, isMainFrame) => {
    if (isMainFrame && !isInPlace) {
      issueConfirmations.get(contentsId)?.invalidate();
      vaultRestores.lockOpenedBy(contentsId, "renderer_changed");
  invalidateProviderOperations(contentsId);
    }
  });
  contents.once("destroyed", () => {
    connections.get(contentsId)?.close();
    connections.delete(contentsId);
    closeRemoteConnections(contentsId);
    closeRendererResources(contentsId);
  });
  // A crashed renderer never sends ready again; close stale native resources.
  contents.on("render-process-gone", () => closeRendererResources(contentsId));
  if (!HIDDEN_WINDOWS) {
    window.once("ready-to-show", () => window.show());
  }

  const devServer = process.env.ELECTRON_RENDERER_URL;
  if (devServer) {
    const query = dotQuery ? `?${new URLSearchParams(dotQuery).toString()}` : "";
    void window.loadURL(`${devServer}/${RENDERER_PAGE}${query}`);
  } else {
    void window.loadFile(join(__dirname, "../renderer", RENDERER_PAGE), dotQuery ? { query: dotQuery } : undefined);
  }
  return window;
}

ipcMain.on(IPC_CHANNELS.send, (event, text: unknown) => {
  if (typeof text === "string") {
    connections.get(event.sender.id)?.send(text);
  }
});

/** Renderer reinitialization does not destroy WebContents: native PTYs and browser
 * views must be released by owner before attaching the replacement renderer. */
function closeRendererResources(contentsId: number): void {
  vaultRestores.lockOpenedBy(contentsId, "renderer_changed");
  invalidateProviderOperations(contentsId);
  issueConfirmations.get(contentsId)?.invalidate();
  issueConfirmations.delete(contentsId);
  terminals.closeAllOf(contentsId);
  browsers.close(contentsId);
  workingWindows.delete(contentsId);
  updateKeepAwake();
}

// renderer 가 상태 수신 함수를 등록한 뒤 보낸다. 화면을 다시 불러오면 다시 오고, 그때 접속을 새로 연다.
ipcMain.on(IPC_CHANNELS.ready, (event) => {
  closeRendererResources(event.sender.id);
  attachGatewayConnection(event.sender);
});

// 원격 호스트의 접속은 renderer 가 열라고 할 때만 연다 (plan 25 §3.3). 사용자가 켠 뒤에만 renderer 가 요청한다.
ipcMain.on(IPC_CHANNELS.remoteOpen, (event, host: unknown) => {
  if (isRemoteHostId(host)) {
    attachGatewayConnection(event.sender, host);
  }
});
ipcMain.on(IPC_CHANNELS.remoteClose, (event, host: unknown) => {
  if (isRemoteHostId(host)) {
    const key = remoteKey(event.sender.id, host);
    remoteConnections.get(key)?.close();
    remoteConnections.delete(key);
  }
});
ipcMain.on(IPC_CHANNELS.remoteSend, (event, host: unknown, text: unknown) => {
  if (isRemoteHostId(host) && typeof text === "string") {
    remoteConnections.get(remoteKey(event.sender.id, host))?.send(text);
  }
});

ipcMain.handle(IPC_CHANNELS.gitStatus, (_event, cwd: unknown) => readGitStatus(cwd));
ipcMain.handle(IPC_CHANNELS.gitFileDiff, (_event, cwd: unknown, path: unknown, untracked: unknown) =>
  readGitFileDiff(cwd, path, untracked),
);
ipcMain.handle(IPC_CHANNELS.gitBranchSummary, (_event, cwd: unknown, fresh: unknown) => readBranchSummary(cwd, fresh));
ipcMain.handle(IPC_CHANNELS.gitCommitDiff, (_event, cwd: unknown, sha: unknown) => readCommitDiff(cwd, sha));
const CANCELLED = { ok: false as const, message: "취소했습니다.", cancelled: true };

/** 사용자에게 확인 창을 띄운다. 실행 버튼을 눌렀을 때만 true 이다. 기본 버튼은 취소이다. */
async function confirmAction(sender: WebContents, type: "warning" | "question", message: string, detail: string, action: string): Promise<boolean> {
  const window = BrowserWindow.fromWebContents(sender);
  const options = { type, message, detail, buttons: [action, "취소"], defaultId: 1, cancelId: 1 };
  const answer = window ? await dialog.showMessageBox(window, options) : await dialog.showMessageBox(options);
  return answer.response === 0;
}

// 변경 버리기는 되돌릴 수 없다. 확인은 main 이 한다. renderer 가 확인을 건너뛸 수 없다.
// main 이 git 에서 읽은 상태로 먼저 확인한다. 거절할 요청에는 확인 창을 띄우지 않고, 창에는 실제로 바뀌는 경로를 보여 준다.
ipcMain.handle(IPC_CHANNELS.gitFileAction, async (event, cwd: unknown, action: unknown, path: unknown) => {
  const planned = await planGitFileAction(cwd, action, path);
  if (!planned.ok) return planned;
  if (action === "discard") {
    const restored = planned.plan.originalPath ? `\n이전 경로를 되살립니다: ${planned.plan.originalPath}` : "";
    const detail = `${planned.plan.path}${restored}\n\n버린 변경은 되돌릴 수 없습니다. 새 파일은 지워집니다.`;
    if (!(await confirmAction(event.sender, "warning", "이 파일의 변경을 버립니까?", detail, "변경 버리기"))) return CANCELLED;
  }
  return executeGitFilePlan(planned.plan);
});
// 부분 버리기도 main 이 확인한다. renderer 는 patch 대신 읽을 때 받은 식별자만 보낸다.
ipcMain.handle(IPC_CHANNELS.gitHunks, (_event, cwd: unknown, path: unknown) => readGitHunks(cwd, path));
ipcMain.handle(IPC_CHANNELS.gitHunkAction, async (event, cwd: unknown, action: unknown, path: unknown, hunkId: unknown) => {
  const planned = await planGitHunkAction(cwd, action, path, hunkId);
  if (!planned.ok) return planned;
  if (planned.plan.action === "discard") {
    const detail = `${planned.plan.path}\n\n선택한 변경 덩어리만 버립니다. 스테이지한 변경은 유지됩니다. 버린 변경은 되돌릴 수 없습니다.`;
    if (!(await confirmAction(event.sender, "warning", "이 부분의 변경을 버립니까?", detail, "변경 버리기"))) return CANCELLED;
  }
  return executeGitHunkPlan(planned.plan);
});

// push 와 PR 만들기는 저장소 밖에 쓴다. main 이 확인하고, 창에 어디에 무엇을 쓰는지 보여 준다.
ipcMain.handle(IPC_CHANNELS.gitPush, async (event, cwd: unknown) => {
  const planned = await planPush(cwd);
  if (!planned.ok) return planned;
  const { branch, remoteUrl } = planned.plan;
  const detail = `브랜치: ${branch}\n원격: ${remoteUrl}\n\n원격 저장소의 ${branch} 브랜치에 커밋을 올립니다. 강제로 덮어쓰지 않습니다.`;
  if (!(await confirmAction(event.sender, "question", "이 브랜치를 push 합니까?", detail, "push"))) return CANCELLED;
  return executePush(planned.plan);
});
ipcMain.handle(IPC_CHANNELS.gitCreatePullRequest, async (event, cwd: unknown, draft: unknown) => {
  const planned = await planPullRequest(cwd, draft);
  if (!planned.ok) return planned;
  const { repository, branch, base, title, login } = planned.plan;
  const detail = `저장소: ${repository}\n${branch} → ${base}\n제목: ${title}\ngh 계정: ${login}${planned.plan.draft ? "\n초안으로 만듭니다." : ""}`;
  if (!(await confirmAction(event.sender, "question", "이 PR 을 만듭니까?", detail, "PR 만들기"))) return CANCELLED;
  return executePullRequest(planned.plan);
});
// `shell.openPath` 는 쓰지 않는다. Windows 에서 `.js`, `.bat` 같은 파일을 실행한다. 에이전트가 만든 파일일 수 있다.
// shortcut: 편집기는 VS Code 주소(`vscode://file/…`)로만 연다. 다른 편집기는 앱 안 편집(P-12)이나 설정이 생기면 더한다.
ipcMain.handle(IPC_CHANNELS.storageReveal, (_event, request: unknown) => revealStoragePath(request, path => shell.showItemInFolder(path)));
ipcMain.handle(IPC_CHANNELS.gitOpenFile, async (_event, cwd: unknown, path: unknown) => {
  const target = await resolveGitFile(cwd, path);
  // 처리기가 없으면 OS 가 앱 선택 창을 띄우고 `openExternal` 은 성공으로 끝난다. 그래서 먼저 등록 여부를 본다.
  if (!app.getApplicationNameForProtocol("vscode://")) {
    shell.showItemInFolder(target);
    return;
  }
  await shell.openExternal(editorUrl(target)).catch(() => shell.showItemInFolder(target));
});


/** 앱 소유 창의 요청과 native 확인만 중계한다. main과 renderer는 프로토콜/증거를 받지 않는다. */
function issueConfirmationOf(event: IpcMainInvokeEvent): IssueConfirmationService {
  const window = BrowserWindow.fromWebContents(event.sender);
  const frame = event.senderFrame;
  if (!window || !mainWindows.has(window) || !frame || frame !== event.sender.mainFrame || !isAppUrl(frame.url, event.sender)) {
    throw new Error("확인을 요청한 앱 창을 확인할 수 없습니다.");
  }
  let service = issueConfirmations.get(event.sender.id);
  if (!service) {
    service = new IssueConfirmationService({
      process: () => utilityProcess.fork(join(__dirname, "issueConfirmation.js"), [], {
        serviceName: "Akra issue confirmation", stdio: "ignore",
      }),
      endpoint: resolveGatewayEndpoint,
      executable: () => resolveGatewayExecutable(repositoryRoot()),
      ownerAlive: () => !window.isDestroyed() && !event.sender.isDestroyed() && event.sender.mainFrame === frame && mainWindows.has(window),
      confirm: async (display, signal) => {
        if (signal.aborted || window.isDestroyed()) return false;
        const answer = await dialog.showMessageBox(window, {
          type: "question",
          message: display.message,
          detail: display.detail,
          buttons: [display.confirm_label, "취소"],
          defaultId: 1,
          cancelId: 1,
          noLink: true,
          signal,
        });
        return !signal.aborted && answer.response === 0;
      },
    }, issueConfirmationQueue);
    issueConfirmations.set(event.sender.id, service);
  }
  return service;
}

ipcMain.handle(IPC_CHANNELS.confirmIssueDecision, (event, request: unknown) => issueConfirmationOf(event).confirmDecision(request));
ipcMain.handle(IPC_CHANNELS.confirmIssueAutomationMode, (event, request: unknown) => issueConfirmationOf(event).confirmMode(request));
ipcMain.handle(IPC_CHANNELS.confirmIssueAutonomy, (event, request: unknown) => issueConfirmationOf(event).confirmAutonomy(request));

ipcMain.handle(IPC_CHANNELS.issueAutomationPolicyOpenFile, async (event, request: unknown) => {
  if (!BrowserWindow.fromWebContents(event.sender) || event.senderFrame !== event.sender.mainFrame) {
    throw new Error("정책 파일 열기 요청을 허용하지 않습니다.");
  }
  await openIssueAutomationPolicyFile(request, {
    platform: process.platform,
    environment: process.env,
    lstat,
    getApplicationNameForProtocol: (protocol) => app.getApplicationNameForProtocol(protocol),
    openExternal: (url) => shell.openExternal(url),
    showItemInFolder: (path) => shell.showItemInFolder(path),
  });
});

ipcMain.handle(IPC_CHANNELS.terminalOpen, (event, request: unknown) => terminals.open(event.sender, request as TerminalOpenRequest));
ipcMain.on(IPC_CHANNELS.terminalWrite, (event, id: unknown, data: unknown) => terminals.write(event.sender.id, id, data));
ipcMain.on(IPC_CHANNELS.terminalResize, (event, id: unknown, cols: unknown, rows: unknown) =>
  terminals.resize(event.sender.id, id, cols, rows),
);
ipcMain.on(IPC_CHANNELS.terminalClose, (event, id: unknown) => terminals.close(event.sender.id, id));

ipcMain.handle(IPC_CHANNELS.browserShow, (event, profileId: unknown, bounds: unknown, url: unknown) => {
  const window = BrowserWindow.fromWebContents(event.sender);
  if (!window || typeof profileId !== "string") throw new Error("브라우저를 열 수 없습니다.");
  return browsers.show(event.sender.id, window, profileId, bounds as BrowserBounds, typeof url === "string" ? url : null);
});
ipcMain.on(IPC_CHANNELS.browserBounds, (event, bounds: unknown) => browsers.setBounds(event.sender.id, bounds as BrowserBounds));
ipcMain.on(IPC_CHANNELS.browserHide, (event) => browsers.hide(event.sender.id));
ipcMain.handle(IPC_CHANNELS.browserNavigate, (event, input: unknown) => {
  browserAgents.pause(event.sender.id, "user_navigation");
  return browsers.navigate(event.sender.id, input);
});
ipcMain.on(IPC_CHANNELS.browserCommand, (event, command: unknown) => {
  browserAgents.pause(event.sender.id, "user_navigation");
  browsers.command(event.sender.id, command);
});
ipcMain.handle(IPC_CHANNELS.browserLoadSettings, () => loadSettings(browserSettingsPath()));
ipcMain.handle(IPC_CHANNELS.browserSaveSettings, (_event, settings: unknown) => saveSettings(browserSettingsPath(), settings));

function browserAgentOwner(event: IpcMainInvokeEvent): number {
  if (!BrowserWindow.fromWebContents(event.sender) || event.senderFrame !== event.sender.mainFrame) throw new Error("브라우저 제어 요청을 허용하지 않습니다.");
  return event.sender.id;
}
function browserIdentifier(value: unknown): string {
  if (typeof value !== "string" || !value.trim() || value.length > 256) throw new Error("잘못된 브라우저 식별자입니다.");
  return value;
}
ipcMain.handle(IPC_CHANNELS.browserAgentPrepare, (event) => browserAgents.prepare(browserAgentOwner(event)));
ipcMain.handle(IPC_CHANNELS.browserAgentBind, (event, id: unknown, thread: unknown) => browserAgents.bind(browserAgentOwner(event), browserIdentifier(id), browserIdentifier(thread)));
ipcMain.handle(IPC_CHANNELS.browserAgentRelease, (event, id: unknown) => browserAgents.release(browserAgentOwner(event), browserIdentifier(id)));
ipcMain.handle(IPC_CHANNELS.browserAgentActiveThread, (event, thread: unknown) => browserAgents.setActiveThread(browserAgentOwner(event), thread === null ? null : browserIdentifier(thread)));
ipcMain.handle(IPC_CHANNELS.browserAgentPause, (event) => browserAgents.pause(browserAgentOwner(event)));
ipcMain.handle(IPC_CHANNELS.browserAgentResume, (event) => browserAgents.resume(browserAgentOwner(event)));
ipcMain.handle(IPC_CHANNELS.browserAgentStatus, (event) => browserAgents.status(browserAgentOwner(event)));

ipcMain.handle(IPC_CHANNELS.githubSummary, () => readGitHubSummary());
ipcMain.handle(IPC_CHANNELS.githubReviewPrompt, (_event, url: unknown) => readReviewPrompt(url));
ipcMain.handle(IPC_CHANNELS.githubProject, (_event, project: unknown) => readProject(project));
ipcMain.handle(IPC_CHANNELS.githubRepository, (_event, cwd: unknown) => readGitHubRepository(cwd));
function requireProviderSender(event: IpcMainInvokeEvent): number {
  if (!BrowserWindow.fromWebContents(event.sender) || event.senderFrame !== event.sender.mainFrame ||
    !connections.has(event.sender.id)) throw new Error("연결 요청을 보낸 앱 창을 확인할 수 없습니다.");
  return event.sender.id;
}
async function providerAction<T>(event: IpcMainInvokeEvent, work: (signal: AbortSignal) => Promise<T>): Promise<T> {
  const owner = requireProviderSender(event);
  const controller = new AbortController();
  const operations = providerOperations.get(owner) ?? new Set<AbortController>();
  providerOperations.set(owner, operations);
  operations.add(controller);
  try { return await work(controller.signal); }
  finally { operations.delete(controller); if (operations.size === 0 && providerOperations.get(owner) === operations) providerOperations.delete(owner); }
}
ipcMain.handle(IPC_CHANNELS.providerLegacyStatus, (event, provider: unknown) => {
  requireProviderSender(event);
  return providerCredentials.legacyStatus(provider);
});
ipcMain.handle(IPC_CHANNELS.providerMigrateLegacy, (event, request: unknown) =>
  providerAction(event, signal => providerCredentials.migrateLegacy(request, signal)));
ipcMain.handle(IPC_CHANNELS.providerPrepareDisconnect, (event, request: unknown) =>
  providerAction(event, signal => providerCredentials.prepareLegacyDisconnect(request, signal)));
ipcMain.handle(IPC_CHANNELS.providerCleanupLegacy, (event, request: unknown) =>
  providerAction(event, signal => providerCredentials.cleanupLegacy(request, signal)));
ipcMain.handle(IPC_CHANNELS.providerOpenExternal, (event, request: unknown) =>
  providerAction(event, signal => providerCredentials.openExternal(request, signal)));
// 스킬 파일은 main 만 쓴다 (#2383). 적용과 롤백은 main 이 경로와 커밋을 보여 주고 확인한다. renderer 가 확인을 건너뛸 수 없다.
function requireSkillSender(event: IpcMainInvokeEvent): void {
  if (!BrowserWindow.fromWebContents(event.sender) || event.senderFrame !== event.sender.mainFrame) throw new Error("스킬 요청을 보낸 창을 확인할 수 없습니다.");
}
ipcMain.handle(IPC_CHANNELS.skillsList, (event, request: unknown) => { requireSkillSender(event); return skills.list(request); });
ipcMain.handle(IPC_CHANNELS.skillsLink, (event, identity: unknown, source: unknown) => { requireSkillSender(event); return skills.link(identity, source); });
ipcMain.handle(IPC_CHANNELS.skillsCheck, (event, identity: unknown) => { requireSkillSender(event); return skills.check(identity); });
ipcMain.handle(IPC_CHANNELS.skillsPreview, (event, identity: unknown) => { requireSkillSender(event); return skills.preview(identity); });
ipcMain.handle(IPC_CHANNELS.skillsApply, (event, identity: unknown, approval: unknown) => {
  requireSkillSender(event);
  return skills.apply(identity, approval, (message, detail) => confirmAction(event.sender, "warning", message, detail, "바꾸기"));
});
ipcMain.handle(IPC_CHANNELS.skillsRollback, (event, identity: unknown, localHash: unknown) => {
  requireSkillSender(event);
  return skills.rollback(identity, localHash, (message, detail) => confirmAction(event.sender, "warning", message, detail, "되돌리기"));
});
ipcMain.handle(
  IPC_CHANNELS.dotsOpenWindow,
  (event, host: unknown, dotId: unknown, draft: unknown) => isRemoteHostId(host) && isDotId(dotId) && openDotWindow(host, dotId, BrowserWindow.fromWebContents(event.sender), dotDraftOf(draft)),
);
ipcMain.handle(IPC_CHANNELS.dotsTakeDraft, (event, host: unknown, dotId: unknown) => {
  if (!isRemoteHostId(host) || !isDotId(dotId)) return null;
  const entry = dotWindows.get(`${host}:${dotId}`);
  if (!entry || entry.window !== BrowserWindow.fromWebContents(event.sender)) return null;
  const draft = entry.draft;
  entry.draft = null;
  return draft;
});
ipcMain.handle(IPC_CHANNELS.dotsHasWindow, (_event, host: unknown, dotId: unknown) => isRemoteHostId(host) && isDotId(dotId) && openDotWindowOf(host, dotId) !== null);
ipcMain.on(IPC_CHANNELS.dotsDock, (event, host: unknown, dotId: unknown, draft: unknown) => {
  if (isRemoteHostId(host) && isDotId(dotId)) {
    dockDotWindow(host, dotId, BrowserWindow.fromWebContents(event.sender), dotDraftOf(draft));
  }
});
ipcMain.on(IPC_CHANNELS.dotsReportSeen, (event, host: unknown, dotId: unknown) => {
  if (!isRemoteHostId(host) || !isDotId(dotId)) return;
  for (const window of BrowserWindow.getAllWindows()) {
    if (window.webContents !== event.sender) window.webContents.send(IPC_CHANNELS.dotsSeen, host, dotId);
  }
});
ipcMain.handle(IPC_CHANNELS.dotsSendAllowed, (_event, dotId: unknown) => isDotId(dotId) && dotSendAllowed.has(dotId));
ipcMain.on(IPC_CHANNELS.dotsAllowSend, (_event, dotId: unknown) => {
  if (isDotId(dotId)) {
    dotSendAllowed.add(dotId);
  }
});
ipcMain.handle(IPC_CHANNELS.dotsGetEnabled, () => isDotsCloudEnabled());
ipcMain.handle(IPC_CHANNELS.dotsSetEnabled, (_event, enabled: unknown) => {
  setDotsCloudEnabled(enabled === true);
  return isDotsCloudEnabled();
});
// Jira 업무 패널의 링크는 별도의 Gateway 검증 경로를 쓴다.
ipcMain.on(IPC_CHANNELS.openExternal, (_event, url: unknown) => {
  if (typeof url !== "string") return;
  if (url.startsWith("https://github.com/") || isNotionPageUrl(url) || isLinearIssueUrl(url)) {
    void shell.openExternal(url);
    return;
  }

});

ipcMain.handle(IPC_CHANNELS.appFocused, () => BrowserWindow.getFocusedWindow() !== null);
ipcMain.handle(IPC_CHANNELS.zoomGet, () => windowState.zoom);
ipcMain.on(IPC_CHANNELS.zoomStep, (_event, direction: unknown) => {
  if (isZoomDirection(direction)) stepZoom(direction);
});

ipcMain.handle(IPC_CHANNELS.settingsGet, () => appSettings);
ipcMain.handle(IPC_CHANNELS.settingsSet, (_event, settings: unknown) => {
  // renderer 는 바꾼 부분만 보낸다. 여기서 합치므로 오래된 사본을 가진 창이 다른 설정을 되돌리지 않는다.
  appSettings = mergeSettings(appSettings, settings);
  if (PERSIST_WINDOW_STATE) saveAppSettings(appSettingsPath, appSettings);
  for (const window of BrowserWindow.getAllWindows()) {
    window.webContents.send(IPC_CHANNELS.settingsChanged, appSettings);
  }
  updateKeepAwake();
  return appSettings;
});
ipcMain.on(IPC_CHANNELS.reportWorking, (event, working: unknown) => {
  if (working === true) {
    workingWindows.add(event.sender.id);
    vaultRestores.lockAll("agent_working");
  } else {
    workingWindows.delete(event.sender.id);
  }
  updateKeepAwake();
});

/**
 * 일반 창은 현재 workspace·이슈 좌표·URL로 전용 창만 연다. 복원한 글은 전용 채널로만 간다.
 * native helper의 OS 확인 뒤에 프로필을 다시 검사한다. 일반 Gateway 메시지는 해석하지 않는다.
 */
const vaultRestores = new VaultRestoreWindows({
  source: createVaultRestoreSource({ executable: () => resolveGatewayExecutable(repositoryRoot()) }),
  profileHold: () =>
    vaultRestoreProfileHold({
      automation:
        app.commandLine.hasSwitch("remote-debugging-port") ||
        app.commandLine.hasSwitch("remote-debugging-pipe") ||
        [...process.argv, ...process.execArgv].some((argument) => /^--(inspect|remote-debugging)/.test(argument)),
      devRenderer: Boolean(process.env.ELECTRON_RENDERER_URL),
      hiddenWindows: HIDDEN_WINDOWS,
      agentWorking: workingWindows.size > 0,
    }),
  preloadPath: join(__dirname, "../preload/vaultRestore.js"),
  // 개발 서버의 화면은 프로필 검사가 보류한다. 그래서 여기서는 빌드한 파일만 읽는다.
  loadPage: (window) => void window.loadFile(join(__dirname, "../renderer/vault-restore.html")),
  hidden: HIDDEN_WINDOWS,
});
ipcMain.handle(IPC_CHANNELS.vaultRestoreOpen, (event, target: unknown) => {
  const opener = BrowserWindow.fromWebContents(event.sender);
  if (!opener || !mainWindows.has(opener) || event.senderFrame !== event.sender.mainFrame) throw new Error("복원본 요청을 보낸 창을 확인할 수 없습니다.");
  if (!isVaultRestoreTarget(target)) throw new Error("현재 프로젝트와 GitHub 이슈 좌표를 확인할 수 없습니다.");
  vaultRestores.open(opener, target);
});
ipcMain.on(IPC_CHANNELS.vaultRestoreProjectChanged, (event) => {
  const opener = BrowserWindow.fromWebContents(event.sender);
  if (opener && mainWindows.has(opener) && event.senderFrame === event.sender.mainFrame) {
    vaultRestores.lockOpenedBy(event.sender.id, "project_switch");
  }
});

ipcMain.handle(IPC_CHANNELS.chooseFolder, async (event) => {
  const owner = BrowserWindow.fromWebContents(event.sender);
  const options = { title: "작업 폴더 선택", properties: ["openDirectory" as const] };
  const result = owner ? await dialog.showOpenDialog(owner, options) : await dialog.showOpenDialog(options);
  return result.canceled ? null : (result.filePaths[0] ?? null);
});

/**
 * 앱 메뉴이다. "새 창" 은 창을 하나 더 열고, 그 창은 자기 Gateway 접속을 가진다 (04 §4). 같은 스레드를 여러 창에서
 * 열면 승인 요청은 모든 창에 오고, 한 창이 답하면 나머지 창에서는 `serverRequest/resolved` 로 사라진다.
 */
function installApplicationMenu(): void {
  // 설정 화면은 renderer 가 그린다. 포커스가 있는 본 창에 열라고 알린다 (plan 26 S1).
  const openSettingsItem: MenuItemConstructorOptions = {
    id: "open-settings",
    label: "설정…",
    accelerator: "CmdOrCtrl+,",
    click: (_item, window) => {
      const target = window instanceof BrowserWindow && mainWindows.has(window) ? window : [...mainWindows][0];
      if (!target) return;
      if (target.isMinimized()) target.restore();
      target.webContents.send(IPC_CHANNELS.settingsOpen);
      target.focus();
    },
  };
  const menu = Menu.buildFromTemplate([
    ...(process.platform === "darwin"
      ? [
          {
            label: app.name,
            submenu: [
              { role: "about" as const },
              { type: "separator" as const },
              openSettingsItem,
              { type: "separator" as const },
              { role: "services" as const },
              { type: "separator" as const },
              { role: "hide" as const },
              { role: "hideOthers" as const },
              { role: "unhide" as const },
              { type: "separator" as const },
              { role: "quit" as const },
            ],
          },
        ]
      : []),
    {
      label: "파일",
      submenu: [
        ...(process.platform === "darwin" ? [] : [openSettingsItem, { type: "separator" as const }]),
        { id: "new-window", label: "새 창", accelerator: "CmdOrCtrl+Shift+N", click: () => createWindow() },
        { type: "separator" },
        { role: "close", label: "창 닫기" },
      ],
    },
    { role: "editMenu" },
    {
      label: "보기",
      submenu: [
        { id: "zoom-in", label: "확대", accelerator: "CmdOrCtrl+Plus", click: () => stepZoom("in") },
        // 미국 자판에서 Shift 없이 누르는 글쇠는 `=` 이다. 같은 동작을 숨긴 항목으로 한 번 더 건다.
        { label: "확대", accelerator: "CmdOrCtrl+=", visible: false, acceleratorWorksWhenHidden: true, click: () => stepZoom("in") },
        { id: "zoom-out", label: "축소", accelerator: "CmdOrCtrl+-", click: () => stepZoom("out") },
        { id: "zoom-reset", label: "실제 크기", accelerator: "CmdOrCtrl+0", click: () => stepZoom("reset") },
        { type: "separator" },
        { role: "reload" },
        { role: "toggleDevTools" },
        { type: "separator" },
        { role: "togglefullscreen" },
      ],
    },
    { role: "windowMenu" },
  ]);
  Menu.setApplicationMenu(menu);
}

void app.whenReady().then(() => {
  installApplicationMenu();
  // OS 화면 잠금과 절전은 열려 있는 로컬 복원본을 다시 잠근다 (#2427).
  powerMonitor.on("lock-screen", () => vaultRestores.lockAll("os_lock"));
  powerMonitor.on("suspend", () => vaultRestores.lockAll("suspend"));
  createWindow();
  if (delegatedChatEnabled()) {
    try {
      const endpoint = resolveGatewayEndpoint();
      const home = delegatedChatHome();
      const driver = new DelegatedChatDriver({
        home,
        pageFor: () => createDelegatedChatPage(delegatedChatWindow),
        pageStatus: chatAccountStatus,
        tunnelStatus: () => delegatedTunnelStatus(home),
        manual: () => delegatedChatWindow.isManual,
        closePage: () => delegatedChatWindow.close(),
      });
      delegatedChatDriver = driver;
      closeDelegatedChatProcess = startDelegatedChatProcess(
        join(__dirname, "delegatedChatGateway.js"), endpoint, driver.dispatch.bind(driver),
      );
    } catch {
      // Main still opens normally if the local Gateway endpoint is unavailable.
    }
  }
  app.on("activate", () => {
    if (BrowserWindow.getAllWindows().length === 0) {
      createWindow();
    }
  });
});

// 셸은 앱과 함께 끝난다. Gateway 데몬과 다르다.
let closingAgents = false;
app.on("before-quit", (event) => {
  for (const service of issueConfirmations.values()) service.invalidate();
  issueConfirmations.clear();
  terminals.closeAll();
  if (closingAgents) return;
  event.preventDefault();
  closingAgents = true;
  if (!delegatedChatDriver) delegatedChatWindow.close();
  void Promise.allSettled([
    closeDelegatedChatProcess?.(),
    delegatedChatDriver?.close(),
    browserAgents.close().catch((error: unknown) => console.error("Browser bridge shutdown failed", error instanceof Error ? error.message : "unknown error")),
  ]).then(() => app.quit());
});

app.on("window-all-closed", () => {
  // Gateway 데몬은 앱과 별개로 계속 실행된다 (D-06).
  if (process.platform !== "darwin") {
    app.quit();
  }
});
