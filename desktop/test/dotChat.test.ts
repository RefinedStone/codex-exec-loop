import { afterEach, describe, expect, it } from "vitest";

import { allowDotSending, DOT_SEND_WARNING, hasDotRepliedSince, isDotSendingAllowed, messageTime, resetDotSendPermissions } from "../src/renderer/views/dotSend";

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

describe("hasDotRepliedSince", () => {
  const me = (id: string) => ({ id, fromDot: false });
  const dot = (id: string) => ({ id, fromDot: true });

  it("does not take the message from before the send as the reply", () => {
    // 보낸 직후의 읽기에 내 글이 아직 없다. 가장 새 메시지는 보내기 전의 dot 메시지이다.
    expect(hasDotRepliedSince([me("m1"), dot("d1")], "d1")).toBe(false);
    // 내 글만 도착했다.
    expect(hasDotRepliedSince([me("m1"), dot("d1"), me("m2")], "d1")).toBe(false);
  });

  it("sees the reply when the newest message is a new one from the dot", () => {
    expect(hasDotRepliedSince([me("m1"), dot("d1"), me("m2"), dot("d2")], "d1")).toBe(true);
    // 빈 방에서 보낸 첫 글의 답이다.
    expect(hasDotRepliedSince([me("m1"), dot("d1")], null)).toBe(true);
    expect(hasDotRepliedSince([], null)).toBe(false);
  });
});
