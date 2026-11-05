// 실측 녹화로 만든 픽스처를 대화 상태 모델에 그대로 흘려 넣어 시험한다.
import { readFileSync } from "node:fs";

import { describe, expect, it } from "vitest";

import { FrameBatcher } from "../src/renderer/conversation/frameBatcher";
import { applyNotifications, isTurnInProgress, transcriptFromResume } from "../src/renderer/conversation/transcript";
import type { ThreadResumeResponse } from "../src/shared/protocol/generated/v2/ThreadResumeResponse";
import { APPROVAL_TURN_RECORDING, INTERRUPTED_TURN_RECORDING } from "./fake-gateway/fakeGateway";

interface Envelope {
  id?: number | string;
  method?: string;
  params?: unknown;
  result?: unknown;
}

/** 녹화에서 thread/resume 응답과 그 뒤 gateway 알림을 꺼낸다. */
function loadConversation(path: string) {
  const lines = readFileSync(path, "utf8")
    .split(/\r?\n/)
    .filter((line) => line.trim().length > 0)
    .map((line) => JSON.parse(line) as { direction: string; text: string })
    .map((record) => ({ direction: record.direction, message: JSON.parse(record.text) as Envelope }));
  const resumeRequest = lines.find((line) => line.direction === "client" && line.message.method === "thread/resume");
  const resumeIndex = lines.findIndex(
    (line) => line.direction === "gateway" && !line.message.method && line.message.id === resumeRequest?.message.id,
  );
  const resume = lines[resumeIndex]!.message.result as ThreadResumeResponse;
  const notifications = lines
    .slice(resumeIndex + 1)
    .filter((line) => line.direction === "gateway" && line.message.method && line.message.id === undefined)
    .map((line) => ({ method: line.message.method!, params: line.message.params }));
  return { resume, notifications };
}

describe("transcript from recorded conversations", () => {
  it("streams the approval turn: command output, agent text and completion", () => {
    const { resume, notifications } = loadConversation(APPROVAL_TURN_RECORDING);
    const initial = transcriptFromResume(resume);
    expect(isTurnInProgress(initial)).toBe(true);

    const isAgentCompleted = (notification: { method: string; params: unknown }) =>
      notification.method === "item/completed" &&
      (notification.params as { item: { type: string } }).item.type === "agentMessage";
    const agentCompletedAt = notifications.findIndex(isAgentCompleted);
    const streamed = notifications.slice(0, agentCompletedAt);
    const deltas = streamed
      .filter((notification) => notification.method === "item/agentMessage/delta")
      .map((notification) => (notification.params as { delta: string }).delta)
      .join("");
    const agentText = (transcript: ReturnType<typeof transcriptFromResume>) => {
      const item = transcript.turns.at(-1)?.items.find((candidate) => candidate.type === "agentMessage");
      return item?.type === "agentMessage" ? item.text : null;
    };
    // While streaming, the agent text is the concatenation of the deltas so far.
    expect(agentText(applyNotifications(initial, streamed))).toBe(deltas);

    const final = applyNotifications(initial, notifications);
    const lastTurn = final.turns.at(-1)!;
    expect(lastTurn.status).toBe("completed");
    const command = lastTurn.items.find((item) => item.type === "commandExecution");
    expect(command).toMatchObject({ type: "commandExecution", status: "failed", exitCode: 1 });
    // item/completed carries the authoritative final text (the sanitized fixture rewrote deltas and final text separately).
    const completedText = (notifications[agentCompletedAt]!.params as { item: { text: string } }).item.text;
    expect(agentText(final)).toBe(completedText);
    expect(isTurnInProgress(final)).toBe(false);
  });

  it("marks the interrupted turn", () => {
    const { resume, notifications } = loadConversation(INTERRUPTED_TURN_RECORDING);
    const final = applyNotifications(transcriptFromResume(resume), notifications);
    expect(final.turns.at(-1)?.status).toBe("interrupted");
  });

  it("ignores notifications for other threads and unknown methods", () => {
    const { resume } = loadConversation(APPROVAL_TURN_RECORDING);
    const initial = transcriptFromResume(resume);
    const unchanged = applyNotifications(initial, [
      { method: "turn/completed", params: { threadId: "other", turn: { id: "x" } } },
      { method: "item/unknownKind/delta", params: { threadId: initial.threadId } },
    ]);
    expect(unchanged).toEqual(initial);
  });
});

describe("FrameBatcher", () => {
  it("applies everything pushed in one frame as a single ordered batch", () => {
    const frames: Array<() => void> = [];
    const batches: number[][] = [];
    const batcher = new FrameBatcher<number>((batch) => batches.push(batch), (callback) => frames.push(callback));
    batcher.push(1);
    batcher.push(2);
    batcher.push(3);
    expect(frames).toHaveLength(1);
    frames.shift()!();
    batcher.push(4);
    frames.shift()!();
    expect(batches).toEqual([[1, 2, 3], [4]]);
  });
});
