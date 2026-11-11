// Electron main: 창을 만들고, 창마다 Gateway 접속을 하나씩 연결한다. 메시지는 해석하지 않는다.
import { spawn } from "node:child_process";
import { join, resolve } from "node:path";

import { app, BrowserWindow, dialog, ipcMain, Menu, safeStorage, shell, type WebContents } from "electron";

import { type BrowserBounds, IPC_CHANNELS, type TerminalOpenRequest } from "../shared/bridge";
import { isNotionPageUrl } from "../shared/notionLinks";
import { GatewayConnection } from "./gatewayConnection";
import { resolveGatewayEndpoint, resolveGatewayExecutable } from "./gatewayEndpoint";
import { spawn as spawnPty } from "node-pty";

import { BrowserPanelService } from "./browserPanel";
import { loadSettings, saveSettings } from "./browserSettings";
import { readGitFileDiff, readGitStatus } from "./gitStatus";
import { readGitHubSummary, readReviewPrompt } from "./githubService";
import { JiraService } from "./jiraService";
import { TerminalService } from "./terminalService";

// 시험용이다. 브라우저 프로필의 쿠키와 설정이 사용자의 실제 앱 데이터 폴더에 쓰이지 않게 한다. ready 전에 정해야 한다.
if (process.env.AKRA_DESKTOP_USER_DATA) {
  app.setPath("userData", process.env.AKRA_DESKTOP_USER_DATA);
}

const connections = new Map<number, GatewayConnection>();
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

function attachGatewayConnection(contents: WebContents): void {
  // 화면을 다시 불러오면 renderer 가 initialize 를 처음부터 하므로 접속도 새로 연다.
  connections.get(contents.id)?.close();
  let endpoint: string;
  try {
    endpoint = resolveGatewayEndpoint();
  } catch (error) {
    // 주소를 정하지 못해도 main 이 멈추지 않게 화면에 이유를 보여 준다.
    const detail = error instanceof Error ? error.message : String(error);
    contents.send(IPC_CHANNELS.connectionState, { phase: "disconnected", endpoint: "", detail });
    return;
  }
  const connection = new GatewayConnection({
    endpoint,
    onMessage: (text) => {
      if (!contents.isDestroyed()) {
        contents.send(IPC_CHANNELS.message, text);
      }
    },
    onState: (state) => {
      if (!contents.isDestroyed()) {
        contents.send(IPC_CHANNELS.connectionState, state);
      }
    },
    startDaemon: startGatewayDaemon,
  });
  connections.set(contents.id, connection);
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

function createWindow(): void {
  const window = new BrowserWindow({
    width: BLUEPRINT ? 1440 : 1200,
    height: BLUEPRINT ? 900 : 800,
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
    terminals.closeAllOf(contentsId);
    browsers.close(contentsId);
  });
  if (!HIDDEN_WINDOWS) {
    window.once("ready-to-show", () => window.show());
  }

  const devServer = process.env.ELECTRON_RENDERER_URL;
  if (devServer) {
    void window.loadURL(`${devServer}/${RENDERER_PAGE}`);
  } else {
    void window.loadFile(join(__dirname, "../renderer", RENDERER_PAGE));
  }
}

ipcMain.on(IPC_CHANNELS.send, (event, text: unknown) => {
  if (typeof text === "string") {
    connections.get(event.sender.id)?.send(text);
  }
});

// renderer 가 상태 수신 함수를 등록한 뒤 보낸다. 화면을 다시 불러오면 다시 오고, 그때 접속을 새로 연다.
ipcMain.on(IPC_CHANNELS.ready, (event) => attachGatewayConnection(event.sender));

ipcMain.handle(IPC_CHANNELS.gitStatus, (_event, cwd: unknown) => readGitStatus(cwd));
ipcMain.handle(IPC_CHANNELS.gitFileDiff, (_event, cwd: unknown, path: unknown, untracked: unknown) =>
  readGitFileDiff(cwd, path, untracked),
);

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
ipcMain.handle(IPC_CHANNELS.jiraStatus, () => jira.status());
ipcMain.handle(IPC_CHANNELS.jiraConnect, (_event, request: unknown) => jira.connect(request));
ipcMain.handle(IPC_CHANNELS.jiraDisconnect, () => jira.disconnect());
ipcMain.handle(IPC_CHANNELS.jiraSummary, () => jira.summary());
ipcMain.handle(IPC_CHANNELS.jiraIssuePrompt, (_event, key: unknown) => jira.issuePrompt(key));
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
        { id: "new-window", label: "새 창", accelerator: "CmdOrCtrl+N", click: () => createWindow() },
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
