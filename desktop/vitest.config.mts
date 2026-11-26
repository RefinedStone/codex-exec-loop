import { configDefaults, defineConfig } from "vitest/config";

/**
 * 실제 git 을 띄우는 시험이다 (#2869). 시험 하나가 git 을 약 50 번 띄운다. Windows 에서 git 한 번은 혼자 약 30ms 이지만
 * 기본 worker 수(코어 수 - 1)만큼 동시에 띄우면 한 번에 약 150ms 가 되어 5 초 제한을 넘는다. 그래서 이 시험들은 나머지
 * 시험이 끝난 뒤에 적은 worker 로 돈다.
 *
 * 2026-11-26 측정(24 코어 Windows, 다른 일 없음): 나누기 전 전체 실행 6 건 timeout. 나눈 뒤 5 회 연속 0 건, 가장 느린 시험
 * 4.8 초. 각 파일의 첫 시험은 기준 저장소를 만드는 일까지 한다. 부하가 있는 기계와 CI 의 여유로 이 묶음만 15 초를 준다.
 */
const REAL_GIT_TESTS = [
  "test/gitStatus.test.ts",
  "test/gitStatusFifo.test.ts",
  "test/gitHunks.test.ts",
  "test/gitBranch.test.ts",
  "test/gitPublish.test.ts",
];
const REAL_GIT_WORKERS = 2;
const REAL_GIT_TEST_TIMEOUT_MS = 15_000;

export default defineConfig({
  test: {
    environment: "node",
    projects: [
      {
        extends: true,
        test: {
          name: "unit",
          include: ["test/**/*.test.ts", "src/**/*.test.ts"],
          exclude: [...configDefaults.exclude, ...REAL_GIT_TESTS],
        },
      },
      {
        extends: true,
        test: {
          name: "real-git",
          include: REAL_GIT_TESTS,
          maxWorkers: REAL_GIT_WORKERS,
          testTimeout: REAL_GIT_TEST_TIMEOUT_MS,
          hookTimeout: REAL_GIT_TEST_TIMEOUT_MS,
          sequence: { groupOrder: 1 },
        },
      },
    ],
  },
});
