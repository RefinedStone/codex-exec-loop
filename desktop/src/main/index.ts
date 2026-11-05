// Electron main: 창을 만들고, 창마다 Gateway 접속을 하나씩 연결한다. 메시지는 해석하지 않는다.
import { spawn } from "node:child_process";
import { join, resolve } from "node:path";

import { app, BrowserWindow, ipcMain, shell, type WebContents } from "electron";

import { IPC_CHANNELS } from "../shared/bridge";
import { GatewayConnection } from "./gatewayConnection";
import { resolveGatewayEndpoint, resolveGatewayExecutable } from "./gatewayEndpoint";

const connections = new Map<number, GatewayConnection>();

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

function createWindow(): void {
  const window = new BrowserWindow({
    width: 1200,
    height: 800,
    title: "Akra Desktop",
    show: false,
    webPreferences: {
      preload: join(__dirname, "../preload/index.js"),
      sandbox: true,
      contextIsolation: true,
      nodeIntegration: false,
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
  // 접속 상태 알림을 놓치지 않도록 화면이 준비된 뒤 접속한다.
  const contentsId = contents.id;
  contents.on("did-finish-load", () => attachGatewayConnection(contents));
  contents.once("destroyed", () => {
    connections.get(contentsId)?.close();
    connections.delete(contentsId);
  });
  window.once("ready-to-show", () => window.show());

  const devServer = process.env.ELECTRON_RENDERER_URL;
  if (devServer) {
    void window.loadURL(devServer);
  } else {
    void window.loadFile(join(__dirname, "../renderer/index.html"));
  }
}

ipcMain.on(IPC_CHANNELS.send, (event, text: unknown) => {
  if (typeof text === "string") {
    connections.get(event.sender.id)?.send(text);
  }
});

void app.whenReady().then(() => {
  createWindow();
  app.on("activate", () => {
    if (BrowserWindow.getAllWindows().length === 0) {
      createWindow();
    }
  });
});

app.on("window-all-closed", () => {
  // Gateway 데몬은 앱과 별개로 계속 실행된다 (D-06).
  if (process.platform !== "darwin") {
    app.quit();
  }
});
