import { describe, expect, it } from "vitest";

import { appendQuote, quoteText } from "./quote";

describe("conversation quote (#2842)", () => {
  it("quotes each line and keeps the existing draft", () => {
    expect(quoteText("first\n\nsecond  \n")).toBe("> first\n>\n> second");
    expect(appendQuote("", "a")).toBe("> a\n\n");
    expect(appendQuote("my note\n", "a\nb")).toBe("my note\n\n> a\n> b\n\n");
  });
});
