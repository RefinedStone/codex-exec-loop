// 기본 데스크톱 동작의 계산이다 (plan 26 U1): 확대 단계, 창 자리 복원, 오른쪽 클릭 메뉴, 이동 기록, 찾기,
// 입력창 글쇠, 알림 판정.
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

import { contextMenuItems, type ContextTarget } from "../src/main/contextMenuItems";
import { loadWindowState, parseWindowState, saveWindowState, visibleBounds } from "../src/main/windowState";
import type { PendingApproval } from "../src/renderer/conversation/approvals";
import { addPrompt, isSendKey, recallCursor } from "../src/renderer/conversation/composerKeys";
import { findTurns, matchOffsets, matchRanges } from "../src/renderer/conversation/find";
import type { TurnView } from "../src/renderer/conversation/transcript";
import { attentionEvents } from "../src/renderer/session/attention";
import type { DesktopState } from "../src/renderer/session/desktopStore";
import { INITIAL_NAV, type NavEntry, pushNav, stepNav } from "../src/renderer/session/navHistory";
import { isZoomScope, nextZoom, parseZoomFactors, stepZoomFactors, validZoom } from "../src/shared/zoom";

describe("zoom", () => {
  it("steps through the fixed factors and stops at both ends", () => {
    expect(nextZoom(1, "in")).toBe(1.1);
    expect(nextZoom(1.1, "out")).toBe(1);
    expect(nextZoom(2, "in")).toBe(2);
    expect(nextZoom(0.5, "out")).toBe(0.5);
    expect(nextZoom(1.75, "reset")).toBe(1);
    // 단계 사이의 값은 가까운 다음 단계로 간다.
    expect(nextZoom(1.2, "in")).toBe(1.25);
    expect(nextZoom(1.2, "out")).toBe(1.1);
  });

  it("replaces a stored value that is not a usable factor", () => {
    expect(validZoom("big")).toBe(1);
    expect(validZoom(Number.NaN)).toBe(1);
    expect(validZoom(9)).toBe(2);
    expect(validZoom(0.1)).toBe(0.5);
  });

  it("changes one scope and leaves every other scope as it is", () => {
    let factors = stepZoomFactors({ sidebar: 1.25 }, "thread", "in");
    expect(factors).toEqual({ sidebar: 1.25, thread: 1.1 });
    factors = stepZoomFactors(factors, "panel:review", "in");
    expect(factors).toEqual({ sidebar: 1.25, thread: 1.1, "panel:review": 1.1 });
    // 100% 로 돌아온 영역은 지운다.
    expect(stepZoomFactors(factors, "thread", "reset")).toEqual({ sidebar: 1.25, "panel:review": 1.1 });
  });

  it("accepts only known scopes and panel ids", () => {
    expect(isZoomScope("thread")).toBe(true);
    expect(isZoomScope("panel:akra.tech-blog")).toBe(true);
    expect(isZoomScope("panel:")).toBe(false);
    expect(isZoomScope("panel:../x")).toBe(false);
    expect(isZoomScope("body")).toBe(false);
    expect(isZoomScope(1)).toBe(false);
  });

  it("moves an old single factor to the body views once and keeps the shell at 100%", () => {
    expect(parseZoomFactors(undefined, 1.75)).toEqual({ home: 1.75, fleet: 1.75, thread: 1.75, remote: 1.75 });
    expect(parseZoomFactors(undefined, 1)).toEqual({});
    // 새 형식이 있으면 옛 값은 보지 않는다: 다시 옮기지 않는다.
    expect(parseZoomFactors({ thread: 1.25 }, 1.75)).toEqual({ thread: 1.25 });
    // 모르는 영역, 숫자가 아닌 값, 100% 는 버린다. 범위 밖은 끝 단계로 줄인다.
    expect(parseZoomFactors({ thread: "x", nope: 1.5, sidebar: 1, terminal: 9 }, undefined)).toEqual({ terminal: 2 });
    expect(parseZoomFactors([1.5], 1.5)).toEqual({ home: 1.5, fleet: 1.5, thread: 1.5, remote: 1.5 });
  });
});

