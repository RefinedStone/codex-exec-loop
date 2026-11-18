import { EventEmitter } from "node:events";
import { runInNewContext } from "node:vm";
import type { NativeImage, WebContents } from "electron";
import { afterEach, describe, expect, it, vi } from "vitest";
import { BrowserAutomation } from "../src/main/browserAgent/automation";
import { NODE_ACTION, PASSWORD_FOCUS, waitInstaller } from "../src/main/browserAgent/pageFunctions";
import type { BrowserAutomationTool, BrowserToolResult } from "../src/shared/browserAgent";

type Params = Record<string, unknown>;
class FakeDebugger extends EventEmitter {
  live = false;
  calls: { method: string; params: Params }[] = [];
  nodes = [{ nodeId: "1", backendDOMNodeId: 11, role: { value: "button" }, name: { value: "Save" } }];
  frames = { frame: { id: "main", url: "https://fixture.test/" },
    childFrames: [] as { frame: { id: string; url: string; parentId: string } }[] };
  handler?: (method: string, params: Params) => Promise<unknown> | unknown;
  isAttached(): boolean { return this.live; }
  attach(): void { this.live = true; }
  detach(): void { this.live = false; this.emit("detach", {}, "target closed"); }
  message(method: string, params: Params = {}): void { this.emit("message", {}, method, params); }
  async sendCommand(method: string, params: Params = {}): Promise<unknown> {
    this.calls.push({ method, params });
    if (this.handler) {
      const value = this.handler(method, params);
      if (value !== undefined) return value;
    }
    if (method === "Page.getFrameTree") return { frameTree: this.frames };
    if (method === "Accessibility.getFullAXTree") return { nodes: this.nodes };
    if (method === "DOM.resolveNode") return { object: { objectId: "node-11" } };
    if (method === "DOM.describeNode") return { node: { attributes: [] } };
    if (method === "Runtime.callFunctionOn") return { result: { value: { x: 20, y: 30 } } };
    if (method === "Page.getNavigationHistory") return { currentIndex: 1, entries: [{ id: 10 }, { id: 11 }, { id: 12 }] };
    if (["Page.navigate", "Page.navigateToHistoryEntry", "Page.reload"].includes(method)) {
      this.message("Page.loadEventFired");
      return { loaderId: "loaded" };
    }
    return {};
  }
}

function image(gray = 0, width = 1920, height = 1080): NativeImage {
  return {
    resize: vi.fn((size: { width: number; height: number }) => image(gray, size.width, size.height)),
    toBitmap: () => Buffer.alloc(4096 * 4, gray),
    getSize: () => ({ width, height }),
    isEmpty: () => false,
    toJPEG: vi.fn(() => Buffer.from("jpeg-fixture")),
  } as unknown as NativeImage;
}
class FakeContents extends EventEmitter {
  debugger = new FakeDebugger();
  capturePage = vi.fn(async () => image());
  getURL(): string { return "https://fixture.test/form?secret=hidden"; }
  isDestroyed(): boolean { return false; }
}
function payload(value: BrowserToolResult): Params {
  const content = value.content[0];
  if (content?.type !== "text") throw Error("Missing text result");
  return JSON.parse(content.text) as Params;
}
const engines: BrowserAutomation[] = [];
function fixture(existing = false): { contents: FakeContents; engine: BrowserAutomation; pause: ReturnType<typeof vi.fn> } {
  const contents = new FakeContents();
  contents.debugger.live = existing;
  const pause = vi.fn();
  const engine = new BrowserAutomation(contents as unknown as WebContents, pause);
  engines.push(engine);
  return { contents, engine, pause };
}
const execute = (engine: BrowserAutomation, tool: BrowserAutomationTool, args: Params = {}, signal = new AbortController().signal) =>
  engine.execute(tool, args, signal);
async function snapshotRef(engine: BrowserAutomation, index = 0): Promise<string> {
  const refs = String(payload(await execute(engine, "browser_snapshot", { diff: false })).text).matchAll(/\[(r\d+)\]/g);
  const ref = [...refs][index]?.[1];
  if (!ref) throw Error("Missing fixture ref");
  return ref;
}
afterEach(() => { for (const engine of engines.splice(0)) engine.dispose(); vi.useRealTimers(); });

