import { afterEach, describe, expect, it } from "vitest";

import { allowDotSending, DOT_SEND_WARNING, isDotSendingAllowed, messageTime, resetDotSendPermissions } from "../src/renderer/views/dotSend";

afterEach(() => resetDotSendPermissions());

describe("permission to send to a dot (plan 25, session once)", () => {
  it("asks once, and a refusal permits nothing", () => {
    const asked: string[] = [];
    const refuse = (message: string) => {
      asked.push(message);
      return false;
    };
    expect(isDotSendingAllowed("dot-1")).toBe(false);
    expect(allowDotSending("dot-1", refuse)).toBe(false);
    expect(isDotSendingAllowed("dot-1")).toBe(false);
    expect(asked).toEqual([DOT_SEND_WARNING]);

    const accept = (message: string) => {
      asked.push(message);
      return true;
    };
    expect(allowDotSending("dot-1", accept)).toBe(true);
    expect(isDotSendingAllowed("dot-1")).toBe(true);
    // 이미 허락한 dot 은 다시 묻지 않는다.
    expect(allowDotSending("dot-1", refuse)).toBe(true);
    expect(asked).toHaveLength(2);
  });

  it("is separate for each dot", () => {
    allowDotSending("dot-1", () => true);
    expect(isDotSendingAllowed("dot-2")).toBe(false);
  });
});

describe("messageTime", () => {
  it("keeps a value that is not a time", () => {
    expect(messageTime("not a time")).toBe("not a time");
    expect(messageTime("2026-11-14T01:00:00Z")).not.toBe("2026-11-14T01:00:00Z");
  });
});
