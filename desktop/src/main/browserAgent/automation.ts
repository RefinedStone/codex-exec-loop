import type { NativeImage, WebContents } from "electron";
import { EventEmitter } from "node:events";
import type { BrowserAutomationTool, BrowserToolResult } from "../../shared/browserAgent";
import { NODE_ACTION, PASSWORD_FOCUS, waitInstaller } from "./pageFunctions";

type Args = Record<string, unknown>;
interface AXNode {
  nodeId: string;
  parentId?: string;
  backendDOMNodeId?: number;
  ignored?: boolean;
  role?: { value?: unknown };
  name?: { value?: unknown };
  value?: { value?: unknown };
  properties?: { name: string; value: { value?: unknown } }[];
}
interface Frame { id: string; url: string; parentId?: string; securityOrigin?: string }
interface FrameTree { frame: Frame; childFrames?: FrameTree[] }
interface Ref { backend: number; frame: string; generation: number }
interface Snapshot { url: string; contentKind: "untrusted_page_data"; text: string; changed: boolean; truncated: boolean; omittedListItems: number }
const LIMIT = 200;
const NETWORK_IDLE_MS = 500;
const LIST_PREVIEW_ITEMS = 10;
const INTERACTIVE_ROLES = new Set(["button", "link", "textbox", "searchbox", "checkbox", "radio", "combobox", "listbox", "option", "menuitem", "menuitemcheckbox", "menuitemradio", "tab", "slider", "spinbutton", "switch", "treeitem"]);
// Engines are replaced on handoff/resume. Old model refs must stay stale then too.
let nextRef = 1;
let nextWait = 1;
const text = (value: unknown, max = 1000): string => typeof value === "string" ? value.slice(0, max) : "";
const errorText = (error: unknown): string => error instanceof Error ? error.message.slice(0, 500) : "Browser operation failed";
const result = (value: unknown, isError = false): BrowserToolResult => ({
  content: [{ type: "text", text: JSON.stringify(value) }], ...(isError ? { isError: true } : {}),
});

/** Closed browser tool boundary. No caller-provided JavaScript or CDP commands. */
export class BrowserAutomation {
  private disposed = false;
  private ownsDebugger = false;
  private busy = false;
  private generation = 0;
  private refs = new Map<string, Ref>();
  private refKeys = new Map<string, string>();
  private frames = new Map<string, Frame>();
  private previousSnapshot = "";
  private previousImage?: Uint8Array;
  private previousImageKey = "";
  private consoleEntries: Args[] = [];
  private failures: Args[] = [];
  private consoleCount = 0;
  private failureCount = 0;
  private requests = new Map<string, { url: string; method: string }>();
  private events = new EventEmitter();
  private ready: Promise<void>;
  private synthetic?: { kind: "mouse" | "key"; type: string; x?: number; y?: number; key?: string };
  private readonly onDetach = (_event: unknown, reason: string): void => {
    if (this.disposed || !this.ownsDebugger) return;
    this.ownsDebugger = false;
    this.events.emit("interrupted", "Debugger detached");
    this.onPause(`Debugger detached: ${text(reason, 200)}`);
  };
  private readonly onNavigation = (_event: unknown, _url: string, _inPlace: boolean, main: boolean): void => {
    if (main) this.resetDocument();
  };
  private readonly onDestroyed = (): void => {
    this.events.emit("interrupted", "Browser view closed");
    if (!this.disposed) this.onPause("Browser view closed");
  };
  private readonly onKey = (_event: unknown, input: { type: string; key: string }): void => {
    if (!this.attached) return;
    if (this.synthetic?.kind === "key" && this.synthetic.type === input.type && this.synthetic.key === input.key) {
      this.synthetic = undefined;
      return;
    }
    this.events.emit("interrupted", "Human keyboard input");
    this.onPause("Human keyboard input");
  };
  private readonly onMouse = (_event: unknown, input: { type: string; x: number; y: number }): void => {
    if (!this.attached) return;
    if (this.synthetic?.kind === "mouse" && this.synthetic.type === input.type &&
      Math.abs((this.synthetic.x ?? -1) - input.x) <= 1 && Math.abs((this.synthetic.y ?? -1) - input.y) <= 1) {
      this.synthetic = undefined;
      return;
    }
    if (["mouseDown", "mouseUp", "mouseWheel", "mouseMove"].includes(input.type)) {
      this.events.emit("interrupted", "Human mouse input");
      this.onPause("Human mouse input");
    }
  };
  private readonly onMessage = (_event: unknown, method: string, params: Args): void => {
    if (!this.attached) return;
    this.events.emit(method, params);
    if (method === "Runtime.consoleAPICalled") {
      const values = Array.isArray(params.args) ? params.args as Args[] : [];
      this.consoleCount++;
      this.push(this.consoleEntries, {
        level: text(params.type, 30),
        // Do not serialize objects: they can contain passwords, cookies or headers.
        text: values.slice(0, 20).map(v => v.type === "string" ? text(v.value) :
          ["number", "boolean", "undefined"].includes(String(v.type)) ? String(v.value).slice(0, 80) : "[object]").join(" ").slice(0, 2000),
      });
    } else if (method === "Runtime.exceptionThrown") {
      this.consoleCount++;
      const details = params.exceptionDetails as Args | undefined;
      this.push(this.consoleEntries, { level: "error", text: text(details?.text, 2000) });
    } else if (method === "Network.requestWillBeSent") {
      const request = params.request as Args | undefined;
      if (this.requests.size >= 1000) this.requests.delete(this.requests.keys().next().value!);
      this.requests.set(String(params.requestId), { url: this.safeUrl(request?.url), method: text(request?.method, 20) });
    } else if (method === "Network.loadingFailed") {
      this.failureCount++;
      this.push(this.failures, { ...this.requests.get(String(params.requestId)), error: text(params.errorText, 300),
        canceled: params.canceled === true });
      this.requests.delete(String(params.requestId));
    } else if (method === "Network.responseReceived") {
      const response = params.response as Args | undefined;
      if (typeof response?.status === "number" && response.status >= 400) {
        this.failureCount++;
        this.push(this.failures, { url: this.safeUrl(response.url), status: response.status });
      }
    } else if (method === "Network.loadingFinished") {
      this.requests.delete(String(params.requestId));
    } else if (method === "Page.javascriptDialogOpening") {
      this.consoleCount++;
      this.push(this.consoleEntries, { level: "dialog", type: text(params.type, 30),
        text: "JavaScript dialog dismissed. Use browser_handoff for human flows." });
      void this.contents.debugger.sendCommand("Page.handleJavaScriptDialog", { accept: false }).catch(() => {});
    } else if (method === "DOM.documentUpdated") {
      this.resetDocument();
    }
    if (["Network.requestWillBeSent", "Network.loadingFailed", "Network.loadingFinished"].includes(method))
      this.events.emit("networkChanged");
  };

