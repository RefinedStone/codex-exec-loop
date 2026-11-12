// 화면 틀의 상태이다: 어떤 보기와 패널이 열려 있는지. 대화 상태는 `desktopStore` 에 있다.
import { create } from "zustand";

export type View = "home" | "thread" | "fleet";
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

export function toggleTheme(): void {
  set((state) => ({ theme: state.theme === "dark" ? "light" : "dark" }));
}
