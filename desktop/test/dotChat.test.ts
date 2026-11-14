import { afterEach, describe, expect, it } from "vitest";

import { reduceDotLive, type DotLiveSlice } from "../src/renderer/session/desktopStore";
import { allowDotSending, dayLabel, DOT_SEND_WARNING, dotStatusLine, hasDotRepliedSince, isDotSendingAllowed, lastReadMessageId, messageTime, resetDotSendPermissions } from "../src/renderer/views/dotSend";

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

describe("the live state of a dot room", () => {
  const empty: DotLiveSlice = { dotLive: {}, dotReplies: {}, dotUnreadRooms: [] };
  const live = (working: boolean, connected = true) => ({ roomId: "room-1", connected, working, activityLabel: working ? "Thinking" : null, usingComputer: false });

  it("counts a reply when a working room becomes idle, and marks the room unread once", () => {
    const working = reduceDotLive(empty, live(true));
    expect(working.dotLive["room-1"]?.working).toBe(true);
    expect(working.dotReplies).toEqual({});

    const done = reduceDotLive(working, live(false));
    expect(done.dotReplies).toEqual({ "room-1": 1 });
    expect(done.dotUnreadRooms).toEqual(["room-1"]);

    const again = reduceDotLive(reduceDotLive(done, live(true)), live(false));
    expect(again.dotReplies).toEqual({ "room-1": 2 });
    expect(again.dotUnreadRooms).toEqual(["room-1"]);
  });

  it("does not take the first idle state or a lost connection as a reply", () => {
    // 보기 시작할 때 서비스는 지금 상태(대기)를 한 번 민다.
    expect(reduceDotLive(empty, live(false)).dotReplies).toEqual({});
    // 일하는 중에 통로가 끊겼다. 답이 끝난 것이 아니다.
    const lost = reduceDotLive(reduceDotLive(empty, live(true)), live(false, false));
    expect(lost.dotReplies).toEqual({});
    expect(lost.dotLive["room-1"]?.connected).toBe(false);
  });

  it("does not miss a reply that finished while the channel was down", () => {
    // 일하는 중 -> 끊김 -> 다시 붙었더니 끝나 있다.
    const lost = reduceDotLive(reduceDotLive(empty, live(true)), live(false, false));
    const back = reduceDotLive(lost, live(false));
    expect(back.dotReplies).toEqual({ "room-1": 1 });
    expect(back.dotUnreadRooms).toEqual(["room-1"]);
    // 다시 붙었는데 아직 일하는 중이면 답이 아니다. 끝나면 그때 센다.
    const stillWorking = reduceDotLive(lost, live(true));
    expect(stillWorking.dotUnreadRooms).toEqual([]);
    expect(reduceDotLive(stillWorking, live(false)).dotUnreadRooms).toEqual(["room-1"]);
  });

  it("reads the chat again after a reconnect, without an unread mark, when no work was known", () => {
    const idle = reduceDotLive(empty, live(false));
    const back = reduceDotLive(reduceDotLive(idle, live(false, false)), live(false));
    expect(back.dotReplies).toEqual({ "room-1": 1 });
    expect(back.dotUnreadRooms).toEqual([]);
  });
});

describe("what the chat shows", () => {
  const message = (id: string, fromDot: boolean, createdAt: string) => ({ id, fromDot, createdAt });

  it("puts the read mark under my newest message that the dot has read", () => {
    const messages = [message("m1", false, "2026-11-14T01:00:00Z"), message("d1", true, "2026-11-14T01:00:05Z"), message("m2", false, "2026-11-14T01:01:00Z"), message("m3", false, "2026-11-14T01:02:00Z")];
    expect(lastReadMessageId(messages, "2026-11-14T01:01:30Z")).toBe("m2");
    expect(lastReadMessageId(messages, "2026-11-14T09:00:00Z")).toBe("m3");
    // 읽은 시각을 모르거나 그 전에 보낸 내 글이 없으면 표시가 없다.
    expect(lastReadMessageId(messages, null)).toBeNull();
    expect(lastReadMessageId(messages, "2026-11-13T00:00:00Z")).toBeNull();
    expect(lastReadMessageId(messages, "not a time")).toBeNull();
  });

  it("describes the dot in one line, with the live state first", () => {
    const dot = { status: "active", paused: false };
    expect(dotStatusLine(dot, { connected: true, working: true, activityLabel: "Thinking", usingComputer: false })).toEqual({ text: "Thinking…", tone: "working" });
    expect(dotStatusLine(dot, { connected: true, working: true, activityLabel: null, usingComputer: true }).text).toBe("컴퓨터로 작업하는 중…");
    expect(dotStatusLine(dot, { connected: true, working: false, activityLabel: null, usingComputer: false }).tone).toBe("idle");
    expect(dotStatusLine(dot, undefined)).toEqual({ text: "active · 실시간 연결 없음", tone: "muted" });
    expect(dotStatusLine({ status: "active", paused: true }, { connected: true, working: true, activityLabel: null, usingComputer: false }).text).toBe("멈춤");
  });

  it("names today and yesterday and gives the same label within a day", () => {
    const now = new Date(2026, 10, 14, 15, 0, 0);
    expect(dayLabel(new Date(2026, 10, 14, 0, 5, 0).toISOString(), now)).toBe("오늘");
    expect(dayLabel(new Date(2026, 10, 13, 23, 55, 0).toISOString(), now)).toBe("어제");
    const older = dayLabel(new Date(2026, 10, 2, 9, 0, 0).toISOString(), now);
    expect(older).toBe(dayLabel(new Date(2026, 10, 2, 21, 0, 0).toISOString(), now));
    expect(older).not.toBe("오늘");
    expect(dayLabel("not a time", now)).toBe("");
  });
});
