import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { stepNav } from "./navHistory";

let ui: typeof import("./uiStore");
let initial: import("./uiStore").UiState;

beforeAll(async () => {
  vi.stubGlobal("navigator", { userAgent: "desktop-navigation-test" });
  vi.stubGlobal("window", {
    localStorage: { getItem: () => null },
    matchMedia: () => ({ matches: false }),
  });
  ui = await import("./uiStore");
  initial = ui.useUiStore.getState();
});

afterEach(() => {
  ui.useUiStore.setState(initial, true);
});

afterAll(() => {
  vi.unstubAllGlobals();
});

describe("navigation out of a maximized issues panel", () => {
  it.each(["thread-a", "thread-b"])("reveals the selected conversation %s while keeping the issues panel open", (threadId) => {
    ui.visit({ view: "thread", threadId: "thread-a" });
    ui.useUiStore.setState({ panel: "issues", panelMaximized: true });

    ui.visit({ view: "thread", threadId });

    const state = ui.useUiStore.getState();
    expect(state.view).toBe("thread");
    expect(state.panel).toBe("issues");
    expect(state.panelMaximized).toBe(false);
    expect(state.nav.entries[state.nav.index]).toEqual({ view: "thread", threadId });
    expect(state.nav.entries).toHaveLength(threadId === "thread-a" ? 2 : 3);
  });

  it("restores the conversation on both backward and forward navigation without adding a visit", () => {
    ui.visit({ view: "thread", threadId: "thread-a" });
    ui.visit({ view: "thread", threadId: "thread-b" });
    const entries = ui.useUiStore.getState().nav.entries;

    for (const [delta, threadId] of [[-1, "thread-a"], [1, "thread-b"]] as const) {
      ui.useUiStore.setState({ panel: "issues", panelMaximized: true });
      const nav = stepNav(ui.useUiStore.getState().nav, delta, () => true);
      expect(nav).not.toBeNull();

      expect(ui.showNavEntry(nav!)).toEqual({ view: "thread", threadId });

      const state = ui.useUiStore.getState();
      expect(state.view).toBe("thread");
      expect(state.panel).toBe("issues");
      expect(state.panelMaximized).toBe(false);
      expect(state.nav.entries).toBe(entries);
      expect(state.nav.entries[state.nav.index]).toEqual({ view: "thread", threadId });
    }
  });
});

describe("dot visibility while the issues panel is maximized", () => {
  it("keeps the dock open without treating the hidden conversation as visible", () => {
    ui.useUiStore.setState({ dotDock: { host: "cloud", dotId: "dot-1" }, panel: "issues", panelMaximized: true });

    expect(ui.isDotDockVisible(ui.useUiStore.getState(), "cloud", "dot-1")).toBe(false);
    expect(ui.useUiStore.getState().dotDock).toEqual({ host: "cloud", dotId: "dot-1" });

    ui.useUiStore.setState({ panelMaximized: false });
    expect(ui.isDotDockVisible(ui.useUiStore.getState(), "cloud", "dot-1")).toBe(true);
    expect(ui.isDotDockVisible(ui.useUiStore.getState(), "other-host", "dot-1")).toBe(false);
    expect(ui.isDotDockVisible(ui.useUiStore.getState(), "cloud", "dot-2")).toBe(false);
  });

  it.each([800, 1600])("reveals an explicitly opened dot in a %s-pixel window", (width) => {
    Object.defineProperty(window, "innerWidth", { configurable: true, value: width });
    ui.useUiStore.setState({ dotDock: { host: "cloud", dotId: "dot-1" }, panel: "issues", panelMaximized: true });
    ui.setDraft(ui.dotDraftKey("dot-1"), "keep this draft");

    ui.showDot("cloud", "dot-1");

    const state = ui.useUiStore.getState();
    expect(state.panel).toBe(width < 1400 ? null : "issues");
    expect(state.panelMaximized).toBe(false);
    expect(ui.isDotDockVisible(state, "cloud", "dot-1")).toBe(true);
    expect(state.drafts[ui.dotDraftKey("dot-1")]).toBe("keep this draft");
  });
});

describe("navigation epochs for asynchronous storage evidence", () => {
  it("invalidates an earlier thread request across home and back to the same thread", () => {
    ui.visit({ view: "thread", threadId: "thread-a" });
    const issuedAt = ui.useUiStore.getState().navigationRevision;
    ui.visit({ view: "home" });
    ui.visit({ view: "thread", threadId: "thread-a" });
    expect(ui.useUiStore.getState().navigationRevision).toBeGreaterThan(issuedAt);
    const beforeBack = ui.useUiStore.getState().navigationRevision;
    const back = stepNav(ui.useUiStore.getState().nav, -1, () => true)!;
    ui.showNavEntry(back);
    const forward = stepNav(ui.useUiStore.getState().nav, 1, () => true)!;
    ui.showNavEntry(forward);
    expect(ui.useUiStore.getState().navigationRevision).toBeGreaterThan(beforeBack);
    expect(ui.useUiStore.getState().view).toBe("thread");
  });
});
