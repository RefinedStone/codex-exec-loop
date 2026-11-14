// Electron main: 창을 만들고, 창마다 Gateway 접속을 하나씩 연결한다. 메시지는 해석하지 않는다.
import { spawn } from "node:child_process";
import { join, resolve } from "node:path";

import { app, BrowserWindow, dialog, ipcMain, Menu, safeStorage, shell, type WebContents } from "electron";

import { type BrowserBounds, IPC_CHANNELS, isDotId, isRemoteHostId, type TerminalOpenRequest } from "../shared/bridge";
import { isNotionPageUrl } from "../shared/notionLinks";
import { GatewayConnection } from "./gatewayConnection";
import { resolveGatewayEndpoint, resolveGatewayExecutable } from "./gatewayEndpoint";
import { spawn as spawnPty } from "node-pty";

import { BrowserPanelService } from "./browserPanel";
import { loadSettings, saveSettings } from "./browserSettings";
import { isDotsCloudEnabled, setDotsCloudEnabled } from "./dotsSettings";
import { readBranchSummary, readCommitDiff, readGitHubRepository } from "./gitBranch";
import { executePullRequest, executePush, planPullRequest, planPush } from "./gitPublish";
import { executeGitHunkPlan, planGitHunkAction, readGitHunks } from "./gitHunks";
import { editorUrl, executeGitFilePlan, planGitFileAction, readGitFileDiff, readGitStatus, resolveGitFile } from "./gitStatus";
import { readGitHubSummary, readIssuePrompt, readProject, readReviewPrompt } from "./githubService";
import { JiraService } from "./jiraService";
import { TerminalService } from "./terminalService";

// 시험용이다. 브라우저 프로필의 쿠키와 설정이 사용자의 실제 앱 데이터 폴더에 쓰이지 않게 한다. ready 전에 정해야 한다.
if (process.env.AKRA_DESKTOP_USER_DATA) {
  app.setPath("userData", process.env.AKRA_DESKTOP_USER_DATA);
}

const connections = new Map<number, GatewayConnection>();
/** 원격 호스트(plan 25)의 접속이다. 키는 `창 id:호스트 id` 이다. renderer 가 열라고 한 것만 있다. */
const remoteConnections = new Map<string, GatewayConnection>();
const remoteKey = (contentsId: number, host: string) => `${contentsId}:${host}`;
const terminals = new TerminalService(spawnPty);
const browsers = new BrowserPanelService();
const browserSettingsPath = () => join(app.getPath("userData"), "browser-settings.json");
// userData 는 위에서 정한 뒤이다. 토큰은 safeStorage 로 암호화한다.
const jira = new JiraService(app.getPath("userData"), safeStorage);

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
 * 화면 시험용이다. 창을 보이지 않게 만들어 사용자가 쓰는 다른 앱 위로 올라오지 않게 한다.
 * 보이지 않는 창에서도 requestAnimationFrame 과 타이머가 돌도록 background throttling 을 끈다.
 */
const HIDDEN_WINDOWS = process.env.AKRA_DESKTOP_HIDDEN === "1";

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

