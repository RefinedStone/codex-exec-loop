// 녹화 정리 스크립트가 허용 목록의 구조 값만 남기는지 확인한다 (리뷰 지적: 토큰형 문자열 원문 노출).
import { describe, expect, it } from "vitest";

// @ts-expect-error -- plain ESM script without type declarations
import { sanitizeValue } from "../scripts/sanitize-recording.mjs";

describe("sanitize-recording", () => {
  it("keeps protocol structure but rewrites token-like values under other keys", () => {
    const message = {
      jsonrpc: "2.0",
      method: "item/agentMessage/delta",
      params: {
        threadId: "01a1abfb-8f0a-7581-a8e4-c2953347f470",
        token: "ghp_1234567890abcdef",
        url: "https://example.com/repo.git?token=abc",
        repository: "private-org/private-repo",
        cwd: "C:/Users/someone/work",
        delta: "hello there",
        status: "completed",
      },
    };
    const sanitized = sanitizeValue("", message) as { method: string; params: Record<string, string> };
    expect(sanitized.method).toBe("item/agentMessage/delta");
    expect(sanitized.params.threadId).toBe(message.params.threadId);
    expect(sanitized.params.status).toBe("completed");
    expect(sanitized.params.cwd).toBe("/work/example-app");
    for (const key of ["token", "url", "repository", "delta"]) {
      expect(sanitized.params[key]).not.toBe(message.params[key as keyof typeof message.params]);
      expect(sanitized.params[key]).toMatch(/^word/);
    }
  });
});
