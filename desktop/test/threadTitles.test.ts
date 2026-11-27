import { describe, expect, it } from "vitest";
import { ProvisionalThreadTitles, provisionalThreadTitle, threadDisplayTitle, UNTITLED_THREAD } from "../src/renderer/session/threadTitles";

describe("display-only provisional thread titles", () => {
  it("preserves authoritative names and uses one bounded first meaningful line for fallback", () => {
    expect(threadDisplayTitle("내가 정한 이름", "다른 이름")).toBe("내가 정한 이름");
    expect(threadDisplayTitle("  ", "\n  Kotlin Kafka #2796\nadditional details")).toBe("Kotlin Kafka #2796");
    expect(threadDisplayTitle(null, "hello")).toBe(UNTITLED_THREAD);
    expect(provisionalThreadTitle("hello!\nFix Kafka #123")).toBe("Fix Kafka #123");
    expect(provisionalThreadTitle("안녕하세요!\nKafka consumer 수정")).toBe("Kafka consumer 수정");
    expect(provisionalThreadTitle("hello\u202e")).toBeNull();
    expect(provisionalThreadTitle("hello\n```rs\nfn main() {}")).toBeNull();
    expect(provisionalThreadTitle("hello\nfix Kafka\npassword: abc")).toBeNull();
    const cluster = "👩🏽‍💻";
    expect(provisionalThreadTitle(cluster.repeat(50))).toBe(cluster.repeat(35) + "…");
    expect(provisionalThreadTitle("작업\u202e 확인")).toBe("작업 확인");
  });

  it("does not display obvious credential-bearing or code-fence input as a title", () => {
    for (const text of ["api_key=example", "Bearer test-token", "설정\npassword: abc", "https://user:pass@example.org", "```rs\nfn main() {}"]) {
      expect(provisionalThreadTitle(text)).toBeNull();
    }
  });

  it("only accepts a meaningful input for a newly registered unnamed local thread", () => {
    const titles = new ProvisionalThreadTitles();
    titles.register("old", "외부 이름", "");
    titles.register("existing", null, "기존 대화");
    titles.register("new", null, "");
    expect(titles.accepted("old", "덮어쓰지 않음")).toBeNull();
    expect(titles.accepted("existing", "덮어쓰지 않음")).toBeNull();
    expect(titles.accepted("new", "안녕하세요!")).toBeNull();
    expect(titles.accepted("new", "Kafka consumer 리뷰 #2796")).toBe("Kafka consumer 리뷰 #2796");
    expect(titles.accepted("new", "second task")).toBeNull();
    expect(titles.preview("new", null, "")).toBe("Kafka consumer 리뷰 #2796");
    expect(titles.preview("another", null, "서버 미리보기")).toBe("서버 미리보기");
  });

  it("a manual attempt, external name, archive or reconnect prevents late provisional updates", () => {
    const titles = new ProvisionalThreadTitles();
    titles.register("manual", null, "");
    titles.stopGenerating("manual");
    expect(titles.accepted("manual", "late")).toBeNull();
    titles.register("external", null, "");
    expect(titles.accepted("external", "first task")).toBe("first task");
    expect(titles.preview("external", "실제 이름", "server")).toBe("server");
    expect(titles.preview("external", null, "server")).toBe("server");
    titles.register("archived", null, "");
    titles.forget("archived");
    expect(titles.accepted("archived", "late")).toBeNull();
    titles.register("reconnect", null, "");
    titles.clear();
    expect(titles.accepted("reconnect", "late")).toBeNull();
  });
});