describe("BrowserAutomation boundary and refs", () => {
  it("rejects keyboard focus hidden behind a closed shadow root", async () => {
    const { engine, contents } = fixture();
    contents.debugger.handler = (method) => {
      if (method === "Runtime.evaluate") return { result: { objectId: "closed-host" } };
      if (method === "DOM.describeNode") return { node: { shadowRoots: [{ shadowRootType: "closed" }] } };
    };
    expect((await execute(engine, "browser_press", { key: "X" })).isError).toBe(true);
    expect(contents.debugger.calls.some(call => call.method === "Input.dispatchKeyEvent")).toBe(false);
  });
  it("filters interactive snapshots and uses diffs by default", async () => {
    const { engine, contents } = fixture();
    contents.debugger.nodes.push({ nodeId: "2", backendDOMNodeId: 12, role: { value: "StaticText" }, name: { value: "Long description" } });
    const first = payload(await execute(engine, "browser_snapshot", { interactive: true }));
    expect(first).toMatchObject({ url: "https://fixture.test/form", contentKind: "untrusted_page_data" });
    expect(first.text).toContain("Save");
    expect(first.text).not.toContain("Long description");
    expect(payload(await execute(engine, "browser_snapshot", { interactive: true })).text).toBe("");
    expect(payload(await execute(engine, "browser_snapshot", { diff: false })).text).toContain("Long description");
  });
  it("attaches once and does not detach an incumbent debugger", async () => {
    const { engine, contents, pause } = fixture(true);
    expect(engine.attached).toBe(false);
    expect((await execute(engine, "browser_snapshot")).isError).toBe(true);
    contents.debugger.message("Page.javascriptDialogOpening", { type: "alert" });
    expect(contents.debugger.calls).toHaveLength(0);
    engine.dispose();
    expect(contents.debugger.live).toBe(true);
    expect(pause).not.toHaveBeenCalled();
  });
  it("rejects unknown fields, executable URLs, credentials and missing refs", async () => {
    const { engine, contents } = fixture();
    for (const args of [{ url: "javascript:alert(1)" }, { url: "file:///tmp/private" },
      { url: "https://user:password@fixture.test" }, { url: "https://fixture.test", owner: 1 }])
      expect((await execute(engine, "browser_navigate", args)).isError).toBe(true);
    expect((await execute(engine, "browser_click")).isError).toBe(true);
    expect(contents.debugger.calls.some(c => c.method === "Page.navigate")).toBe(false);
  });
  it("keeps refs stable, produces diffs, and never reuses refs after navigation", async () => {
    const { engine, contents } = fixture();
    const ref = await snapshotRef(engine);
    expect(payload(await execute(engine, "browser_snapshot", { diff: true }))).toMatchObject({ changed: false, text: "" });
    contents.debugger.nodes[0]!.name.value = "Updated";
    const changed = String(payload(await execute(engine, "browser_snapshot", { diff: true })).text);
    expect(changed).toContain(`- [${ref}] button "Save"`);
    expect(changed).toContain(`+ [${ref}] button "Updated"`);
    contents.emit("did-start-navigation", {}, "https://fixture.test/next", false, true);
    expect((await execute(engine, "browser_click", { ref })).isError).toBe(true);
    expect(await snapshotRef(engine)).not.toBe(ref);
  });
  it("never reuses refs when the parent replaces an engine on resume", async () => {
    const first = fixture(), oldRef = await snapshotRef(first.engine);
    first.engine.dispose();
    const second = fixture(), newRef = await snapshotRef(second.engine);
    expect(newRef).not.toBe(oldRef);
    expect((await execute(second.engine, "browser_click", { ref: oldRef })).isError).toBe(true);
  });
  it("bounds snapshot output and redacts password values", async () => {
    const { engine, contents } = fixture();
    contents.debugger.nodes = Array.from({ length: 50 }, (_, i) =>
      ({ nodeId: String(i), backendDOMNodeId: i + 1, role: { value: "textbox" }, name: { value: "Password" }, value: { value: "private-password" } }));
    contents.debugger.handler = method => method === "DOM.describeNode" ?
      { node: { attributes: ["type", "password"] } } : undefined;
    const value = payload(await execute(engine, "browser_snapshot", { maxChars: 200 }));
    expect(value.truncated).toBe(true);
    expect(String(value.text).length).toBeLessThanOrEqual(200);
    expect(value.text).not.toContain("private-password");
    expect(value.text).toContain("[redacted]");
  });
  it("includes same-origin frames and marks cross-origin frames without touching them", async () => {
    const { engine, contents } = fixture();
    contents.debugger.frames.childFrames.push(
      { frame: { id: "same", parentId: "main", url: "https://fixture.test/frame" } },
      { frame: { id: "other", parentId: "main", url: "https://other.test/" } });
    const value = payload(await execute(engine, "browser_snapshot"));
    expect(String(value.text)).toContain("cross-origin");
    expect(contents.debugger.calls.filter(c => c.method === "Accessibility.getFullAXTree").map(c => c.params.frameId))
      .toEqual(["main", "same"]);
  });
  it("rejects a ref whose frame changed origin", async () => {
    const { engine, contents } = fixture();
    contents.debugger.frames.childFrames.push({ frame: { id: "same", parentId: "main", url: "https://fixture.test/frame" } });
    const ref = await snapshotRef(engine, 1);
    contents.debugger.frames.childFrames[0]!.frame.url = "https://other.test/";
    expect((await execute(engine, "browser_click", { ref })).isError).toBe(true);
  });
});