  constructor(private readonly contents: WebContents, private readonly onPause: (reason: string) => void) {
    contents.debugger.on("detach", this.onDetach);
    contents.debugger.on("message", this.onMessage);
    contents.on("did-start-navigation", this.onNavigation);
    contents.on("destroyed", this.onDestroyed);
    contents.on("before-input-event", this.onKey);
    contents.on("before-mouse-event", this.onMouse);
    try {
      // An existing attachment belongs to DevTools or another owner.
      if (contents.debugger.isAttached()) throw Error("Browser debugger is already in use");
      contents.debugger.attach("1.3");
      this.ownsDebugger = true;
      this.ready = this.initialize();
    } catch (error) {
      this.ready = Promise.reject(error);
    }
    void this.ready.catch(() => {});
  }

  get attached(): boolean { return !this.disposed && this.ownsDebugger && this.contents.debugger.isAttached(); }

  private async initialize(): Promise<void> {
    for (const domain of ["Page", "DOM", "Runtime", "Network", "Accessibility"]) {
      if (this.disposed) throw Error("Browser automation disposed");
      await this.contents.debugger.sendCommand(`${domain}.enable`);
    }
    await this.contents.debugger.sendCommand("Runtime.addBinding", { name: "__akraBrowserWait" });
    await this.contents.debugger.sendCommand("Page.setLifecycleEventsEnabled", { enabled: true });
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.events.emit("interrupted", "Browser automation disposed");
    this.contents.removeListener("did-start-navigation", this.onNavigation);
    this.contents.removeListener("destroyed", this.onDestroyed);
    this.contents.removeListener("before-input-event", this.onKey);
    this.contents.removeListener("before-mouse-event", this.onMouse);
    this.contents.debugger.removeListener("message", this.onMessage);
    this.contents.debugger.removeListener("detach", this.onDetach);
    if (this.ownsDebugger && this.contents.debugger.isAttached()) this.contents.debugger.detach();
    this.refs.clear();
    this.refKeys.clear();
    this.requests.clear();
    this.previousImage = undefined;
    this.events.removeAllListeners();
  }

