// 화면 틀의 상태이다: 어떤 보기와 패널이 열려 있는지. 대화 상태는 `desktopStore` 에 있다.
import { create } from "zustand";

export type View = "home" | "thread" | "fleet" | "remote" | "dot";
export type PanelId = "review" | "kernel" | "browser" | "work" | "docs";
export type Theme = "light" | "dark";

/** 단축키 표시에 쓰는 보조 키 이름이다. macOS 는 ⌘, 나머지는 Ctrl 이다. */
export const MOD_KEY = navigator.userAgent.includes("Mac") ? "⌘" : "Ctrl+";

export interface Toast {
  id: number;
  text: string;
}

export interface UiState {
  theme: Theme;
  view: View;
  sidebarOpen: boolean;
  panel: PanelId | null;
  terminalOpen: boolean;
  paletteOpen: boolean;
  shortcutsOpen: boolean;
  /** 새 스레드 화면이 고른 작업 폴더이다. null 이면 보낼 때 폴더를 고르게 한다. */
  homeCwd: string | null;
  /** 입력창에 쓰던 글이다. 키는 스레드 id 이고, 새 스레드 화면은 `HOME_DRAFT` 이다. 보기를 바꿔도 남는다. */
  drafts: Record<string, string>;
  toasts: Toast[];
  /** 원격 호스트(클라우드 에이전트, plan 25)의 구역을 켰다. 기본값은 꺼짐이고, 꺼져 있으면 접속하지 않는다. */
  remoteAgents: boolean;
  /** `remote` 와 `dot` 보기가 보여 주는 호스트이다. */
  remoteHost: string | null;
  /** `dot` 보기가 보여 주는 dot 이다. */
  remoteDotId: string | null;
  /**
   * 전송 답을 기다리는 dot 의 id 이다. 화면 밖에 둔다: 답을 기다리는 사이 다른 화면을 보고 와도 "전송 중"이 남는다.
   * 쓰던 글도 화면 밖에 남으므로, 이것이 없으면 같은 글을 한 번 더 보낼 수 있다.
   */
  sendingDotIds: string[];
}

const REMOTE_AGENTS_KEY = "akra.remoteAgents";

function storedRemoteAgents(): boolean {
  try {
    return window.localStorage.getItem(REMOTE_AGENTS_KEY) === "1";
  } catch {
    return false;
  }
}