describe("BrowserAutomation actions and cancellation", () => {
  it("waits for a complete quiet network interval after the last request", async () => {
    vi.useFakeTimers();
    const { engine, contents } = fixture();
    await snapshotRef(engine);
    const complete = vi.fn();
    const pending = execute(engine, "browser_wait", { event: "networkidle", timeoutMs: 2000 }).then(value => { complete(); return value; });
    await vi.advanceTimersByTimeAsync(250);
    contents.debugger.message("Network.requestWillBeSent", { requestId: "r", request: { url: "https://fixture.test/", method: "GET" } });
    await vi.advanceTimersByTimeAsync(500);
    expect(complete).not.toHaveBeenCalled();
    contents.debugger.message("Network.loadingFinished", { requestId: "r" });
    await vi.advanceTimersByTimeAsync(499);
    expect(complete).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect((await pending).isError).not.toBe(true);
  });
  it("submits typed input with an Enter press", async () => {
    const { engine, contents } = fixture();
    const ref = await snapshotRef(engine);
    expect((await execute(engine, "browser_type", { ref, text: "hello", submit: true })).isError).not.toBe(true);
    expect(contents.debugger.calls.filter(c => c.method === "Input.dispatchKeyEvent").map(c => [c.params.type, c.params.key, c.params.text]))
      .toEqual([["rawKeyDown", "Enter", undefined], ["char", "Enter", "\r"], ["keyUp", "Enter", undefined]]);
  });
  it("clicks with CDP coordinates and only suppresses the matching synthetic event", async () => {
    const { engine, contents, pause } = fixture();
    const ref = await snapshotRef(engine);
    contents.debugger.handler = (method, params) => {
      if (method === "Input.dispatchMouseEvent") contents.emit("before-mouse-event", {}, {
        type: params.type === "mouseMoved" ? "mouseMove" : params.type === "mousePressed" ? "mouseDown" : "mouseUp", x: params.x, y: params.y,
      });
    };
    expect((await execute(engine, "browser_click", { ref })).isError).not.toBe(true);
    expect(contents.debugger.calls.filter(c => c.method === "Input.dispatchMouseEvent").map(c => c.params.type))
      .toEqual(["mouseMoved", "mousePressed", "mouseReleased"]);
    expect(pause).not.toHaveBeenCalled();
    contents.emit("before-mouse-event", {}, { type: "mouseDown", x: 20, y: 30 });
    expect(pause).toHaveBeenCalledWith("Human mouse input");
  });
  it("guards key events narrowly without pausing its own press", async () => {
    const { engine, contents, pause } = fixture();
    contents.debugger.handler = (method, params) => {
      if (method === "Input.dispatchKeyEvent" && params.type !== "char") contents.emit("before-input-event", {}, {
        type: params.type === "rawKeyDown" ? "keyDown" : params.type, key: params.key,
      });
    };
    expect((await execute(engine, "browser_press", { key: "Enter" })).isError).not.toBe(true);
    expect(pause).not.toHaveBeenCalled();
    contents.emit("before-input-event", {}, { type: "keyDown", key: "x" });
    expect(pause).toHaveBeenCalledWith("Human keyboard input");
  });
  it("does not suppress unrelated human input during a synthetic command", async () => {
    const { engine, contents, pause } = fixture();
    const ref = await snapshotRef(engine);
    contents.debugger.handler = method => {
      if (method === "Input.dispatchMouseEvent") contents.emit("before-mouse-event", {}, { type: "mouseDown", x: 200, y: 300 });
    };
    expect((await execute(engine, "browser_click", { ref })).isError).toBe(true);
    expect(pause).toHaveBeenCalled();
    expect(contents.debugger.calls.filter(c => c.method === "Input.dispatchMouseEvent")).toHaveLength(1);
  });
  it("rejects canceled actions before side effects and concurrent actions", async () => {
    const { engine, contents } = fixture();
    const controller = new AbortController(); controller.abort();
    expect((await execute(engine, "browser_navigate", { url: "https://fixture.test" }, controller.signal)).isError).toBe(true);
    expect(contents.debugger.calls.some(c => c.method === "Page.navigate")).toBe(false);
    const waiting = new AbortController();
    const armed = new Promise<void>(resolve => {
      contents.debugger.handler = method => { if (method === "Runtime.evaluate") resolve(); };
    });
    const first = execute(engine, "browser_wait", { text: "Later" }, waiting.signal);
    await armed;
    expect(payload(await execute(engine, "browser_reload")).error).toContain("busy");
    waiting.abort();
    expect((await first).isError).toBe(true);
    expect(contents.debugger.calls.some(c => c.method === "Runtime.evaluate" && String(c.params.expression).includes("?.get"))).toBe(true);
  });
  it("does not insert text after a password or overlay rejection", async () => {
    const { engine, contents } = fixture();
    const ref = await snapshotRef(engine);
    contents.debugger.handler = method => method === "Runtime.callFunctionOn" ?
      { exceptionDetails: { exception: { description: "Password input is not allowed" } }, result: {} } : undefined;
    expect((await execute(engine, "browser_type", { ref, text: "secret" })).isError).toBe(true);
    expect(contents.debugger.calls.some(c => c.method === "Input.insertText")).toBe(false);
  });
  it("runs type, select, press and scroll through their closed operations", async () => {
    const { engine, contents } = fixture();
    const ref = await snapshotRef(engine);
    for (const [tool, args] of [
      ["browser_type", { ref, text: "hello" }],
      ["browser_select", { ref, values: ["one"] }],
      ["browser_press", { ref, key: "Enter" }],
      ["browser_scroll", { ref, y: 200 }],
    ] as const) expect((await execute(engine, tool, args)).isError).not.toBe(true);
    expect(contents.debugger.calls.some(c => c.method === "Input.insertText" && c.params.text === "hello")).toBe(true);
    expect(contents.debugger.calls.filter(c => c.method === "Input.dispatchKeyEvent")).toHaveLength(3);
  });
  it("subscribes before navigation and handles history in both directions", async () => {
    const { engine, contents } = fixture();
    for (const tool of ["browser_navigate", "browser_back", "browser_forward", "browser_reload"] as const)
      expect((await execute(engine, tool, tool === "browser_navigate" ? { url: "about:blank" } : {})).isError).not.toBe(true);
    expect(contents.debugger.calls.filter(c => c.method === "Page.navigateToHistoryEntry").map(c => c.params.entryId)).toEqual([10, 12]);
  });
  it("completes same-document history navigation without waiting for a load event", async () => {
    const { engine, contents } = fixture();
    contents.debugger.handler = method => {
      if (method === "Page.navigateToHistoryEntry") {
        contents.debugger.message("Page.navigatedWithinDocument", { frameId: "main", url: "https://fixture.test/#previous" });
        return {};
      }
    };
    expect((await execute(engine, "browser_back")).isError).not.toBe(true);
  });
  it("pauses on debugger detach but not on dispose", async () => {
    const a = fixture();
    a.contents.debugger.detach();
    expect(a.engine.attached).toBe(false);
    expect(a.pause).toHaveBeenCalledTimes(1);
    a.engine.dispose();
    expect(a.pause).toHaveBeenCalledTimes(1);
    const b = fixture(); b.engine.dispose();
    expect(b.pause).not.toHaveBeenCalled();
  });
});