  async execute(tool: BrowserAutomationTool, raw: unknown, signal: AbortSignal): Promise<BrowserToolResult> {
    if (this.busy) return result({ error: "Browser automation is busy" }, true);
    this.busy = true;
    const local = new AbortController();
    const abort = (): void => local.abort(signal.reason);
    const interrupt = (reason: string): void => local.abort(new Error(reason));
    signal.addEventListener("abort", abort, { once: true });
    this.events.on("interrupted", interrupt);
    if (signal.aborted) abort();
    try {
      const args = this.validate(tool, raw);
      await this.cancelable(this.ready, local.signal);
      this.guard(local.signal);
      const startConsole = this.consoleCount, startFailure = this.failureCount;
      if (tool === "browser_console" || tool === "browser_network_failures") {
        const buffer = tool === "browser_console" ? this.consoleEntries : this.failures;
        const entries = buffer.slice(-Number(args.limit ?? LIMIT));
        if (args.clear) buffer.length = 0;
        return result({ entries, total: tool === "browser_console" ? this.consoleCount : this.failureCount });
      }
      if (tool === "browser_snapshot") return result(await this.snapshot(local.signal, args.diff !== false, Number(args.maxChars ?? 24000), args.interactive === true));
      if (tool === "browser_screenshot") return await this.screenshot(args, local.signal);
      if (tool === "browser_wait") await this.wait(args, local.signal);
      else if (["browser_navigate", "browser_back", "browser_forward", "browser_reload"].includes(tool))
        await this.navigate(tool, args, local.signal);
      else await this.action(tool, args, local.signal);
      const snapshot = await this.snapshot(local.signal, true, 12000);
      return result({ url: this.safeUrl(this.contents.getURL()), snapshot, newConsole: this.consoleCount - startConsole,
        newNetworkFailures: this.failureCount - startFailure });
    } catch (error) {
      return result({ error: errorText(error) }, true);
    } finally {
      if (this.attached) {
        void this.contents.debugger.sendCommand("Runtime.releaseObjectGroup", { objectGroup: "akra-browser" }).catch(() => {});
      }
      this.busy = false;
      signal.removeEventListener("abort", abort);
      this.events.removeListener("interrupted", interrupt);
    }
  }

  private guard(signal: AbortSignal): void {
    if (signal.aborted) throw Error("Browser operation canceled");
    if (!this.attached || this.contents.isDestroyed()) throw Error("Browser debugger is detached");
  }

  private cancelable<T>(promise: Promise<T>, signal: AbortSignal): Promise<T> {
    if (signal.aborted) return Promise.reject(Error("Browser operation canceled"));
    return new Promise<T>((resolve, reject) => {
      const abort = (): void => { cleanup(); reject(Error("Browser operation canceled")); };
      const cleanup = (): void => signal.removeEventListener("abort", abort);
      signal.addEventListener("abort", abort, { once: true });
      promise.then(value => { cleanup(); resolve(value); }, error => { cleanup(); reject(error); });
    });
  }

  private async command<T = Args>(method: string, args: Args, signal: AbortSignal): Promise<T> {
    this.guard(signal);
    return this.cancelable(this.contents.debugger.sendCommand(method, args) as Promise<T>, signal);
  }

  private validate(tool: BrowserAutomationTool, raw: unknown): Args {
    if (raw === null || typeof raw !== "object" || Array.isArray(raw)) throw Error("Arguments must be an object");
    const args = raw as Args;
    const fields: Record<BrowserAutomationTool, readonly string[]> = {
      browser_navigate: ["url"], browser_back: [], browser_forward: [], browser_reload: [],
      browser_snapshot: ["diff", "maxChars", "interactive"], browser_click: ["ref"], browser_type: ["ref", "text", "clear", "submit"],
      browser_press: ["ref", "key"], browser_select: ["ref", "values"], browser_scroll: ["ref", "x", "y"],
      browser_wait: ["ref", "text", "state", "event", "timeoutMs"], browser_screenshot: ["ref", "stable"],
      browser_console: ["clear", "limit"], browser_network_failures: ["clear", "limit"],
    };
    if (!fields[tool] || Object.keys(args).some(key => !fields[tool].includes(key))) throw Error("Unknown browser argument");
    for (const key of ["ref", "text", "key", "url", "state", "event"]) {
      if (args[key] !== undefined && (typeof args[key] !== "string" ||
        (args[key] as string).length > (["ref", "key"].includes(key) ? 128 : 32768)))
        throw Error(`Invalid ${key}`);
    }
    for (const key of ["clear", "diff", "stable", "interactive", "submit"])
      if (args[key] !== undefined && typeof args[key] !== "boolean") throw Error(`Invalid ${key}`);
    for (const [key, min, max] of [["maxChars", 100, 48000], ["timeoutMs", 1, 45000], ["limit", 1, LIMIT],
      ["x", -100000, 100000], ["y", -100000, 100000]] as const)
      if (args[key] !== undefined && (typeof args[key] !== "number" || !Number.isFinite(args[key]) ||
        args[key] < min || args[key] > max)) throw Error(`Invalid ${key}`);
    if (["browser_click", "browser_type", "browser_select"].includes(tool) && !args.ref) throw Error("ref is required");
    if (tool === "browser_type" && typeof args.text !== "string") throw Error("text is required");
    if (tool === "browser_press" && !args.key) throw Error("key is required");
    if (tool === "browser_select" && (!Array.isArray(args.values) || args.values.length > 100 ||
      !args.values.every(value => typeof value === "string" && value.length <= 32768))) throw Error("Invalid values");
    if (tool === "browser_navigate") {
      if (typeof args.url !== "string") throw Error("url is required");
      const url = new URL(args.url);
      if (!["http:", "https:"].includes(url.protocol) && url.href !== "about:blank") throw Error("Only HTTP(S) or about:blank navigation is allowed");
      if (url.username || url.password) throw Error("URL credentials are not allowed");
    }
    if (tool === "browser_wait") {
      if ((!args.ref && !args.text && !args.event) || args.event && (args.ref || args.text))
        throw Error("Wait requires a ref/text condition or an event");
      if (args.state !== undefined && !["visible", "hidden", "attached", "detached"].includes(String(args.state))) throw Error("Invalid wait state");
      if (args.event !== undefined && !["load", "domcontentloaded", "navigation", "networkidle"].includes(String(args.event))) throw Error("Invalid wait event");
    }
    return args;
  }