function initialTheme(): Theme {
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

export const useUiStore = create<UiState>(() => ({
  theme: initialTheme(),
  view: "home",
  sidebarOpen: true,
  panel: null,
  terminalOpen: false,
  paletteOpen: false,
  shortcutsOpen: false,
  homeCwd: null,
  drafts: {},
  toasts: [],
  remoteAgents: storedRemoteAgents(),
  remoteHost: null,
  remoteDotId: null,
  sendingDotIds: [],
}));

const set = useUiStore.setState;

const TOAST_MS = 2800;
let toastCounter = 0;

export function toast(text: string): void {
  toastCounter += 1;
  const id = toastCounter;
  set((state) => ({ toasts: [...state.toasts, { id, text }] }));
  window.setTimeout(() => set((state) => ({ toasts: state.toasts.filter((item) => item.id !== id) })), TOAST_MS);
}

export function showView(view: View): void {
  set({ view, paletteOpen: false });
}

/** 새 스레드 화면을 연다. `cwd` 를 주면 그 폴더를 고른 상태로 연다. */
export function showHome(cwd?: string): void {
  set((state) => ({ view: "home", paletteOpen: false, homeCwd: cwd ?? state.homeCwd }));
}

/** 새 스레드 화면 입력창의 초안 키이다. */
export const HOME_DRAFT = "home";

/** dot 에 보내는 중인지를 적는다. 답이나 오류가 오면 끈다. */
export function setDotSending(dotId: string, sending: boolean): void {
  set((state) => {
    const others = state.sendingDotIds.filter((id) => id !== dotId);
    return { sendingDotIds: sending ? [...others, dotId] : others };
  });
}

export function setDraft(key: string, text: string): void {
  set((state) => ({ drafts: { ...state.drafts, [key]: text } }));
}

/** 글을 새 스레드 화면의 입력창에 채운다. 바로 보내지 않는다. */
export function handOffToNewThread(text: string): void {
  set((state) => ({ view: "home", paletteOpen: false, drafts: { ...state.drafts, [HOME_DRAFT]: text } }));
}

export function togglePanel(panel: PanelId): void {
  set((state) => ({ panel: state.panel === panel ? null : panel, paletteOpen: false }));
}

export function toggleTerminal(): void {
  set((state) => ({ terminalOpen: !state.terminalOpen, paletteOpen: false }));
}

export const REMOTE_AGENTS_WARNING =
  "클라우드 에이전트의 대화를 이 앱에서 읽습니다. 공급자의 약관과 계정 정책이 외부 클라이언트를 허용하는지 먼저 확인하세요. 켭니까?";

/** 원격 호스트 구역을 켜거나 끈다. 켤 때는 경고를 보이고 사용자가 확인해야 켠다 (plan 25 §8). */
export function toggleRemoteAgents(confirm: (message: string) => boolean = (message) => window.confirm(message)): void {
  const enable = !useUiStore.getState().remoteAgents;
  if (enable && !confirm(REMOTE_AGENTS_WARNING)) return;
  setRemoteAgents(enable);
}

function setRemoteAgents(enable: boolean): void {
  try {
    window.localStorage.setItem(REMOTE_AGENTS_KEY, enable ? "1" : "0");
  } catch {
    // 저장하지 못하면 이 창에서만 유지된다.
  }
  set({ remoteAgents: enable, paletteOpen: false });
}

/** 원격 호스트의 대화 화면을 연다. */
export function showRemote(host: string): void {
  set({ view: "remote", remoteHost: host, paletteOpen: false });
}

/** 원격 호스트의 dot 과의 대화 화면을 연다 (D-19). */
export function showDot(host: string, dotId: string): void {
  set({ view: "dot", remoteHost: host, remoteDotId: dotId, paletteOpen: false });
}

export const DOTS_CLOUD_ENABLE_WARNING =
  "dots 클라우드 호스트를 켜거나 끕니다. OpenAI 의 비공개 백엔드에 공식 앱 신원으로 붙습니다. 약관과 계정 위험을 확인하세요. 설정은 Gateway 를 다시 시작해야 적용됩니다. 계속합니까?";

/**
 * dots 클라우드 활성화 플래그와 사이드바의 원격 구역을 함께 켜고 끈다 (plan 25 §3.2). 클라우드 호스트는 Gateway 를
 * 다시 시작해야 생긴다. 그 전에는 구역을 켜도 보일 호스트가 없다.
 */
export async function toggleDotsCloud(
  confirm: (message: string) => boolean = (message) => window.confirm(message),
  notify: (message: string) => void = (message) => window.alert(message),
): Promise<void> {
  set({ paletteOpen: false });
  const current = await window.akra.dots.getEnabled();
  if (!confirm(DOTS_CLOUD_ENABLE_WARNING)) return;
  const enabled = await window.akra.dots.setEnabled(!current);
  // 사이드바의 원격 구역도 같이 켜고 끈다. 경고는 위에서 이미 확인했다. 두 번 켜게 하지 않는다.
  setRemoteAgents(enabled);
  notify(
    enabled
      ? "dots 클라우드를 켰습니다. Gateway 를 다시 시작하면 사이드바에 클라우드 에이전트 구역이 나타납니다."
      : "dots 클라우드를 껐습니다. 사이드바의 구역은 바로 사라집니다. Gateway 는 다시 시작할 때 클라우드 호스트를 내립니다.",
  );
}

/**
 * dots 클라우드는 켜져 있는데 원격 구역을 켜거나 끈 적이 없으면 구역을 켠다. 구역이 플래그와 따로 켜지던 때에
 * dots 를 켠 사용자는 구역이 꺼진 채로 남았다. 사용자가 구역을 직접 끈 적이 있으면 그 선택을 따른다.
 */
export async function showRemoteAgentsWhenDotsAreOn(): Promise<void> {
  let chosen: string | null = null;
  try {
    chosen = window.localStorage.getItem(REMOTE_AGENTS_KEY);
  } catch {
    return;
  }
  if (chosen === null && (await window.akra.dots.getEnabled())) {
    setRemoteAgents(true);
  }
}

export function toggleTheme(): void {
  set((state) => ({ theme: state.theme === "dark" ? "light" : "dark" }));
}