describe("BrowserAutomation event waits and observers", () => {
  it("accepts only its binding result and releases listeners after completion", async () => {
    const { engine, contents } = fixture();
    const installed = new Promise<void>(resolve => {
      contents.debugger.handler = (method, params) => {
        if (method === "Runtime.evaluate" && String(params.expression).includes("MutationObserver")) {
          const id = /const id = "(w\d+)"/.exec(String(params.expression))?.[1];
          contents.debugger.message("Runtime.bindingCalled", { name: "other", payload: JSON.stringify({ id }) });
          contents.debugger.message("Runtime.bindingCalled", { name: "__akraBrowserWait", payload: JSON.stringify({ id }) });
          resolve();
        }
      };
    });
    const waiting = execute(engine, "browser_wait", { text: "Delayed text" });
    await installed;
    expect((await waiting).isError).not.toBe(true);
  });
  it("times out deterministically and cancels event waits immediately", async () => {
    vi.useFakeTimers();
    const { engine } = fixture();
    // Complete initialization before subscribing to an event.
    await execute(engine, "browser_console");
    const timed = execute(engine, "browser_wait", { event: "load", timeoutMs: 123 });
    await vi.advanceTimersByTimeAsync(123);
    expect(payload(await timed).error).toContain("timed out");
    const abort = new AbortController();
    const pending = execute(engine, "browser_wait", { event: "navigation" }, abort.signal);
    abort.abort();
    expect((await pending).isError).toBe(true);
    expect(vi.getTimerCount()).toBe(0);
  });
  it("bounds console/failure buffers and drops request headers and URL secrets", async () => {
    const { engine, contents } = fixture();
    for (let i = 0; i < 250; i++) {
      contents.debugger.message("Runtime.consoleAPICalled", { type: "log",
        args: [{ type: "string", value: "x".repeat(4000) }, { type: "object", description: "secret-cookie" }] });
      contents.debugger.message("Network.requestWillBeSent", { requestId: String(i),
        request: { url: "https://user:pass@fixture.test/fail?token=secret", method: "GET", headers: { Cookie: "secret" } } });
      contents.debugger.message("Network.loadingFailed", { requestId: String(i), errorText: "Failed" });
    }
    const logs = payload(await execute(engine, "browser_console"));
    expect(logs.total).toBe(250);
    expect((logs.entries as Params[])).toHaveLength(200);
    expect(JSON.stringify(logs)).not.toContain("secret-cookie");
    const failures = payload(await execute(engine, "browser_network_failures", { limit: 2, clear: true }));
    expect((failures.entries as Params[])).toHaveLength(2);
    expect(JSON.stringify(failures)).not.toMatch(/token|Cookie|pass@/);
    expect(payload(await execute(engine, "browser_network_failures")).entries).toEqual([]);
  });
  it("dismisses dialogs without retaining prompt text or accepting the dialog", async () => {
    const { engine, contents } = fixture();
    contents.debugger.message("Page.javascriptDialogOpening", { type: "prompt", message: "secret", defaultPrompt: "password" });
    expect(contents.debugger.calls.find(c => c.method === "Page.handleJavaScriptDialog")?.params).toEqual({ accept: false });
    const value = await execute(engine, "browser_console");
    expect(JSON.stringify(value)).not.toContain("secret");
    expect(JSON.stringify(value)).toContain("dismissed");
  });
});