  private resetDocument(): void {
    this.generation++;
    this.refs.clear();
    this.refKeys.clear();
    this.frames.clear();
    this.requests.clear();
    this.previousSnapshot = "";
    this.previousImage = undefined;
    this.previousImageKey = "";
    this.events.emit("documentChanged");
  }

  private safeUrl(raw: unknown): string {
    try {
      const url = new URL(String(raw));
      url.username = ""; url.password = ""; url.search = ""; url.hash = "";
      return url.href.slice(0, 2000);
    } catch { return ""; }
  }

  private push(buffer: Args[], entry: Args): void {
    buffer.push(entry);
    if (buffer.length > LIMIT) buffer.shift();
  }

  private async readFrames(signal: AbortSignal): Promise<Frame[]> {
    const { frameTree } = await this.command<{ frameTree: FrameTree }>("Page.getFrameTree", {}, signal);
    const frames: Frame[] = [];
    const visit = (tree: FrameTree): void => {
      if (frames.length >= 100) return;
      frames.push(tree.frame);
      for (const child of tree.childFrames ?? []) visit(child);
    };
    visit(frameTree);
    this.frames = new Map(frames.map(frame => [frame.id, frame]));
    return frames;
  }

  private sameOrigin(frame: Frame, root: Frame): boolean {
    if (frame.id === root.id) return true;
    if (frame.securityOrigin !== undefined && root.securityOrigin !== undefined &&
      (["null", "://", ""].includes(frame.securityOrigin) || frame.securityOrigin !== root.securityOrigin)) return false;
    const parent = frame.parentId ? this.frames.get(frame.parentId) : undefined;
    if (parent && !this.sameOrigin(parent, root)) return false;
    if (frame.url === "about:blank" || frame.url === "about:srcdoc") {
      return !!parent && this.sameOrigin(parent, root);
    }
    try { return new URL(frame.url).origin === new URL(root.url).origin; } catch { return false; }
  }

