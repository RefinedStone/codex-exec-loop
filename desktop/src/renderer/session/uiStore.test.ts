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

describe("drafts that survive a restart (#2839)", () => {
  const memory = () => {
    const values = new Map<string, string>();
    return { values, getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => void values.set(key, value),
      removeItem: (key: string) => void values.delete(key), key: (index: number) => [...values.keys()][index] ?? null,
      get length() { return values.size; } };
  };

  it("saves only the changed draft, keeps another window's drafts and drops sent ones", () => {
    const storage = memory();
    const original = window.localStorage;
    Object.assign(window, { localStorage: storage });
    try {
      // A dot window saved its own draft.
      storage.setItem("akra.drafts", JSON.stringify({ "dot:d1": "dot text", broken: 3 }));
      ui.setDraft("thread-1", "half written");
      expect(ui.storedDrafts()).toEqual({ "dot:d1": "dot text", "thread-1": "half written" });
      expect(storage.getItem("akra.drafts")).toBeNull();
      ui.setDraft("thread-1", "");
      expect(ui.storedDrafts()).toEqual({ "dot:d1": "dot text" });
      ui.handOffToNewThread("moved text");
      expect(ui.storedDrafts()[ui.HOME_DRAFT]).toBe("moved text");
    } finally {
      Object.assign(window, { localStorage: original });
    }
  });

  it("keeps the newest 50 drafts and skips a huge one", () => {
    const storage = memory();
    const original = window.localStorage;
    Object.assign(window, { localStorage: storage });
    try {
      for (let index = 0; index < 55; index += 1) ui.setDraft(`thread-${index}`, `draft ${index}`);
      const kept = Object.keys(ui.storedDrafts());
      expect(kept).toHaveLength(50);
      expect(kept).not.toContain("thread-0");
      expect(kept).toContain("thread-54");
      ui.setDraft("thread-big", "x".repeat(20_001));
      expect(ui.storedDrafts()["thread-big"]).toBeUndefined();
      expect(ui.useUiStore.getState().drafts["thread-big"]).toHaveLength(20_001);
    } finally {
      Object.assign(window, { localStorage: original });
    }
  });

  it("does not revive a sent dot draft when its other window clears it during a thread save", () => {
    const storage = memory();
    storage.setItem("akra.drafts", JSON.stringify({ "dot:d1": "already sent", "thread-1": "old text" }));
    const original = window.localStorage;
    const read = storage.getItem;
    let interleaved = false;
    storage.getItem = (key) => {
      const snapshot = read(key);
      if (!interleaved && key === "akra.drafts") {
        interleaved = true;
        ui.setDraft("dot:d1", "");
      }
      return snapshot;
    };
    Object.assign(window, { localStorage: storage });
    try {
      ui.setDraft("thread-1", "latest thread text");
      expect(interleaved).toBe(true);
      expect(ui.storedDrafts()).toEqual({ "thread-1": "latest thread text" });
    } finally { Object.assign(window, { localStorage: original }); }
  });

  it("keeps another window's new draft when it writes just before this window commits", () => {
    const storage = memory();
    const original = window.localStorage;
    const write = storage.setItem;
    let interleaved = false;
    storage.setItem = (key, value) => {
      if (!interleaved) {
        interleaved = true;
        ui.setDraft("dot:d2", "other window text");
      }
      write(key, value);
    };
    Object.assign(window, { localStorage: storage });
    try {
      ui.setDraft("thread-2", "this window text");
      expect(interleaved).toBe(true);
      expect(ui.storedDrafts()).toEqual({ "dot:d2": "other window text", "thread-2": "this window text" });
    } finally { Object.assign(window, { localStorage: original }); }
  });

  it("commits immediately and lets the latest sequential edit or deletion of a thread win", () => {
    const storage = memory();
    const original = window.localStorage;
    Object.assign(window, { localStorage: storage });
    const now = vi.spyOn(Date, "now").mockReturnValue(123456789);
    try {
      ui.setDraft("same-thread", "first");
      ui.setDraft("same-thread", "second");
      expect(ui.storedDrafts()).toEqual({ "same-thread": "second" });
      ui.setDraft("same-thread", "");
      expect(ui.storedDrafts()).toEqual({});
    } finally { now.mockRestore(); Object.assign(window, { localStorage: original }); }
  });

  it("keeps typing in memory when persistent storage rejects writes", () => {
    const storage = memory();
    storage.setItem = () => { throw new Error("QuotaExceededError"); };
    const original = window.localStorage;
    Object.assign(window, { localStorage: storage });
    try {
      expect(() => ui.setDraft("quota-thread", "keep typing")).not.toThrow();
      expect(ui.useUiStore.getState().drafts["quota-thread"]).toBe("keep typing");
    } finally { Object.assign(window, { localStorage: original }); }
  });


  it("does not delete another window's refreshed draft while trimming an older immutable record", () => {
    const storage = memory();
    const original = window.localStorage;
    Object.assign(window, { localStorage: storage });
    try {
      for (let index = 0; index < 50; index += 1) ui.setDraft(`thread-${index}`, `draft ${index}`);
      const oldest = [...storage.values.entries()].find(([, value]) => JSON.parse(value).key === "thread-0")!;
      expect(oldest).toBeDefined();
      const remove = storage.removeItem;
      let interleaved = false;
      storage.removeItem = (key) => {
        if (!interleaved && key === oldest[0]) {
          interleaved = true;
          ui.setDraft("thread-0", "refreshed by another window");
        }
        remove(key);
      };
      ui.setDraft("thread-50", "new draft");
      const saved = ui.storedDrafts();
      expect(interleaved).toBe(true);
      expect(Object.keys(saved)).toHaveLength(50);
      expect(saved["thread-0"]).toBe("refreshed by another window");
      expect(saved["thread-50"]).toBe("new draft");
      expect(saved["thread-1"]).toBeUndefined();
    } finally { Object.assign(window, { localStorage: original }); }
  });

});