describe("BrowserAutomation screenshots", () => {
  it("resizes the longest edge, emits JPEG, suppresses duplicates and resets on navigation", async () => {
    const { engine, contents } = fixture();
    const first = await execute(engine, "browser_screenshot");
    expect(first.content[1]).toEqual({ type: "image", data: Buffer.from("jpeg-fixture").toString("base64"), mimeType: "image/jpeg" });
    expect(payload(first)).toMatchObject({ width: 1280, height: 720, changed: true });
    expect(payload(await execute(engine, "browser_screenshot")).changed).toBe(false);
    contents.emit("did-start-navigation", {}, "https://fixture.test/next", false, true);
    expect((await execute(engine, "browser_screenshot")).content[1]?.type).toBe("image");
  });
  it("uses controlled 200ms samples for stability and cancellation", async () => {
    vi.useFakeTimers();
    const { engine, contents } = fixture();
    await execute(engine, "browser_console");
    const stable = execute(engine, "browser_screenshot", { stable: true });
    await vi.advanceTimersByTimeAsync(200);
    expect(payload(await stable).stable).toBe(true);
    expect(contents.capturePage).toHaveBeenCalledTimes(2);
    const controller = new AbortController();
    const pending = execute(engine, "browser_screenshot", { stable: true }, controller.signal);
    await vi.advanceTimersByTimeAsync(0);
    controller.abort();
    expect((await pending).isError).toBe(true);
    expect(vi.getTimerCount()).toBe(0);
  });
  it("bounds unstable screenshot sampling at three seconds", async () => {
    vi.useFakeTimers();
    const { engine, contents } = fixture();
    let frame = 0;
    contents.capturePage.mockImplementation(async () => image(frame++ % 2 ? 255 : 0));
    await execute(engine, "browser_console");
    const unstable = execute(engine, "browser_screenshot", { stable: true });
    await vi.advanceTimersByTimeAsync(3000);
    expect(payload(await unstable).stable).toBe(false);
    expect(contents.capturePage).toHaveBeenCalledTimes(16);
    expect(vi.getTimerCount()).toBe(0);
  });
});

