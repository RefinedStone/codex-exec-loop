import { BaseWindow, session, WebContentsView } from "electron";

export const DELEGATED_CHAT_PARTITION = "persist:akra-delegated-chat-v1";
export const DELEGATED_CHAT_URL = "https://chatgpt.com/";
const AUTH_ORIGINS = new Set(["https://auth.openai.com", "https://auth0.openai.com"]);
const CHALLENGE_ORIGIN = "https://challenges.cloudflare.com";

export function isDelegatedChatNavigation(url: string, opts: {
  chatOrigin: string;
  manual: boolean;
  mainFrame: boolean;
}): boolean {
  try {
    const destination = new URL(url);
    if (destination.protocol !== "https:" || destination.username || destination.password) return false;
    return destination.origin === opts.chatOrigin
      || (opts.manual && AUTH_ORIGINS.has(destination.origin))
      || (!opts.mainFrame && destination.origin === CHALLENGE_ORIGIN);
  } catch {
    return false;
  }
}

/** This view has no preload, tools, or renderer-facing automation interface. */
export class DelegatedChatWindow {
  private window: BaseWindow | undefined;
  private view: WebContentsView | undefined;
  private manual = false;
  private manualGeneration = 0;
  private policyInstalled = false;
  private readonly origin: string;
  private readonly startUrl: string;

  constructor(startUrl = DELEGATED_CHAT_URL) {
    this.startUrl = startUrl;
    this.origin = new URL(startUrl).origin;
  }

  get isManual(): boolean {
    return this.manual;
  }

  get automationGeneration(): number {
    return this.manualGeneration;
  }

  getContents(): Electron.WebContents {
    if (this.view && !this.view.webContents.isDestroyed()) return this.view.webContents;
    this.window?.destroy();
    const chatSession = session.fromPartition(DELEGATED_CHAT_PARTITION);
    if (!this.policyInstalled) {
      chatSession.setPermissionCheckHandler(() => false);
      chatSession.setPermissionRequestHandler((_contents, _permission, respond) => respond(false));
      chatSession.setDevicePermissionHandler(() => false);
      chatSession.setDisplayMediaRequestHandler((_request, respond) => respond(null));
      chatSession.on("will-download", (event) => event.preventDefault());
      this.policyInstalled = true;
    }
    const window = new BaseWindow({
      width: 1000,
      height: 780,
      title: "Akra · Delegated ChatGPT",
      show: false,
    });
    const view = new WebContentsView({
      webPreferences: {
        session: chatSession,
        sandbox: true,
        contextIsolation: true,
        nodeIntegration: false,
        webSecurity: true,
        webviewTag: false,
        devTools: false,
        backgroundThrottling: false,
      },
    });
    this.window = window;
    this.view = view;
    const contents = view.webContents;
    contents.setWindowOpenHandler(() => ({ action: "deny" }));
    contents.on("will-attach-webview", (event) => event.preventDefault());
    const guard = (event: { url: string; isMainFrame: boolean; preventDefault(): void }) => {
      if (!isDelegatedChatNavigation(event.url, {
        chatOrigin: this.origin,
        manual: this.manual,
        mainFrame: event.isMainFrame,
      })) event.preventDefault();
    };
    contents.on("will-frame-navigate", guard);
    contents.on("will-redirect", guard);
    // A manual login or challenge keeps control until the operator closes the window.
    window.on("close", (event) => {
      if (this.window !== window) return;
      event.preventDefault();
      window.hide();
      this.manual = false;
    });
    window.once("closed", () => {
      if (!contents.isDestroyed()) contents.close();
      if (this.window !== window) return;
      this.window = undefined;
      this.view = undefined;
      this.manual = false;
    });
    window.contentView.addChildView(view);
    const layout = () => {
      const { width, height } = window.getContentBounds();
      view.setBounds({ x: 0, y: 0, width, height });
    };
    window.on("resize", layout);
    layout();
    return contents;
  }

  async openForManualUse(conversationUrl?: string): Promise<void> {
    const contents = this.getContents();
    // OpenAI sign-in and challenge handling belong to the operator while the window is open.
    this.manual = true;
    this.manualGeneration++;
    this.window?.show();
    contents.focus();
    if (conversationUrl) await contents.loadURL(conversationUrl);
    else if (!contents.getURL() || contents.getURL() === "about:blank") await contents.loadURL(this.startUrl);
  }

  close(): void {
    const window = this.window;
    this.window = undefined;
    this.view = undefined;
    this.manual = false;
    this.manualGeneration++;
    window?.destroy();
  }
}
