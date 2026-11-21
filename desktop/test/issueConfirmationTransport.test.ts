import { EventEmitter } from "node:events";
import { PassThrough } from "node:stream";
import { beforeEach, describe, expect, it, vi } from "vitest";

const childProcess = vi.hoisted(() => ({ spawn: vi.fn() }));
vi.mock("node:child_process", () => ({ spawn: childProcess.spawn }));

import { IssueConfirmationTransport } from "../src/confirmation/transport";

function child() {
  const process = Object.assign(new EventEmitter(), {
    stdin: new PassThrough(),
    stdout: new PassThrough(),
    stderr: new PassThrough(),
    exitCode: null as number | null,
    signalCode: null,
    kill: vi.fn(),
  });
  const sent: Record<string, unknown>[] = [];
  let input = "";
  process.stdin.on("data", (data: Buffer) => {
    input += data.toString();
    let end;
    while ((end = input.indexOf("\n")) >= 0) {
      sent.push(JSON.parse(input.slice(0, end)) as Record<string, unknown>);
      input = input.slice(end + 1);
    }
  });
  process.stdin.on("finish", () => {
    process.exitCode = 0;
    process.emit("exit", 0);
  });
  return { process, sent, frame: (value: unknown) => process.stdout.write(JSON.stringify(value) + "\n") };
}

describe("confirmation helper pipe", () => {
  it("keeps shutdown pending until the helper has exited after EOF", async () => {
    const fake = child();
    fake.process.stdin.removeAllListeners("finish");
    childProcess.spawn.mockReturnValue(fake.process);
    const transport = new IssueConfirmationTransport("/app/akra-gateway", vi.fn());
    const prepared = transport.prepare();
    fake.frame({ type: "ready" });
    await vi.waitFor(() => expect(fake.sent).toHaveLength(1));
    fake.frame({ id: fake.sent[0]?.id, result: { registration_id: "registration-private", session_file: "/private/session" } });
    await prepared;
    transport.close();
    let stopped = false;
    const exited = transport.waitClosed().then(() => { stopped = true; });
    await Promise.resolve();
    expect(stopped).toBe(false);
    expect(fake.process.stdin.writableEnded).toBe(true);
    fake.process.exitCode = 0;
    fake.process.emit("exit", 0);
    await exited;
    expect(stopped).toBe(true);
  });

  beforeEach(() => childProcess.spawn.mockReset());

  it("uses a fixed argv and only pipe frames for registration, intent, and proof", async () => {
    const fake = child();
    childProcess.spawn.mockReturnValue(fake.process);
    const closed = vi.fn();
    const transport = new IssueConfirmationTransport("/app/akra-gateway", closed);
    const prepared = transport.prepare();
    expect(childProcess.spawn).toHaveBeenCalledExactlyOnceWith("/app/akra-gateway", ["desktop-confirmation-bridge"], {
      detached: false, windowsHide: true, stdio: "pipe",
    });
    expect(fake.sent).toHaveLength(0);
    fake.frame({ type: "ready" });
    await vi.waitFor(() => expect(fake.sent).toHaveLength(1));
    expect(fake.sent[0]).toMatchObject({ command: "prepare" });
    fake.frame({ id: fake.sent[0]?.id, result: { registration_id: "registration-private", session_file: "/private/session" } });
    expect(await prepared).toEqual({ registration_id: "registration-private", session_file: "/private/session" });
    const approved = transport.approve("registration-private", { decision_id: "decision-1", option: "approve" });
    await vi.waitFor(() => expect(fake.sent).toHaveLength(2));
    expect(fake.sent[1]).toMatchObject({ command: "approve", registration_id: "registration-private", intent: { decision_id: "decision-1", option: "approve" } });
    fake.frame({ id: fake.sent[1]?.id, result: { proof_id: "private-proof" } });
    expect(await approved).toEqual({ proof_id: "private-proof" });
    transport.close();
    expect(closed).not.toHaveBeenCalled();
    expect(fake.process.stdin.writableEnded).toBe(true);
  });

  it("rejects oversized or malformed frames without including helper output in the error", async () => {
    const fake = child();
    childProcess.spawn.mockReturnValue(fake.process);
    const closed = vi.fn();
    const transport = new IssueConfirmationTransport("/app/akra-gateway", closed);
    const prepared = transport.prepare();
    const rejection = expect(prepared).rejects.toThrow(/^확인 증거를 준비할 수 없습니다/);
    fake.process.stdout.write("PRIVATE-PROOF".repeat(6_000));
    await rejection;
    expect(closed).toHaveBeenCalledOnce();
  });
});