  private async snapshot(signal: AbortSignal, diff: boolean, maxChars: number, interactive = false): Promise<Snapshot> {
    const frames = await this.readFrames(signal);
    const root = frames[0];
    if (!root) throw Error("Browser has no document");
    const lines: string[] = [];
    let length = 0, truncated = false, omittedListItems = 0;
    const epoch = this.generation;
    for (const frame of frames) {
      if (!this.sameOrigin(frame, root)) {
        const line = `[cross-origin frame ${this.safeUrl(frame.url)}: actions rejected]`;
        if (length + line.length > maxChars) { truncated = true; break; }
        lines.push(line); length += line.length + 1;
        continue;
      }
      const { nodes } = await this.command<{ nodes: AXNode[] }>("Accessibility.getFullAXTree", { frameId: frame.id, depth: 30 }, signal);
      if (epoch !== this.generation) throw Error("Document changed during snapshot");
      const byId = new Map(nodes.map(node => [node.nodeId, node]));
      const counts = new Map<string, number>(), omitted = new Set<string>();
      for (const node of nodes) {
        if (node.ignored || node.role?.value !== "listitem") continue;
        const parent = node.parentId ?? "";
        const count = (counts.get(parent) ?? 0) + 1;
        counts.set(parent, count);
        if (count > LIST_PREVIEW_ITEMS) omitted.add(node.nodeId);
      }
      omittedListItems += omitted.size;
      if (omitted.size && !interactive) {
        const summary = `... ${omitted.size} more list items; interactive controls retained`;
        if (length + summary.length <= maxChars) { lines.push(summary); length += summary.length + 1; }
      }
      for (const node of nodes) {
        if (node.ignored) continue;
        const role = text(node.role?.value, 80);
        const actionable = INTERACTIVE_ROLES.has(role);
        if (!actionable) {
          let ancestor: AXNode | undefined = node;
          let hiddenBySummary = false;
          for (let depth = 0; ancestor && depth < 64; depth++) {
            if (omitted.has(ancestor.nodeId)) { hiddenBySummary = true; break; }
            ancestor = ancestor.parentId ? byId.get(ancestor.parentId) : undefined;
          }
          if (hiddenBySummary) continue;
        }
        if (interactive && !actionable || !node.name?.value && ["generic", "group", "none"].includes(role)) continue;
        let ref = "";
        if (node.backendDOMNodeId && actionable) {
          const key = `${frame.id}:${node.backendDOMNodeId}`;
          ref = this.refKeys.get(key) ?? "";
          if (!ref) {
            if (this.refs.size >= 10000) { truncated = true; break; }
            ref = `r${nextRef++}`;
            this.refKeys.set(key, ref);
            this.refs.set(ref, { backend: node.backendDOMNodeId, frame: frame.id, generation: epoch });
          }
        }
        let password = node.properties?.some(p => p.name === "protected" && p.value.value === true);
        if (node.backendDOMNodeId && ["textbox", "searchbox"].includes(role)) {
          const description = await this.command<{ node: { attributes?: string[] } }>("DOM.describeNode",
            { backendNodeId: node.backendDOMNodeId, depth: 0 }, signal);
          const attributes = description.node.attributes ?? [];
          for (let i = 0; i < attributes.length; i += 2)
            if (attributes[i] === "type" && attributes[i + 1]?.toLowerCase() === "password") password = true;
        }
        const properties = node.properties?.filter(p => ["disabled", "checked", "selected", "expanded", "required"].includes(p.name))
          .map(p => `${p.name}=${String(p.value.value).slice(0, 30)}`).join(" ") ?? "";
        const line = `${ref ? `[${ref}] ` : ""}${role} ${JSON.stringify(text(node.name?.value, 500))}` +
          `${password ? " value=[redacted]" : node.value ? ` value=${JSON.stringify(text(node.value.value, 500))}` : ""}${properties ? ` ${properties}` : ""}`;
        if (length + line.length > maxChars) { truncated = true; break; }
        lines.push(line); length += line.length + 1;
      }
      if (truncated) break;
    }
    if (epoch !== this.generation) throw Error("Document changed during snapshot");
    if (truncated && !interactive) return this.snapshot(signal, diff, maxChars, true);
    const full = lines.join("\n").slice(0, maxChars);
    const changed = full !== this.previousSnapshot;
    let output = full;
    if (diff && this.previousSnapshot) {
      const previous = new Set(this.previousSnapshot.split("\n")), current = new Set(lines);
      output = [...this.previousSnapshot.split("\n").filter(line => !current.has(line)).map(line => `- ${line}`),
        ...lines.filter(line => !previous.has(line)).map(line => `+ ${line}`)].join("\n");
      truncated ||= output.length > maxChars;
      output = output.slice(0, maxChars);
    }
    this.previousSnapshot = full;
    return { url: this.safeUrl(this.contents.getURL()), contentKind: "untrusted_page_data", text: output, changed, truncated, omittedListItems };
  }

  private async resolve(refName: string, signal: AbortSignal): Promise<string> {
    const ref = this.refs.get(refName);
    if (!ref || ref.generation !== this.generation) throw Error("Stale or unknown ref; take a new snapshot");
    const frames = await this.readFrames(signal);
    if (ref.generation !== this.generation) throw Error("Stale ref; document changed");
    const frame = this.frames.get(ref.frame);
    if (!frame || !frames[0] || !this.sameOrigin(frame, frames[0])) throw Error("Cross-origin or detached frame actions are not supported");
    const { object } = await this.command<{ object: { objectId?: string } }>("DOM.resolveNode",
      { backendNodeId: ref.backend, objectGroup: "akra-browser" }, signal);
    if (!object.objectId) throw Error("Stale ref: node is detached");
    return object.objectId;
  }

  private async nodeCall(ref: string, operation: string, args: Args, signal: AbortSignal): Promise<Args> {
    const objectId = await this.resolve(ref, signal);
    return this.call(objectId, NODE_ACTION, [operation, args], signal);
  }

