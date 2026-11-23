import { describe, expect, it } from "vitest";
import { bindingPauseControlAction, bindingPauseControlLabel, hasValidBindingPause, isBindingPauseReceipt, bindingPauseRequestNotice, preserveNewerBindingPause } from "../src/renderer/kernel/issueBindingPause";
import type { IssueAutomationAnswer, IssueBindingPauseState } from "../src/renderer/kernel/protocol";

const command = { type: "set_issue_binding_paused", command_id: "pause-1", binding_id: "binding-1", paused: true, expected_revision: 3 } as const;
const pause = (revision = 4): IssueBindingPauseState => ({ status: "ready", binding_id: "binding-1", revision, paused: true, pending: null });
const answer = (binding_pause: IssueBindingPauseState): IssueAutomationAnswer => ({ type: "issue_automation", status: "ready", binding_id: "binding-1", binding: null,
  mode: "off", epoch: 1, revision: 7, binding_pause, last_attempt_at_ms: null, last_success_at_ms: null, last_failure: null, consecutive_failures: 0,
  collecting: false, next_poll_after_ms: null });
const receipt = { command_id: "pause-1", binding_id: "binding-1", revision: 4, paused: true, outcome: "paused", changed: true } as const;

describe("binding pause state and acceptance", () => {
  it("keeps the legacy omission readable and rejects another binding or contradictory completion", () => {
    expect(hasValidBindingPause({ binding_id: "binding-1" })).toBe(true);
    expect(hasValidBindingPause({ binding_id: "binding-1", binding_pause: pause() })).toBe(true);
    expect(hasValidBindingPause({ binding_id: "binding-2", binding_pause: pause() })).toBe(false);
    expect(hasValidBindingPause({ binding_id: "binding-1", binding_pause: { ...pause(), revision: -1 } })).toBe(false);
    expect(isBindingPauseReceipt({ ...receipt, outcome: "resumed" })).toBe(false);
    expect(isBindingPauseReceipt({ ...receipt, changed: false })).toBe(false);
  });

  it.each([
    { kind: "padded", bindingId: " legacy binding " },
    { kind: "over 512 bytes", bindingId: "x".repeat(513) },
  ])("reads an exact $kind legacy binding ID without accepting it as a new completion", ({ bindingId }) => {
    const current: IssueBindingPauseState = { status: "ready", binding_id: bindingId, revision: 0, paused: false, pending: null };
    const query = { ...answer(current), binding_id: bindingId };
    expect(hasValidBindingPause(query)).toBe(true);
    expect(hasValidBindingPause({ ...query, binding_pause: { status: "unavailable", binding_id: bindingId } })).toBe(true);
    expect(hasValidBindingPause({ ...query, binding_id: `${bindingId}-other` })).toBe(false);
    expect(hasValidBindingPause({ ...query, binding_pause: { status: "unavailable", binding_id: `${bindingId}-other` } })).toBe(false);
    expect(isBindingPauseReceipt({ ...receipt, binding_id: bindingId })).toBe(false);
    expect(hasValidBindingPause({ ...query, binding_pause: { ...current, pending: { command_id: " padded-command ", paused: true } } })).toBe(false);
  });

  it("preserves stored binding identity without normalization and rejects blank or NUL IDs", () => {
    expect(hasValidBindingPause({ binding_id: "legacy binding", binding_pause: { ...pause(), binding_id: " legacy binding " } })).toBe(false);
    for (const bindingId of ["", " \t\n", "\u0085", "legacy\0binding"]) {
      expect(hasValidBindingPause({ binding_id: bindingId, binding_pause: { ...pause(), binding_id: bindingId } })).toBe(false);
    }
  });

  it("reports accepted pending separately and never treats a wrong receipt as success", () => {
    const accepted = { command_id: "pause-1", binding_id: "binding-1", requested_paused: true, accepted_revision: 4, receipt: null };
    expect(bindingPauseRequestNotice(accepted, command)).toMatchObject({ known: true, notice: expect.stringContaining("접수") });
    expect(bindingPauseRequestNotice(accepted, command).notice).not.toContain("일시정지했습니다");
    expect(bindingPauseRequestNotice({ ...accepted, receipt }, command)).toMatchObject({ known: true, notice: expect.stringContaining("일시정지했습니다") });
    expect(bindingPauseRequestNotice({ ...accepted, receipt: { ...receipt, command_id: "other" } }, command).known).toBe(false);
  });

  it("describes unchanged settings without claiming a new pause or resume completed", () => {
    for (const paused of [true, false]) {
      const unchanged = { ...receipt, revision: command.expected_revision, paused, outcome: "unchanged", changed: false };
      const accepted = { command_id: command.command_id, binding_id: command.binding_id, requested_paused: paused,
        accepted_revision: command.expected_revision, receipt: unchanged };
      expect(bindingPauseRequestNotice(accepted, { ...command, paused })).toEqual({
        known: true, notice: "설정이 이미 요청한 상태입니다.",
      });
    }
  });

  it("does not let an older run revision or pre-completion pending response reopen a completed generation", () => {
    const resumed = answer({ status: "ready", binding_id: "binding-1", revision: 5, paused: false, pending: null });
    expect(preserveNewerBindingPause(answer(pause(4)), resumed)).toEqual(resumed);
    expect(preserveNewerBindingPause(answer({ status: "ready", binding_id: "binding-1", revision: 5, paused: true, pending: { command_id: "resume", paused: false } }), resumed)).toEqual(resumed);
  });

  it("keeps pending and unavailable controls disabled and offers native resume only after completion", () => {
    const pending: IssueBindingPauseState = { status: "ready", binding_id: "binding-1", revision: 4, paused: false, pending: { command_id: "pause-1", paused: true } };
    expect(bindingPauseControlLabel(pending)).toBe("일시정지 처리 중");
    expect(bindingPauseControlAction(pending, true, false, true)).toEqual({ label: "처리 중…", disabled: true });
    const resuming: IssueBindingPauseState = { ...pending, paused: true, pending: { command_id: "resume-1", paused: false } };
    expect(bindingPauseControlLabel(resuming)).toBe("재개 확인 중");
    expect(bindingPauseControlAction(resuming, true, false, true).disabled).toBe(true);
    const unavailable: IssueBindingPauseState = { status: "unavailable", binding_id: "binding-1" };
    expect(bindingPauseControlLabel(unavailable)).toBe("일시정지 상태 확인 필요");
    expect(bindingPauseControlAction(unavailable, true, false, true).disabled).toBe(true);
    expect(bindingPauseControlLabel(pause())).toBe("일시정지됨");
    expect(bindingPauseControlAction(pause(), true, false, true)).toEqual({ label: "이슈 자동화 재개…", disabled: false });
    expect(bindingPauseControlAction(pause(), false, false, true).disabled).toBe(true);
    expect(bindingPauseControlAction(pause(), true, true, true).disabled).toBe(true);
    expect(bindingPauseControlAction(pause(), true, false, false).disabled).toBe(true);
  });
});
