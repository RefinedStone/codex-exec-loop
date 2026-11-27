import { expect, test, vi } from "vitest";

vi.mock("electron", () => ({}));

const { isDelegatedChatNavigation } = await import("../src/main/delegatedChatWindow");

const chatOrigin = "https://chatgpt.com";
const allows = (url: string, manual: boolean, mainFrame = true) =>
  isDelegatedChatNavigation(url, { chatOrigin, manual, mainFrame });

test("manual sign-in may pass through any https identity provider and its frames", () => {
  for (const url of [
    "https://auth.openai.com/log-in",
    "https://accounts.google.com/o/oauth2/v2/auth",
    "https://appleid.apple.com/auth/authorize",
    "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
  ]) {
    expect(allows(url, true)).toBe(true);
    expect(allows(url, true, false)).toBe(true);
  }
});

test("automation stays on the chat origin and the challenge frame", () => {
  expect(allows("https://chatgpt.com/c/abc", false)).toBe(true);
  expect(allows("https://challenges.cloudflare.com/turnstile", false, false)).toBe(true);
  expect(allows("https://challenges.cloudflare.com/turnstile", false, true)).toBe(false);
  expect(allows("https://auth.openai.com/log-in", false)).toBe(false);
  expect(allows("https://accounts.google.com/", false)).toBe(false);
});

test("non-https and credential URLs are refused even during manual sign-in", () => {
  expect(allows("http://accounts.google.com/", true)).toBe(false);
  expect(allows("https://user:pass@accounts.google.com/", true)).toBe(false);
  expect(allows("javascript:alert(1)", true)).toBe(false);
  expect(allows("not a url", true)).toBe(false);
});
