// 화면 틀의 상태이다: 어떤 보기와 패널이 열려 있는지. 대화 상태는 `desktopStore` 에 있다.
import { create } from "zustand";

export type View = "home" | "thread" | "fleet" | "remote";
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
  /** `remote` 보기가 보여 주는 호스트이다. */
  remoteHost: string | null;
  /**
   * 옆 칸에 열어 둔 dot 과의 대화이다 (D-19). 보기(`view`)와 따로 있다: 스레드로 일하는 동안 옆에 남는다.
   * null 이면 옆 칸이 없다.
   */
  dotDock: { host: string; dotId: string } | null;
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

const THEME_KEY = "akra.theme";

/** 사용자가 고른 밝기이다. 고른 적이 없으면 운영체제의 설정을 따른다. dot 창도 같은 값을 읽는다. */
export function chosenTheme(): Theme {
  try {
    const stored = window.localStorage.getItem(THEME_KEY);
    if (stored === "dark" || stored === "light") return stored;
  } catch {
    // 저장소를 읽지 못하면 운영체제의 설정을 따른다.
  }
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

export const useUiStore = create<UiState>(() => ({
  theme: chosenTheme(),
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
  dotDock: null,
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

/** dot 에게 쓰던 글의 초안 키이다. */
export function dotDraftKey(dotId: string): string {
  return `dot:${dotId}`;
}

export function setDraft(key: string, text: string): void {
  set((state) => ({ drafts: { ...state.drafts, [key]: text } }));
}

/** 글을 새 스레드 화면의 입력창에 채운다. 바로 보내지 않는다. */
export function handOffToNewThread(text: string): void {
  set((state) => ({ view: "home", paletteOpen: false, drafts: { ...state.drafts, [HOME_DRAFT]: text } }));
}

/** 이 폭보다 좁은 창에는 오른쪽 패널과 dot 옆 칸을 함께 둘 자리가 없다. 나중에 연 것만 남긴다. */
const TWO_SIDE_COLUMNS_MIN_WIDTH = 1400;

function isNarrowWindow(): boolean {
  return window.innerWidth < TWO_SIDE_COLUMNS_MIN_WIDTH;
}

/** 오른쪽 패널을 연다. 패널을 여는 곳은 모두 이것을 쓴다: 좁은 창에서는 dot 옆 칸을 닫아 자리를 만든다. */
export function openPanel(panel: PanelId): void {
  set((state) => ({ panel, paletteOpen: false, dotDock: isNarrowWindow() ? null : state.dotDock }));
}

export function togglePanel(panel: PanelId): void {
  if (useUiStore.getState().panel === panel) {
    set({ panel: null, paletteOpen: false });
  } else {
    openPanel(panel);
  }
}

export function toggleTerminal(): void {
  set((state) => ({ terminalOpen: !state.terminalOpen, paletteOpen: false }));
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

/**
 * dot 과의 대화를 옆 칸에 연다 (D-19). 보던 화면은 그대로 둔다. 좁은 창에서는 오른쪽 패널을 닫아 자리를 만든다.
 */
export function showDot(host: string, dotId: string): void {
  set((state) => ({ dotDock: { host, dotId }, paletteOpen: false, panel: isNarrowWindow() ? null : state.panel }));
}

/** dot 과의 대화 옆 칸을 닫는다. 쓰던 글과 보내기 허락은 남는다. */
export function closeDot(): void {
  set({ dotDock: null });
}

export const DOTS_CLOUD_ENABLE_WARNING =
  "Dots 를 켭니다. OpenAI 의 비공개 백엔드에 공식 앱 신원으로 붙습니다. 약관과 계정 위험을 확인하세요. 계속합니까?";

/**
 * dots 클라우드 활성화 플래그와 사이드바의 원격 구역을 함께 켜고 끈다 (plan 25 §3.2). 켜는 명령은 이것 하나이다.
 * 둘 가운데 하나라도 꺼져 있으면 둘 다 켠다. 켤 때만 경고를 보이고 사용자가 확인해야 켠다. 클라우드 호스트는
 * Gateway 를 다시 시작해야 생긴다. 그 전에는 구역을 켜도 보일 호스트가 없다.
 */
export async function toggleDotsCloud(confirm: (message: string) => boolean = (message) => window.confirm(message), notify: (message: string) => void = toast): Promise<void> {
  set({ paletteOpen: false });
  const enable = !(await window.akra.dots.getEnabled()) || !useUiStore.getState().remoteAgents;
  if (enable && !confirm(DOTS_CLOUD_ENABLE_WARNING)) return;
  const enabled = await window.akra.dots.setEnabled(enable);
  setRemoteAgents(enabled);
  notify(enabled ? "Dots 를 켰습니다. 사이드바에 안 보이면 Gateway 를 다시 시작하세요." : "Dots 를 껐습니다.");
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
  const theme: Theme = useUiStore.getState().theme === "dark" ? "light" : "dark";
  try {
    window.localStorage.setItem(THEME_KEY, theme);
  } catch {
    // 저장하지 못하면 이 창에서만 유지된다.
  }
  set({ theme });
}
