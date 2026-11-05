import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { Markdown } from "./Markdown";

const render = (text: string) => renderToStaticMarkup(createElement(Markdown, { text }));

describe("Markdown", () => {
  it("renders code blocks, inline code and GFM tables", () => {
    const html = render("Run `npm test`:\n\n```sh\nnpm test\n```\n\n| a | b |\n| - | - |\n| 1 | 2 |");
    expect(html).toContain("<code>npm test</code>");
    expect(html).toContain('<code class="language-sh">');
    expect(html).toContain("<table>");
  });

  it("opens links in a new window so the app view never navigates", () => {
    const html = render("[docs](https://example.com)");
    expect(html).toContain('href="https://example.com"');
    expect(html).toContain('target="_blank"');
    expect(html).toContain('rel="noreferrer"');
  });

  it("does not render raw HTML from the model", () => {
    const html = render('<img src=x onerror="alert(1)"><script>alert(1)</script>');
    expect(html).not.toContain("<img");
    expect(html).not.toContain("<script");
  });
});
