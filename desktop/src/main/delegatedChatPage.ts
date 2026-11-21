import type { PagePort } from "akra-tunnel-core/chat";
import { CHAT_CONVERSATION } from "../shared/delegatedChat";
import type { DelegatedChatWindow } from "./delegatedChatWindow";

const CHAT_ORIGIN = "https://chatgpt.com";
const AUTOMATION_WORLD = 1001;

/** The core supplies the only page functions. No debugger, generic Javascript IPC, or network inspection. */
export function createDelegatedChatPage(window: DelegatedChatWindow): PagePort {
  if (window.isManual) throw new Error("chat_manual_action");
  const contents = window.getContents();
  const generation = window.automationGeneration;
  function assertActive() {
    if (window.isManual || generation !== window.automationGeneration || contents.isDestroyed()) {
      throw new Error("chat_manual_action");
    }
  }
  function assertChatPage() {
    assertActive();
    if (!contents.getURL().startsWith(`${CHAT_ORIGIN}/`)) throw new Error("chat_page_unavailable");
  }
  return {
    assertActive,
    async evaluate(fn, args) {
      assertChatPage();
      const value = await contents.executeJavaScriptInIsolatedWorld(AUTOMATION_WORLD, [
        { code: `(${fn.toString()})(${JSON.stringify(args)})` },
      ]);
      assertActive();
      return value;
    },
    insertText(text) {
      assertChatPage();
      return contents.insertText(text);
    },
    focus() {
      assertActive();
      // The core focuses the composer inside the page. Do not bring the host window forward.
    },
    load(url) {
      assertActive();
      if (url !== `${CHAT_ORIGIN}/` && url !== `${CHAT_ORIGIN}/?temporary-chat=true` && !CHAT_CONVERSATION.test(url)) {
        throw new Error("chat_navigation_denied");
      }
      return contents.loadURL(url);
    },
    url() {
      assertActive();
      return contents.getURL();
    },
  };
}