describe("fixed page action implementation", () => {
  it("rejects the focused password inside a shadow root", () => {
    const input = { type: "password" };
    const context = { document: { activeElement: { shadowRoot: { activeElement: input } } } };
    expect(() => runInNewContext(PASSWORD_FOCUS, context)).toThrow("Password");
    input.type = "text";
    expect(runInNewContext(PASSWORD_FOCUS, context)).toBe(input);
  });
  function nodeFixture(overrides: Params = {}): { node: Params; action: (op: string, args?: Params) => unknown } {
    const win: Params = { innerWidth: 800, innerHeight: 600, getComputedStyle: () => ({ visibility: "visible", display: "block" }) };
    win.top = win;
    const doc: Params = { defaultView: win };
    const node: Params = {
      isConnected: true, ownerDocument: doc, getBoundingClientRect: () => ({ left: 10, top: 20, right: 110, bottom: 60, width: 100, height: 40 }),
      scrollIntoView: vi.fn(), getAttribute: () => null, contains: () => false,
      focus: vi.fn(), select: vi.fn(), closest: () => null, matches: () => overrides.disabled === true, tagName: "INPUT", ...overrides,
    };
    doc.elementFromPoint = () => node;
    // Execute the exact fixed source sent to CDP, not a mock of its safety checks.
    const fn = new Function(`return (${NODE_ACTION})`)() as (this: Params, op: string, args: Params) => unknown;
    return { node, action: (op, args = {}) => fn.call(node, op, args) };
  }
  it("rejects detached, disabled, hidden and overlay-covered nodes", () => {
    expect(() => nodeFixture({ isConnected: false }).action("click")).toThrow("detached");
    expect(() => nodeFixture({ disabled: true }).action("click")).toThrow("disabled");
    expect(() => nodeFixture({ getBoundingClientRect: () => ({ width: 0, height: 0 }) }).action("click")).toThrow("hidden");
    const fixture = nodeFixture();
    (fixture.node.ownerDocument as Params).elementFromPoint = () => ({});
    expect(() => fixture.action("click")).toThrow("overlay");
  });
  it("rejects passwords before focusing, and selects editable text for replacement", () => {
    const password = nodeFixture({ type: "password" });
    expect(() => password.action("type", { clear: true })).toThrow("Password");
    expect(password.node.focus).not.toHaveBeenCalled();
    const editable = nodeFixture();
    editable.action("type", { clear: true });
    expect(editable.node.focus).toHaveBeenCalledTimes(1);
    expect(editable.node.select).toHaveBeenCalledTimes(1);
  });
  it("rejects cross-origin frame access and accounts for same-origin frame offsets", () => {
    const nested = nodeFixture();
    const doc = nested.node.ownerDocument as Params;
    const win = doc.defaultView as Params;
    const top: Params = { innerWidth: 1000, innerHeight: 800, document: {} };
    top.top = top;
    const frameDoc: Params = { elementFromPoint: () => frame, defaultView: top };
    const frame: Params = { ownerDocument: frameDoc, clientLeft: 2, clientTop: 3, clientWidth: 200, clientHeight: 100,
      getBoundingClientRect: () => ({ left: 100, top: 200 }), contains: () => false, scrollIntoView: vi.fn() };
    win.top = top; win.parent = top; win.frameElement = frame;
    expect(nested.action("click")).toEqual({ x: 162, y: 243 });
    nested.node.getBoundingClientRect = () => ({ left: 10, top: 20, right: 610, bottom: 60, width: 600, height: 40 });
    expect(nested.action("crop")).toEqual({ x: 112, y: 223, width: 190, height: 40 });
    win.frameElement = null;
    expect(() => nested.action("click")).toThrow("Cross-origin");
  });
  it("selects only available values and dispatches both native form events", () => {
    class FormEvent { constructor(readonly type: string) {} }
    const select = nodeFixture({ tagName: "SELECT", multiple: true,
      options: [{ value: "one", disabled: false, selected: false }, { value: "two", disabled: true, selected: false }],
      dispatchEvent: vi.fn() });
    ((select.node.ownerDocument as Params).defaultView as Params).Event = FormEvent;
    select.action("select", { values: ["one"] });
    expect((select.node.options as Params[])[0]!.selected).toBe(true);
    expect(select.node.dispatchEvent).toHaveBeenNthCalledWith(1, expect.objectContaining({ type: "input" }));
    expect(select.node.dispatchEvent).toHaveBeenNthCalledWith(2, expect.objectContaining({ type: "change" }));
    expect(() => select.action("select", { values: ["two"] })).toThrow("unavailable");
  });
});