  private async call(objectId: string, functionDeclaration: string, values: unknown[], signal: AbortSignal): Promise<Args> {
    const response = await this.command<{ result: { value?: Args }; exceptionDetails?: Args }>("Runtime.callFunctionOn",
      { objectId, functionDeclaration, arguments: values.map(value => ({ value })), returnByValue: true }, signal);
    if (response.exceptionDetails) throw Error(text((response.exceptionDetails.exception as Args | undefined)?.description, 400) ||
      text(response.exceptionDetails.text, 400) || "Page action failed");
    return response.result.value ?? {};
  }

  private async evaluate(expression: string, signal: AbortSignal): Promise<void> {
    const response = await this.command("Runtime.evaluate", { expression, returnByValue: true }, signal);
    if (response.exceptionDetails) {
      const details = response.exceptionDetails as Args;
      throw Error(text((details.exception as Args | undefined)?.description, 400) || "Page action failed");
    }
  }

  private async syntheticCommand(method: string, params: Args, expected: NonNullable<BrowserAutomation["synthetic"]>, signal: AbortSignal): Promise<void> {
    this.guard(signal);
    this.synthetic = expected;
    try { await this.command(method, params, signal); } finally { this.synthetic = undefined; }
  }

  private async action(tool: BrowserAutomationTool, args: Args, signal: AbortSignal): Promise<void> {
    const ref = String(args.ref ?? "");
    if (tool === "browser_click") {
      const point = await this.nodeCall(ref, "click", {}, signal);
      await this.syntheticCommand("Input.dispatchMouseEvent", { type: "mouseMoved", x: point.x, y: point.y },
        { kind: "mouse", type: "mouseMove", x: Number(point.x), y: Number(point.y) }, signal);
      for (const type of ["mousePressed", "mouseReleased"]) {
        await this.syntheticCommand("Input.dispatchMouseEvent", { type, x: point.x, y: point.y, button: "left", clickCount: 1 },
          { kind: "mouse", type: type === "mousePressed" ? "mouseDown" : "mouseUp", x: Number(point.x), y: Number(point.y) }, signal);
      }
    } else if (tool === "browser_type") {
      const point = await this.nodeCall(ref, "type", { clear: args.clear !== false }, signal);
      await this.syntheticCommand("Input.dispatchMouseEvent", { type: "mouseMoved", x: point.x, y: point.y },
        { kind: "mouse", type: "mouseMove", x: Number(point.x), y: Number(point.y) }, signal);
      await this.command("Input.insertText", { text: args.text }, signal);
      if (args.submit) await this.action("browser_press", { key: "Enter" }, signal);
    } else if (tool === "browser_select") {
      await this.nodeCall(ref, "select", args, signal);
    } else if (tool === "browser_scroll") {
      const deltaX = Number(args.x ?? 0), deltaY = Number(args.y ?? 600);
      if (ref) await this.nodeCall(ref, "scroll", { deltaX, deltaY }, signal);
      else await this.evaluate(`window.scrollBy({left:${deltaX},top:${deltaY},behavior:"instant"})`, signal);
    } else if (tool === "browser_press") {
      if (ref) await this.nodeCall(ref, "focus", {}, signal);
      const focus = await this.command<{ result?: { objectId?: string }; exceptionDetails?: Args }>("Runtime.evaluate",
        { expression: PASSWORD_FOCUS, objectGroup: "akra-browser" }, signal);
      if (focus.exceptionDetails) throw Error("Password or inaccessible frame input is not allowed");
      if (focus.result?.objectId) {
        const { node } = await this.command<{ node: { shadowRoots?: { shadowRootType?: string }[] } }>("DOM.describeNode",
          { objectId: focus.result.objectId, depth: 1, pierce: true }, signal);
        if (node.shadowRoots?.some(root => root.shadowRootType === "closed"))
          throw Error("Closed shadow focus cannot be inspected safely; use browser_handoff");
      }
      const key = String(args.key);
      const special: Record<string, { code: string; value: number; text?: string }> = {
        Enter: { code: "Enter", value: 13, text: "\r" }, Tab: { code: "Tab", value: 9 },
        Escape: { code: "Escape", value: 27 }, Backspace: { code: "Backspace", value: 8 },
        Delete: { code: "Delete", value: 46 }, ArrowLeft: { code: "ArrowLeft", value: 37 },
        ArrowUp: { code: "ArrowUp", value: 38 }, ArrowRight: { code: "ArrowRight", value: 39 },
        ArrowDown: { code: "ArrowDown", value: 40 }, Home: { code: "Home", value: 36 },
        End: { code: "End", value: 35 }, PageUp: { code: "PageUp", value: 33 }, PageDown: { code: "PageDown", value: 34 },
        Space: { code: "Space", value: 32, text: " " },
      };
      const entry = special[key];
      if (!entry && [...key].length !== 1) throw Error("Unsupported key");
      const inputKey = key === "Space" ? " " : key;
      const character = entry ? entry.text : key;
      for (const type of character ? ["rawKeyDown", "char", "keyUp"] : ["rawKeyDown", "keyUp"]) {
        await this.syntheticCommand("Input.dispatchKeyEvent", { type, key: inputKey,
          ...(entry ? { code: entry.code, windowsVirtualKeyCode: entry.value, nativeVirtualKeyCode: entry.value } : {}),
          ...(type === "char" ? { text: character } : {}) },
        { kind: "key", type: type === "rawKeyDown" ? "keyDown" : type, key: inputKey }, signal);
      }
    } else throw Error("Unsupported browser tool");
  }

