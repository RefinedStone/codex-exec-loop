// 로컬 복원본 창이다 (#2427). 이슈 한 건마다 전용 창 하나를 연다.
//
// 이 창은 일반 표면과 나눈다:
// - 전용 preload(`vaultRestore.js`)만 쓴다. `window.akra`, Gateway 접속, 대화 IPC 가 없다.
// - 브라우저 패널(`BrowserPanelService`)에 등록하지 않는다. 스레드의 브라우저 자동화와 캡처는 그 패널의 보기만 쓴다.
// - DevTools, 다른 주소로의 이동, 새 창을 막는다. 세션은 디스크에 쓰지 않는 전용 partition 이다.
// - 복원본 채널의 요청은 보낸 webContents 가 이 창의 main frame 일 때만 받는다.
// 화면 캡처 제외(`setContentProtection`)는 OS 가 지원할 때만 동작한다. 캡처를 막는다고 보장하지 않는다.
import { BrowserWindow, dialog, ipcMain, type IpcMainEvent, type WebContents } from "electron";

import { type VaultRelockCause, type VaultRestoreHold, VAULT_RESTORE_CHANNELS } from "../shared/vaultRestore";
import { VaultRestoreSession, type VaultRestoreSource } from "./vaultRestoreSession";

/** 복원본을 연 뒤 다시 잠글 때까지의 시간이다. */
const VIEWING_SESSION_MS = 5 * 60_000;

interface Entry {
  window: BrowserWindow;
  session: VaultRestoreSession;
  openerId: number;
  issueUrl: string;
  /** 처음 `ready` 에만 확인을 시작한다. 화면을 다시 불러오면 잠근다. */
  started: boolean;
}

export interface VaultRestoreWindowsDeps {
  source: VaultRestoreSource;
  profileHold(): VaultRestoreHold | null;
  preloadPath: string;
  /** 창에 복원본 화면(`vault-restore.html`)을 읽힌다. */
  loadPage(window: BrowserWindow): void;
  /** 화면 시험용이다. 창을 보이지 않게 둔다. */
  hidden: boolean;
}

export class VaultRestoreWindows {
  /** 키는 복원본 창의 webContents id 이다. */
  private readonly entries = new Map<number, Entry>();

  constructor(private readonly deps: VaultRestoreWindowsDeps) {
    ipcMain.on(VAULT_RESTORE_CHANNELS.ready, (event) => {
      const entry = this.entryOf(event);
      if (!entry) return;
      if (entry.started) return entry.session.lock("user");
      entry.started = true;
      void entry.session.unlock();
    });
    ipcMain.on(VAULT_RESTORE_CHANNELS.unlock, (event) => void this.entryOf(event)?.session.unlock());
    ipcMain.on(VAULT_RESTORE_CHANNELS.lock, (event) => this.entryOf(event)?.session.lock("user"));
    ipcMain.on(VAULT_RESTORE_CHANNELS.review, (event) => void this.entryOf(event)?.session.requestReview());
    ipcMain.on(VAULT_RESTORE_CHANNELS.reconnect, (event, mappings: unknown) => void this.entryOf(event)?.session.reconnect(mappings));
  }

  /** 복원본 창의 main frame 이 보낸 요청만 받는다. 일반 창, 브라우저 패널, iframe 의 요청은 버린다. */
  private entryOf(event: IpcMainEvent): Entry | undefined {
    return event.senderFrame === event.sender.mainFrame ? this.entries.get(event.sender.id) : undefined;
  }

  /** `opener` 가 고른 이슈의 복원본 창을 연다. 같은 창이 같은 이슈를 이미 열었으면 그 창을 앞으로 가져온다. */
  open(opener: BrowserWindow, issueUrl: string): void {
    const openerId = opener.webContents.id;
    for (const entry of this.entries.values()) {
      if (entry.openerId === openerId && entry.issueUrl === issueUrl) {
        if (entry.window.isMinimized()) entry.window.restore();
        return entry.window.focus();
      }
    }
    const window = new BrowserWindow({
      width: 720,
      height: 640,
      minWidth: 420,
      minHeight: 320,
      title: "로컬 복원본 · 읽기 전용",
      show: false,
      webPreferences: {
        preload: this.deps.preloadPath,
        sandbox: true,
        contextIsolation: true,
        nodeIntegration: false,
        devTools: false,
        spellcheck: false,
        // `persist:` 가 없으므로 이 세션은 디스크에 쓰지 않는다.
        partition: "akra-vault-restore",
      },
    });
    window.setContentProtection(true);
    const contents = window.webContents;
    const contentsId = contents.id;
    const session = new VaultRestoreSession({
      issueUrl,
      source: this.deps.source,
      profileHold: this.deps.profileHold,
      confirm: () => confirmViewing(window, issueUrl),
      emit: (state) => {
        if (!contents.isDestroyed()) contents.send(VAULT_RESTORE_CHANNELS.state, state);
      },
      sessionMs: VIEWING_SESSION_MS,
    });
    this.entries.set(contentsId, { window, session, openerId, issueUrl, started: false });
    lockDownNavigation(contents);
    window.on("minimize", () => session.lock("window_hidden"));
    window.on("hide", () => session.lock("window_hidden"));
    window.once("closed", () => {
      session.lock("window_closed");
      this.entries.delete(contentsId);
    });
    // 이 창을 연 창이 닫히면 열람 권한의 근거가 없다. 함께 닫는다.
    opener.once("closed", () => {
      if (!window.isDestroyed()) window.close();
    });
    if (!this.deps.hidden) window.once("ready-to-show", () => window.show());
    this.deps.loadPage(window);
  }

  /** 모든 복원본을 다시 잠근다 (OS 잠금, 절전, 에이전트 턴 시작). */
  lockAll(cause: VaultRelockCause): void {
    for (const entry of this.entries.values()) entry.session.lock(cause);
  }

  /** 그 창이 연 복원본을 다시 잠근다 (프로젝트 전환). */
  lockOpenedBy(openerId: number, cause: VaultRelockCause): void {
    for (const entry of this.entries.values()) {
      if (entry.openerId === openerId) entry.session.lock(cause);
    }
  }
}

function lockDownNavigation(contents: WebContents): void {
  contents.on("will-navigate", (event) => event.preventDefault());
  contents.setWindowOpenHandler(() => ({ action: "deny" }));
}

/** 사용자 확인이다. 복원본 창에 붙은 시스템 대화상자이다. 기본 버튼은 취소이다. */
async function confirmViewing(window: BrowserWindow, issueUrl: string): Promise<boolean> {
  if (window.isDestroyed()) return false;
  const answer = await dialog.showMessageBox(window, {
    type: "question",
    message: "이 이슈의 가린 구간을 이 창에서 엽니까?",
    detail: `${issueUrl}\n\n복원한 글은 이 창에만 보입니다. 대화, 에이전트, GitHub 에는 보내지 않습니다. 같은 사용자 계정의 화면 캡처나 접근성 도구는 이 글을 읽을 수 있습니다.`,
    buttons: ["복원본 열기", "취소"],
    defaultId: 1,
    cancelId: 1,
  });
  return answer.response === 0;
}