describe("fixed page observer implementation", () => {
  it("subscribes before checking and disconnects exactly when delayed text appears", () => {
    vi.useFakeTimers();
    let available = false;
    const callbacks: (() => void)[] = [];
    const disconnect = vi.fn(), binding = vi.fn();
    class Observer {
      constructor(callback: () => void) { callbacks.push(callback); }
      observe(): void {}
      disconnect = disconnect;
    }
    const win = { getComputedStyle: () => ({ visibility: "visible", display: "block" }),
      addEventListener: vi.fn(), removeEventListener: vi.fn() };
    const parent = { closest: () => null, getBoundingClientRect: () => ({ width: 10, height: 10 }) };
    const doc = { defaultView: win, body: {}, documentElement: {}, addEventListener: vi.fn(), removeEventListener: vi.fn(),
      createTreeWalker: () => {
        let visited = false;
        return { nextNode: () => {
          if (visited || !available) return null;
          visited = true;
          return { textContent: "Delayed content", parentElement: parent };
        } };
      } };
    const context = { document: doc, MutationObserver: Observer, ResizeObserver: Observer, setTimeout, clearTimeout,
      __akraBrowserWait: binding };
    runInNewContext(`(${waitInstaller("w1", "Delayed", false, "visible", 1000)})()`, context);
    expect(binding).not.toHaveBeenCalled();
    expect(callbacks).toHaveLength(2);
    available = true;
    callbacks[0]!();
    expect(binding).toHaveBeenCalledWith('{"id":"w1"}');
    expect(disconnect).toHaveBeenCalledTimes(2);
    expect(vi.getTimerCount()).toBe(0);
  });
});