/** `dotQuery` 가 있으면 dot 창이다: 작고, renderer 가 그 dot 의 대화만 그린다. */
function createWindow(dotQuery?: { dotWindow: string; dot: string }): BrowserWindow {
  const window = new BrowserWindow({
    width: dotQuery ? 440 : BLUEPRINT ? 1440 : 1200,
    height: dotQuery ? 680 : BLUEPRINT ? 900 : 800,
    ...(dotQuery ? { minWidth: 340, minHeight: 420 } : {}),
    title: BLUEPRINT ? "Akra Desktop Blueprint" : "Akra Desktop",
    show: false,
    // 블루프린트는 창 제목 막대를 화면 위 막대와 합친다 (macOS).
    ...(BLUEPRINT && process.platform === "darwin" ? { titleBarStyle: "hiddenInset" as const, trafficLightPosition: { x: 14, y: 14 } } : {}),
    webPreferences: {
      preload: join(__dirname, "../preload/index.js"),
      sandbox: true,
      contextIsolation: true,
      nodeIntegration: false,
      backgroundThrottling: !HIDDEN_WINDOWS,
    },
  });
  const contents = window.webContents;

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
  contents.once("destroyed", () => {
    connections.get(contentsId)?.close();
    connections.delete(contentsId);
    closeRemoteConnections(contentsId);
    terminals.closeAllOf(contentsId);
    browsers.close(contentsId);
  });
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

// renderer 가 상태 수신 함수를 등록한 뒤 보낸다. 화면을 다시 불러오면 다시 오고, 그때 접속을 새로 연다.
ipcMain.on(IPC_CHANNELS.ready, (event) => attachGatewayConnection(event.sender));

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
ipcMain.handle(IPC_CHANNELS.gitOpenFile, async (_event, cwd: unknown, path: unknown) => {
  const target = await resolveGitFile(cwd, path);
  // 처리기가 없으면 OS 가 앱 선택 창을 띄우고 `openExternal` 은 성공으로 끝난다. 그래서 먼저 등록 여부를 본다.
  if (!app.getApplicationNameForProtocol("vscode://")) {
    shell.showItemInFolder(target);
    return;
  }
  await shell.openExternal(editorUrl(target)).catch(() => shell.showItemInFolder(target));
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
ipcMain.handle(IPC_CHANNELS.browserNavigate, (event, input: unknown) => browsers.navigate(event.sender.id, input));
ipcMain.on(IPC_CHANNELS.browserCommand, (event, command: unknown) => browsers.command(event.sender.id, command));
ipcMain.handle(IPC_CHANNELS.browserLoadSettings, () => loadSettings(browserSettingsPath()));
ipcMain.handle(IPC_CHANNELS.browserSaveSettings, (_event, settings: unknown) => saveSettings(browserSettingsPath(), settings));

ipcMain.handle(IPC_CHANNELS.githubSummary, () => readGitHubSummary());
ipcMain.handle(IPC_CHANNELS.githubReviewPrompt, (_event, url: unknown) => readReviewPrompt(url));
ipcMain.handle(IPC_CHANNELS.githubProject, (_event, project: unknown) => readProject(project));
ipcMain.handle(IPC_CHANNELS.githubIssuePrompt, (_event, url: unknown) => readIssuePrompt(url));
ipcMain.handle(IPC_CHANNELS.githubRepository, (_event, cwd: unknown) => readGitHubRepository(cwd));
ipcMain.handle(IPC_CHANNELS.jiraStatus, () => jira.status());
ipcMain.handle(IPC_CHANNELS.jiraConnect, (_event, request: unknown) => jira.connect(request));
ipcMain.handle(IPC_CHANNELS.jiraDisconnect, () => jira.disconnect());
ipcMain.handle(IPC_CHANNELS.jiraSummary, () => jira.summary());
ipcMain.handle(IPC_CHANNELS.jiraIssuePrompt, (_event, key: unknown) => jira.issuePrompt(key));
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
// renderer 가 보낸 주소는 github.com, Notion, 연결된 Jira 사이트의 티켓만 기본 브라우저로 연다.
ipcMain.on(IPC_CHANNELS.openExternal, (_event, url: unknown) => {
  if (typeof url !== "string") return;
  if (url.startsWith("https://github.com/") || isNotionPageUrl(url)) {
    void shell.openExternal(url);
    return;
  }
  void jira.isSiteUrl(url).then(async (allowed) => {
    if (allowed) await shell.openExternal(url);
  });
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
  const menu = Menu.buildFromTemplate([
    ...(process.platform === "darwin" ? [{ role: "appMenu" as const }] : []),
    {
      label: "파일",
      submenu: [
        { id: "new-window", label: "새 창", accelerator: "CmdOrCtrl+Shift+N", click: () => createWindow() },
        { type: "separator" },
        { role: "close", label: "창 닫기" },
      ],
    },
    { role: "editMenu" },
    {
      label: "보기",
      submenu: [{ role: "reload" }, { role: "toggleDevTools" }, { type: "separator" }, { role: "togglefullscreen" }],
    },
    { role: "windowMenu" },
  ]);
  Menu.setApplicationMenu(menu);
}

void app.whenReady().then(() => {
  installApplicationMenu();
  createWindow();
  app.on("activate", () => {
    if (BrowserWindow.getAllWindows().length === 0) {
      createWindow();
    }
  });
});

// 셸은 앱과 함께 끝난다. Gateway 데몬과 다르다.
app.on("before-quit", () => terminals.closeAll());

app.on("window-all-closed", () => {
  // Gateway 데몬은 앱과 별개로 계속 실행된다 (D-06).
  if (process.platform !== "darwin") {
    app.quit();
  }
});
