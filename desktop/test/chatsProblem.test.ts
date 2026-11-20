import { describe, expect, it } from "vitest";

import { JsonRpcError } from "../src/renderer/rpc/jsonRpcClient";
import { CHATS_RATE_LIMITED, chatsListProblem, rateLimitedMessage } from "../src/renderer/session/chatsProblem";

describe("ChatGPT history read failures (plan 25)", () => {
  it("tells the wait time of a rate limit", () => {
    const message = rateLimitedMessage(new JsonRpcError(CHATS_RATE_LIMITED, "limited", { retryAfterSeconds: 29.2 }));
    expect(message).toContain("30초");
  });

  it("gives a rate limit notice without a wait time", () => {
    const message = rateLimitedMessage(new JsonRpcError(CHATS_RATE_LIMITED, "limited"));
    expect(message).toContain("제한");
    expect(message).not.toContain("초 뒤");
  });

  it("does not take other errors for a rate limit", () => {
    expect(rateLimitedMessage(new JsonRpcError(-32000, "boom"))).toBeNull();
    expect(rateLimitedMessage(new Error("boom"))).toBeNull();
  });

  it("hides the section when the gateway has no chats method", () => {
    expect(chatsListProblem(new JsonRpcError(-32601, "unknown gateway method"))).toBeNull();
  });

  it("keeps the reason of a failed list read", () => {
    expect(chatsListProblem(new JsonRpcError(CHATS_RATE_LIMITED, "limited", { retryAfterSeconds: 5 }))).toContain("5초");
    expect(chatsListProblem(new JsonRpcError(-32000, "the chats list request failed (HTTP 401)"))).toBe("the chats list request failed (HTTP 401)");
  });
});
