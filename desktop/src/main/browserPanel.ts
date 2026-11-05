// 앱 창 안의 브라우저 보기이다 (R3). 창마다 WebContentsView 를 하나 두고, renderer 가 알려 준 자리에 놓는다.
// 브라우저 보기는 프로필별 세션(persist:akra-browser-<id>)을 쓴다. 앱 화면 renderer 의 세션과 보안 설정을 공유하지 않는다:
// preload 없음, sandbox, contextIsolation, 권한 요청 거절, http(s) 밖으로의 이동 차단.
import { type BrowserWindow, session, type Session, WebContentsView } from "electron";

import { type BrowserBounds, type BrowserState, IPC_CHANNELS } from "../shared/bridge";
import { normalizeUrl, profilePartition } from "./browserSettings";

interface OwnerView {
  view: WebContentsView;
  profileId: string;
  window: BrowserWindow;
}

const hardenedSessions = new WeakSet<Session>();

/** 프로필 세션에 한 번만 건다. 사이트의 카메라, 알림 같은 권한 요청은 모두 거절한다. */
function hardenSession(target: Session): void {
  if (hardenedSessions.has(target)) return;
  hardenedSessions.add(target);
  target.setPermissionRequestHandler((_contents, _permission, callback) => callback(false));
  target.setPermissionCheckHandler(() => false);
}

function isWebUrl(url: string): boolean {
  return url.startsWith("https://") || url.startsWith("http://") || url === "about:blank";
}

function clampBounds(bounds: BrowserBounds): Electron.Rectangle {
  const value = (number: unknown) => (typeof number === "number" && Number.isFinite(number) ? Math.max(0, Math.round(number)) : 0);
  return { x: value(bounds?.x), y: value(bounds?.y), width: value(bounds?.width), height: value(bounds?.height) };
}

export class BrowserPanelService {
  private readonly views = new Map<number, OwnerView>();

  /** 브라우저 보기를 보이거나 프로필을 바꾼다. 프로필이 바뀌면 그 프로필의 세션으로 보기를 새로 만든다. */
  show(ownerId: number, window: BrowserWindow, profileId: string, bounds: BrowserBounds, url: string | null): BrowserState {
    let entry = this.views.get(ownerId);
    if (entry && entry.profileId !== profileId) {
      this.close(ownerId);
      entry = undefined;
    }
    if (!entry) {
      entry = this.create(ownerId, window, profileId);
    }
    entry.view.setBounds(clampBounds(bounds));
    entry.view.setVisible(true);
    if (url) {
      this.navigate(ownerId, url);
    }
    return this.state(entry);
  }

  setBounds(ownerId: number, bounds: BrowserBounds): void {
    this.views.get(ownerId)?.view.setBounds(clampBounds(bounds));
  }

  hide(ownerId: number): void {
    this.views.get(ownerId)?.view.setVisible(false);
  }

  /** 주소창 입력을 연다. 열 수 없는 주소이면 false 이다. */
  navigate(ownerId: number, input: unknown): boolean {
    const entry = this.views.get(ownerId);
    const url = typeof input === "string" ? normalizeUrl(input) : null;
    if (!entry || !url) return false;
    void entry.view.webContents.loadURL(url).catch(() => undefined);
    return true;
  }

  command(ownerId: number, command: unknown): void {
    const contents = this.views.get(ownerId)?.view.webContents;
    if (!contents) return;
    if (command === "back" && contents.navigationHistory.canGoBack()) contents.navigationHistory.goBack();
    if (command === "forward" && contents.navigationHistory.canGoForward()) contents.navigationHistory.goForward();
    if (command === "reload") contents.reload();
    if (command === "stop") contents.stop();
  }

  /** 창이 닫히면 그 창의 브라우저 보기를 닫는다. WebContentsView 의 webContents 는 직접 닫아야 한다. */
  close(ownerId: number): void {
    const entry = this.views.get(ownerId);
    if (!entry) return;
    this.views.delete(ownerId);
    if (!entry.window.isDestroyed()) entry.window.contentView.removeChildView(entry.view);
    if (!entry.view.webContents.isDestroyed()) entry.view.webContents.close();
  }

  private create(ownerId: number, window: BrowserWindow, profileId: string): OwnerView {
    const partition = profilePartition(profileId);
    hardenSession(session.fromPartition(partition));
    const view = new WebContentsView({
      webPreferences: { partition, sandbox: true, contextIsolation: true, nodeIntegration: false },
    });
    const entry: OwnerView = { view, profileId, window };
    this.views.set(ownerId, entry);
    window.contentView.addChildView(view);

    const contents = view.webContents;
    const report = () => {
      const owner = this.views.get(ownerId);
      if (owner === entry && !window.isDestroyed()) {
        window.webContents.send(IPC_CHANNELS.browserState, this.state(entry));
      }
    };
    contents.on("will-navigate", (event, url) => {
      if (!isWebUrl(url)) event.preventDefault();
    });
    // 새 창은 열지 않고 같은 보기에서 연다.
    contents.setWindowOpenHandler(({ url }) => {
      if (isWebUrl(url)) void contents.loadURL(url).catch(() => undefined);
      return { action: "deny" };
    });
    for (const event of ["did-start-loading", "did-stop-loading", "did-navigate", "did-navigate-in-page", "page-title-updated"] as const) {
      contents.on(event as "did-start-loading", report);
    }
    contents.on("did-fail-load", (_event, code, description, url, isMainFrame) => {
      // -3 은 사용자가 다른 주소로 옮겨 가서 취소된 경우이다.
      if (isMainFrame && code !== -3 && !window.isDestroyed()) {
        window.webContents.send(IPC_CHANNELS.browserState, { ...this.state(entry), error: `${description} (${url})` });
      }
    });
    return entry;
  }

  private state(entry: OwnerView): BrowserState {
    const contents = entry.view.webContents;
    return {
      profileId: entry.profileId,
      url: contents.getURL(),
      title: contents.getTitle(),
      loading: contents.isLoading(),
      canGoBack: contents.navigationHistory.canGoBack(),
      canGoForward: contents.navigationHistory.canGoForward(),
      error: null,
    };
  }
}