  private eventWait(event: string, signal: AbortSignal, timeoutMs: number, match: (params: Args) => boolean = () => true): { promise: Promise<void>; cancel: () => void } {
    let cancel = (): void => {};
    const promise = new Promise<void>((resolve, reject) => {
      const cleanup = (): void => {
        clearTimeout(timer);
        this.events.removeListener(event, listener);
        signal.removeEventListener("abort", abort);
      };
      const listener = (params: Args): void => { if (match(params)) { cleanup(); resolve(); } };
      const abort = (): void => { cleanup(); reject(Error("Browser operation canceled")); };
      const timer = setTimeout(() => { cleanup(); reject(Error("Browser wait timed out")); }, timeoutMs);
      cancel = () => { cleanup(); reject(Error("Browser wait canceled")); };
      this.events.on(event, listener);
      signal.addEventListener("abort", abort, { once: true });
      if (signal.aborted) abort();
    });
    void promise.catch(() => {});
    return { promise, cancel };
  }

  private async navigate(tool: BrowserAutomationTool, args: Args, signal: AbortSignal): Promise<void> {
    let command = "Page.reload", params: Args = {};
    if (tool === "browser_navigate") { command = "Page.navigate"; params = { url: args.url }; }
    else if (tool !== "browser_reload") {
      const history = await this.command<{ currentIndex: number; entries: { id: number }[] }>("Page.getNavigationHistory", {}, signal);
      const entry = history.entries[history.currentIndex + (tool === "browser_back" ? -1 : 1)];
      if (!entry) throw Error("No browser history entry");
      command = "Page.navigateToHistoryEntry"; params = { entryId: entry.id };
    }
    const root = (await this.readFrames(signal))[0];
    const wait = this.eventWait("Page.loadEventFired", signal, 30000);
    const sameDocument = this.eventWait("Page.navigatedWithinDocument", signal, 30000, p => p.frameId === root?.id);
    try {
      const response = await this.command(command, params, signal);
      if (response.errorText) throw Error(text(response.errorText, 300));
      // Same-document navigation has no loader and no load event.
      if (command === "Page.navigate" && !response.loaderId) return;
      await Promise.race([wait.promise, sameDocument.promise]);
    } finally { wait.cancel(); sameDocument.cancel(); }
  }

  private async wait(args: Args, signal: AbortSignal): Promise<void> {
    const timeout = Number(args.timeoutMs ?? 10000);
    if (args.event === "networkidle") {
      await new Promise<void>((resolve, reject) => {
        let quiet: ReturnType<typeof setTimeout> | undefined;
        const cleanup = () => {
          clearTimeout(quiet);
          clearTimeout(deadline);
          this.events.off("networkChanged", changed);
          signal.removeEventListener("abort", aborted);
        };
        const aborted = () => { cleanup(); reject(Error("Browser operation canceled")); };
        const changed = () => {
          clearTimeout(quiet);
          if (this.requests.size === 0) quiet = setTimeout(() => { cleanup(); resolve(); }, NETWORK_IDLE_MS);
        };
        const deadline = setTimeout(() => { cleanup(); reject(Error("Browser wait timed out")); }, timeout);
        this.events.on("networkChanged", changed);
        signal.addEventListener("abort", aborted, { once: true });
        if (signal.aborted) aborted();
        else changed();
      });
      return;
    }
    if (args.event) {
      const name = args.event === "load" ? "Page.loadEventFired" :
        args.event === "domcontentloaded" ? "Page.domContentEventFired" : "Page.frameNavigated";
      const wait = this.eventWait(name, signal, timeout, p => args.event !== "navigation" || !(p.frame as Args | undefined)?.parentId);
      await wait.promise;
      return;
    }
    const id = `w${nextWait++}`;
    const objectId = args.ref ? await this.resolve(String(args.ref), signal) : undefined;
    const wait = this.eventWait("Runtime.bindingCalled", signal, timeout, p => {
      if (p.name !== "__akraBrowserWait") return false;
      try { return (JSON.parse(String(p.payload)) as Args).id === id; } catch { return false; }
    });
    const changed = (): void => wait.cancel();
    this.events.on("documentChanged", changed);
    try {
      const source = waitInstaller(id, args.text as string | undefined, !!objectId, String(args.state ?? "visible"), timeout);
      if (objectId) await this.call(objectId, source, [], signal);
      else await this.evaluate(`(${source})()`, signal);
      await wait.promise;
    } finally {
      wait.cancel();
      this.events.removeListener("documentChanged", changed);
      if (this.attached) {
        // Cleanup must still run after cancellation. It has no page side effects.
        const cleanup = `globalThis.__akraBrowserWaits?.get(${JSON.stringify(id)})?.()`;
        void this.contents.debugger.sendCommand(objectId ? "Runtime.callFunctionOn" : "Runtime.evaluate",
          objectId ? { objectId, functionDeclaration: `function(){ ${cleanup}; }` } : { expression: cleanup }).catch(() => {});
      }
    }
  }

