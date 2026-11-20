// 서버가 준 준비검사 DTO의 표시만 시험한다. 실제 판정은 Rust 회귀가 검증한다.
export function readinessAnswer({ summary = "서버의 준비 조건을 충족합니다.", readiness = "ready", missingSections = [] } = {}) {
  return {
    status: "assessed",
    assessment: {
      previewOnly: true, readiness, summary, missingSections, errors: [],
      riskClass: "high", riskSummary: "서버 예상: 높음",
      effectiveMode: "observe", assumedMode: "propose", modeSummary: "현재 관찰 · 예상 기준 제안",
      expectedGates: [
        { gate: "merge", action: "human", summary: "먼저 온 통합", actionSummary: "사람 확인" },
        { gate: "start", action: "human", summary: "나중 온 시작", actionSummary: "사람 확인" },
      ],
      reasons: [{ code: "execution_grant_required", summary: "게시 후 현재 자격을 확인합니다." }],
      policyVersion: "synthetic-automation-v1", disclosurePolicyVersion: "synthetic-disclosure-v1",
      sharedCopyHash: { version: 1, sha256: "synthetic-readiness-copy" },
    },
  };
}

export function readinessBlocked(reason = "policy_blocked") {
  return {
    status: "cannot_assess", reason,
    summary: reason === "disclosure_confirmation_required"
      ? "AI 전달 공개 정책에서 확인이 필요합니다."
      : reason === "frozen_draft" ? "기록된 게시본의 준비검사를 생략합니다."
        : "자동화 정책을 해석하지 못했습니다.",
    nextSummary: "게시 승인과 생성 결과 확인은 별도로 유지됩니다.",
    policyError: reason === "policy_blocked" ? { scope: "project", field: "policy", reason: "invalid_toml" } : null,
  };
}


/** 두 초안 화면의 비동기 표시 계약을 같은 가짜 응답으로 검증한다. */
export function readinessFixture() {
  return {
    current: readinessAnswer({
      readiness: "needs_info", summary: "서버가 완료 기준을 요청합니다.", missingSections: ["완료 기준"],
    }),
    holdNext: false,
    pending: [],
    reply(preview, frozen = false) {
      const result = { ...preview, readiness: frozen ? readinessBlocked("frozen_draft") : structuredClone(this.current) };
      if (!this.holdNext) return result;
      this.holdNext = false;
      return new Promise((resolve, reject) => this.pending.push({ result, resolve, reject }));
    },
  };
}

let barrierSequence = 0;
async function flushReplies(page) {
  const id = "readiness-ui-barrier-" + ++barrierSequence;
  // 같은 연결의 앞선 응답이 renderer까지 도착했음을 확인한다. 제품 명령은 추가하지 않는다.
  await page.evaluate((requestId) => new Promise((resolve, reject) => {
    let unsubscribe = () => {};
    const timeout = window.setTimeout(() => {
      unsubscribe();
      reject(new Error("준비검사 응답 확인 시간이 지났습니다."));
    }, 5000);
    unsubscribe = window.akra.onMessage((text) => {
      const message = JSON.parse(text);
      if (message.id !== requestId) return;
      unsubscribe();
      window.clearTimeout(timeout);
      window.setTimeout(resolve, 0);
    });
    window.akra.send(JSON.stringify({ jsonrpc: "2.0", id: requestId, method: "akra/issue/readinessBarrier", params: {} }));
  }), id);
}

