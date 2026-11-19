// `npm run e2e` 가 실행할 화면 시험 목록. scripts/run-e2e.mjs 와 test/runE2e.test.ts 가 함께 쓴다.

/** 기본 gate 에서 빼는 수동 시험과 그 이유. */
export const MANUAL_E2E_SPECS = new Map([
  ["browserAutomation.mjs", "opens a visible Electron window (show: true)"],
  ["browserBridge.mjs", "needs AKRA_GATEWAY_BIN to point to a built native gateway"],
]);

/** e2e 폴더의 파일 이름에서 기본 gate 가 실행할 시험을 이름 순서로 돌려준다. */
export function defaultE2eSpecs(fileNames) {
  return fileNames.filter((name) => name.endsWith(".mjs") && !MANUAL_E2E_SPECS.has(name)).sort();
}