  private imageSample(image: NativeImage): Uint8Array {
    const bitmap = image.resize({ width: 64, height: 64, quality: "good" }).toBitmap();
    const sample = new Uint8Array(4096);
    for (let i = 0; i < sample.length; i++) sample[i] =
      Math.round((Number(bitmap[i * 4]) + Number(bitmap[i * 4 + 1]) + Number(bitmap[i * 4 + 2])) / 3);
    return sample;
  }

  private changedPixels(a: Uint8Array, b: Uint8Array): number {
    let changed = 0;
    for (let i = 0; i < a.length; i++) if (Math.abs(Number(a[i]) - Number(b[i])) > 8) changed++;
    return changed / a.length;
  }

  private delay(signal: AbortSignal): Promise<void> {
    return new Promise((resolve, reject) => {
      const cleanup = (): void => signal.removeEventListener("abort", abort);
      const abort = (): void => { clearTimeout(timer); cleanup(); reject(Error("Browser operation canceled")); };
      const timer = setTimeout(() => { cleanup(); resolve(); }, 200);
      signal.addEventListener("abort", abort, { once: true });
      if (signal.aborted) abort();
    });
  }

  private async screenshot(args: Args, signal: AbortSignal): Promise<BrowserToolResult> {
    const crop = args.ref ? await this.nodeCall(String(args.ref), "crop", {}, signal) : undefined;
    const rect = crop ? { x: Math.max(0, Math.floor(Number(crop.x))), y: Math.max(0, Math.floor(Number(crop.y))),
      width: Math.min(16000, Math.ceil(Number(crop.width))), height: Math.min(16000, Math.ceil(Number(crop.height))) } : undefined;
    if (rect && (rect.width <= 0 || rect.height <= 0)) throw Error("Ref is outside the viewport");
    let image: NativeImage | undefined, sample: Uint8Array | undefined, prior: Uint8Array | undefined;
    const epoch = this.generation;
    const imageKey = `${epoch}:${String(args.ref ?? "viewport")}`;
    let stable = !args.stable;
    const deadline = Date.now() + 3000;
    for (let attempt = 0; attempt < (args.stable ? 16 : 1); attempt++) {
      this.guard(signal);
      if (attempt) {
        if (Date.now() + 200 > deadline) break;
        await this.delay(signal);
      }
      this.guard(signal);
      image = await this.cancelable(this.contents.capturePage(rect), signal);
      if (epoch !== this.generation) throw Error("Document changed during screenshot");
      if (image.isEmpty()) throw Error("Browser screenshot is empty");
      sample = this.imageSample(image);
      if (!args.stable || prior && this.changedPixels(prior, sample) < 0.01) { stable = true; break; }
      prior = sample;
    }
    this.guard(signal);
    if (!image || !sample) throw Error("Browser screenshot is empty");
    if (this.previousImageKey === imageKey && this.previousImage && this.changedPixels(this.previousImage, sample) < 0.01)
      return result({ changed: false, stable, text: "Screenshot unchanged (less than 1% changed pixels)." });
    const size = image.getSize();
    if (Math.max(size.width, size.height) > 1280) {
      const scale = 1280 / Math.max(size.width, size.height);
      image = image.resize({ width: Math.round(size.width * scale), height: Math.round(size.height * scale), quality: "good" });
    }
    const jpeg = image.toJPEG(70);
    if (jpeg.length > 4 * 1024 * 1024) throw Error("Browser screenshot exceeds image limit");
    this.previousImage = sample;
    this.previousImageKey = imageKey;
    return { content: [
      { type: "text", text: JSON.stringify({ changed: true, stable, width: image.getSize().width, height: image.getSize().height }) },
      { type: "image", data: jpeg.toString("base64"), mimeType: "image/jpeg" },
    ] };
  }
}