describe("window state", () => {
  const screenArea = { x: 0, y: 0, width: 1440, height: 900 };

  it("keeps bounds that a display shows and drops bounds that are off screen", () => {
    const bounds = { x: 100, y: 80, width: 1200, height: 700 };
    expect(visibleBounds(bounds, [screenArea])).toEqual(bounds);
    // 뺀 두 번째 화면에 있던 창이다.
    expect(visibleBounds({ ...bounds, x: 2000 }, [screenArea])).toBeNull();
    // 제목 막대가 화면 위로 나갔다.
    expect(visibleBounds({ ...bounds, y: -300 }, [screenArea])).toBeNull();
    expect(visibleBounds({ ...bounds, x: 2000 }, [screenArea, { x: 1440, y: 0, width: 1920, height: 1080 }])).not.toBeNull();
    expect(visibleBounds(null, [screenArea])).toBeNull();
  });

  it("validates the file and survives a missing or broken file", () => {
    expect(parseWindowState({ zooms: { thread: 1.25 }, bounds: { x: 1, y: 2, width: 900.4, height: 600 }, maximized: true })).toEqual({
      zooms: { thread: 1.25 },
      bounds: { x: 1, y: 2, width: 900, height: 600 },
      maximized: true,
    });
    expect(parseWindowState({ zoom: "x", bounds: { x: 1, y: 2, width: 10, height: 10 } })).toEqual({ zooms: {}, bounds: null, maximized: false });
    const directory = mkdtempSync(join(tmpdir(), "akra-window-state-"));
    try {
      const path = join(directory, "window-state.json");
      expect(loadWindowState(path).zooms).toEqual({});
      writeFileSync(path, "{broken");
      expect(loadWindowState(path).bounds).toBeNull();
      saveWindowState(path, { zooms: { thread: 1.5, sidebar: 0.9 }, bounds: { x: 0, y: 0, width: 1000, height: 700 }, maximized: false });
      expect(loadWindowState(path)).toEqual({ zooms: { thread: 1.5, sidebar: 0.9 }, bounds: { x: 0, y: 0, width: 1000, height: 700 }, maximized: false });
      // 옛 판의 파일이다. 본문만 175% 로 옮기고, 저장하면 새 형식만 남는다.
      writeFileSync(path, JSON.stringify({ zoom: 1.75, bounds: null, maximized: false }));
      const migrated = loadWindowState(path);
      expect(migrated.zooms).toEqual({ home: 1.75, fleet: 1.75, thread: 1.75, remote: 1.75 });
      saveWindowState(path, migrated);
      expect(JSON.parse(readFileSync(path, "utf8"))).toEqual({ zooms: migrated.zooms, bounds: null, maximized: false });
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  });
});

describe("context menu", () => {
  const target = (fields: Partial<ContextTarget>): ContextTarget => ({
    isEditable: false,
    selectionText: "",
    linkURL: "",
    misspelledWord: "",
    dictionarySuggestions: [],
    editFlags: { canCut: false, canCopy: false, canPaste: true, canSelectAll: true },
    ...fields,
  });
  const names = (fields: Partial<ContextTarget>) => contextMenuItems(target(fields)).map((item) => (item.kind === "edit" ? item.role : item.kind));

  it("shows nothing on plain content and copy on a selection", () => {
    expect(names({})).toEqual([]);
    expect(names({ selectionText: "  " })).toEqual([]);
    expect(names({ selectionText: "글" })).toEqual(["copy"]);
  });

  it("shows the edit commands, spelling suggestions and link copy in an input", () => {
    expect(names({ isEditable: true })).toEqual(["cut", "copy", "paste", "pasteAndMatchStyle", "separator", "selectAll"]);
    expect(names({ isEditable: true, misspelledWord: "teh", dictionarySuggestions: ["the", "ten"] }).slice(0, 4)).toEqual(["suggestion", "suggestion", "learn", "separator"]);
    expect(names({ linkURL: "https://example.com" })).toEqual(["copyLink"]);
    expect(names({ linkURL: "https://example.com", selectionText: "글" })).toEqual(["copyLink", "separator", "copy"]);
    const cut = contextMenuItems(target({ isEditable: true })).find((item) => item.kind === "edit" && item.role === "cut");
    expect(cut).toMatchObject({ enabled: false });
  });
});

describe("navigation history", () => {
  const thread = (threadId: string): NavEntry => ({ view: "thread", threadId });
  const all = () => true;

  it("records visits, drops the forward entries after a new visit, and ignores a repeat", () => {
    let nav = pushNav(pushNav(INITIAL_NAV, thread("a")), thread("b"));
    expect(pushNav(nav, thread("b"))).toBe(nav);
    nav = stepNav(nav, -1, all) ?? nav;
    expect(nav.entries[nav.index]).toEqual(thread("a"));
    expect(stepNav(nav, 1, all)?.index).toBe(2);
    nav = pushNav(nav, { view: "fleet" });
    expect(nav.entries).toEqual([{ view: "home" }, thread("a"), { view: "fleet" }]);
    expect(stepNav(nav, 1, all)).toBeNull();
  });

  it("skips a deleted thread and an entry that equals the current one", () => {
    const nav = [thread("a"), thread("gone"), thread("a"), { view: "fleet" } as NavEntry].reduce(pushNav, INITIAL_NAV);
    const exists = (entry: NavEntry) => !(entry.view === "thread" && entry.threadId === "gone");
    const back = stepNav(nav, -1, exists);
    expect(back?.index).toBe(3);
    // `a` 에서 뒤로 가면 지워진 스레드와 같은 `a` 를 건너뛰어 home 으로 간다.
    expect(back && stepNav(back, -1, exists)?.index).toBe(0);
    expect(stepNav(INITIAL_NAV, -1, all)).toBeNull();
  });
});

describe("find in conversation", () => {
  const turn = (items: unknown[]) => ({ items }) as unknown as TurnView;
  const turns = [
    turn([{ type: "userMessage", id: "u", content: [{ type: "text", text: "Zoom 을 고쳐줘" }] }]),
    turn([{ type: "commandExecution", id: "c", command: "zoom", aggregatedOutput: "zoom" }]),
    turn([{ type: "agentMessage", id: "a", text: "확대(zoom) 값을 저장했습니다" }]),
  ];

  it("finds the turns whose user or agent text has the query, ignoring case", () => {
    expect(findTurns(turns, "ZOOM")).toEqual([0, 2]);
    expect(findTurns(turns, "  ")).toEqual([]);
    expect(findTurns(turns, "없는 글")).toEqual([]);
  });

  it("lists every start offset without overlap", () => {
    expect(matchOffsets("aaaa", "aa")).toEqual([0, 2]);
    expect(matchOffsets("abc", "")).toEqual([]);
  });
});

describe("composer keys", () => {
  const key = (fields: Record<string, unknown> = {}) => ({ key: "Enter", shiftKey: false, ctrlKey: false, metaKey: false, keyCode: 13, nativeEvent: { isComposing: false }, ...fields });

  it("does not send on the Enter that ends an IME composition", () => {
    expect(isSendKey(key())).toBe(true);
    expect(isSendKey(key({ shiftKey: true }))).toBe(false);
    expect(isSendKey(key({ nativeEvent: { isComposing: true } }))).toBe(false);
    expect(isSendKey(key({ keyCode: 229 }))).toBe(false);
  });

  it("walks the sent prompts from the newest to the oldest and back to an empty input", () => {
    expect(recallCursor(0, null, "older")).toBeNull();
    expect(recallCursor(3, null, "older")).toBe(2);
    expect(recallCursor(3, 2, "older")).toBe(1);
    expect(recallCursor(3, 0, "older")).toBe(0);
    expect(recallCursor(3, 1, "newer")).toBe(2);
    expect(recallCursor(3, 2, "newer")).toBeNull();
    expect(addPrompt(["a"], "a")).toEqual(["a"]);
    expect(addPrompt(["a"], " ")).toEqual(["a"]);
    expect(addPrompt(Array.from({ length: 50 }, (_, index) => `p${index}`), "new")).toHaveLength(50);
  });
});

describe("attention events", () => {
  type Source = Parameters<typeof attentionEvents>[0];
  const connected = { phase: "connected", endpoint: "" } as DesktopState["connection"];
  const state = (fields: Partial<Source>): Source => ({ connection: connected, activeThreadIds: [], activeRefills: 0, pendingApprovals: [], folders: [], ...fields });
  const approval = (requestId: number, threadId: string) => ({ requestId, threadId }) as unknown as PendingApproval;

  it("reports a finished turn, a failed turn and a new approval request", () => {
    const failedFolder = [{ cwd: "/w", threads: [{ id: "b", title: "B", updatedAt: 0, branch: null, failed: true }] }];
    expect(attentionEvents(state({ activeThreadIds: ["a", "b", "c"] }), state({ activeThreadIds: ["c"], folders: failedFolder }))).toEqual([
      { kind: "finished", threadId: "a" },
      { kind: "failed", threadId: "b" },
    ]);
    expect(attentionEvents(state({ pendingApprovals: [approval(1, "a")] }), state({ pendingApprovals: [approval(1, "a"), approval(2, "b")] }))).toEqual([{ kind: "approval", threadId: "b" }]);
  });

  it("stays silent while the connection is not up on both sides", () => {
    const disconnected = { phase: "disconnected", endpoint: "" } as DesktopState["connection"];
    expect(attentionEvents(state({ activeThreadIds: ["a"] }), state({ connection: disconnected }))).toEqual([]);
    expect(attentionEvents(state({ connection: disconnected, activeThreadIds: ["a"] }), state({}))).toEqual([]);
    // 다시 붙은 뒤 목록을 통째로 다시 채웠다.
    expect(attentionEvents(state({ activeThreadIds: ["a"] }), state({ activeRefills: 1 }))).toEqual([]);
  });
});

describe("original-text search ranges", () => {
  it("maps case expansions before and inside the match to original UTF-16 offsets", () => {
    expect(matchRanges("İstanbul", "bul")).toEqual([{ start: 5, end: 8 }]);
    expect(matchRanges("İstanbul", "i\u0307s")).toEqual([{ start: 0, end: 2 }]);
    expect(matchRanges("İİ", "i\u0307")).toEqual([{ start: 0, end: 1 }, { start: 1, end: 2 }]);
  });

  it("preserves surrogate pairs and context-sensitive lowercasing", () => {
    expect(matchRanges("😀İstanbul 한글", "bul")).toEqual([{ start: 7, end: 10 }]);
    expect(matchRanges("ΟΣ", "ος")).toEqual([{ start: 0, end: 2 }]);
    expect(matchRanges("İstanbul", "")).toEqual([]);
  });
});