export async function exerciseReadiness({ page, panel, fixture, requests, previewMethod, approval, publish, errorId, until, check }) {
  const readiness = panel.getByTestId("issue-draft-readiness");
  const assessed = panel.getByTestId("issue-draft-readiness-assessed");
  const blocked = panel.getByTestId("issue-draft-readiness-blocked");
  const labels = panel.getByTestId("issue-draft-expected-labels");
  const refresh = panel.getByTestId("issue-draft-readiness-refresh");
  const summaryIs = (summary) => until(async () => (await assessed.count()) === 1 && (await assessed.innerText()).includes(summary));
  check("두 초안 화면은 서버가 준 누락 항목과 다음 준비 상태를 표시한다",
    await until(async () => (await panel.getByTestId("issue-draft-readiness-missing").count()) === 1 &&
      (await panel.getByTestId("issue-draft-readiness-missing").innerText()).includes("완료 기준")));
  fixture.current = readinessAnswer({ summary: "현재 서버 준비 결과" });
  await refresh.click();
  check("현재 서버 준비 결과를 별도 표시한다", await summaryIs("현재 서버 준비 결과"));
  const gateText = await panel.getByTestId("issue-draft-readiness-gates").innerText();
  check("예상 관문은 서버 순서와 설명을 그대로 표시한다",
    gateText.indexOf("먼저 온 통합") < gateText.indexOf("나중 온 시작") &&
    (await readiness.innerText()).includes("실행 허가가 아닙니다."));
  check("예상 위험도는 renderer에서 판정하지 않는다",
    (await panel.getByTestId("issue-draft-readiness-risk").innerText()).includes("서버 예상: 높음"));
  await approval.check();

  fixture.current = readinessBlocked();
  await refresh.click();
  check("잘못된 자동화 정책은 준비검사만 막는다",
    await until(async () => (await blocked.count()) === 1 && (await blocked.getAttribute("data-reason")) === "policy_blocked") &&
    await approval.isChecked() && await publish.isEnabled());
  fixture.current = readinessBlocked("disclosure_confirmation_required");
  await refresh.click();
  check("AI 전달 확인은 게시 승인과 별도 상태로 표시한다",
    await until(async () => (await blocked.count()) === 1 && (await blocked.getAttribute("data-reason")) === "disclosure_confirmation_required") &&
    await approval.isChecked());

  fixture.current = readinessAnswer({ summary: "지연된 예전 결과" });
  fixture.holdNext = true;
  const pendingCount = fixture.pending.length;
  await labels.fill("akra:auto");
  if (!await until(() => fixture.pending.length === pendingCount + 1)) throw new Error("지연된 준비검사를 받지 못했습니다.");
  check("새 준비검사 중에는 이전 상태를 숨긴다",
    (await panel.getByTestId("issue-draft-readiness-pending").count()) === 1 &&
    (await blocked.count()) === 0 && (await assessed.count()) === 0);
  fixture.current = readinessAnswer({ summary: "라벨 변경 후 현재 결과" });
  await labels.fill(" akra:gated, bug ");
  check("예상 라벨 변경은 최신 결과만 보여 준다", await summaryIs("라벨 변경 후 현재 결과"));
  const lastPreview = requests.filter(({ method }) => method === previewMethod).at(-1);
  check("입력한 예상 라벨만 preview에 전달하고 게시 승인은 유지한다",
    JSON.stringify(lastPreview.params.expectedLabels) === JSON.stringify(["akra:gated", "bug"]) &&
    await approval.isChecked());
  const delayedSuccess = fixture.pending[pendingCount];
  delayedSuccess.resolve(delayedSuccess.result);
  await flushReplies(page);
  check("뒤늦은 성공은 새 준비검사를 덮어쓰지 않는다",
    (await assessed.innerText()).includes("라벨 변경 후 현재 결과") && !(await readiness.innerText()).includes("지연된 예전 결과"));

  fixture.holdNext = true;
  await labels.fill("stale-error");
  if (!await until(() => fixture.pending.length === pendingCount + 2)) throw new Error("지연된 오류 준비검사를 받지 못했습니다.");
  fixture.current = readinessAnswer({ summary: "오래된 오류 이후 현재 결과" });
  await labels.fill("akra:manual");
  if (!await summaryIs("오래된 오류 이후 현재 결과")) throw new Error("최신 준비검사가 도착하지 않았습니다.");
  fixture.pending[pendingCount + 1].reject(new Error("오래된 준비검사 실패"));
  await flushReplies(page);
  check("뒤늦은 오류는 새 결과와 기존 게시 승인을 지우지 않는다",
    (await assessed.innerText()).includes("오래된 오류 이후 현재 결과") &&
    (await panel.getByTestId(errorId).count()) === 0 && await approval.isChecked());
  check("준비검사는 게시나 생성 요청을 보내지 않는다",
    !requests.some(({ method }) => method.endsWith("/publish")));
}
