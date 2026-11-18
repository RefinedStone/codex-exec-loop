export const BROWSER_TOOL_NAMES = [
  "browser_status", "browser_navigate", "browser_back", "browser_forward", "browser_reload",
  "browser_snapshot", "browser_click", "browser_type", "browser_press", "browser_select",
  "browser_scroll", "browser_wait", "browser_screenshot", "browser_console",
  "browser_network_failures", "browser_handoff",
] as const;

export type BrowserToolName = typeof BROWSER_TOOL_NAMES[number];
export type BrowserAutomationTool = Exclude<BrowserToolName, "browser_status" | "browser_handoff">;

export function isBrowserToolName(value: unknown): value is BrowserToolName {
  return BROWSER_TOOL_NAMES.some((name) => name === value);
}

export type BrowserToolContent =
  | { readonly type: "text"; readonly text: string }
  | { readonly type: "image"; readonly data: string; readonly mimeType: "image/jpeg" };

export interface BrowserToolResult {
  readonly content: readonly BrowserToolContent[];
  readonly isError?: boolean;
}

export interface BrowserRegistration {
  readonly registrationId: string;
  readonly sessionFile: string;
  readonly command: string;
  readonly args: readonly string[];
}

export interface BrowserAgentState {
  readonly phase: "idle" | "active" | "paused" | "handoff" | "unavailable";
  readonly threadId: string | null;
  readonly lastAction: string | null;
  readonly reason: string | null;
  readonly attached: boolean;
}

export interface BrowserAgentBridge {
  prepare(): Promise<BrowserRegistration>;
  bind(registrationId: string, threadId: string): Promise<void>;
  release(registrationId: string): Promise<void>;
  setActiveThread(threadId: string | null): Promise<void>;
  pause(): Promise<void>;
  resume(): Promise<void>;
  status(): Promise<BrowserAgentState>;
  onState(listener: (state: BrowserAgentState) => void): () => void;
}
