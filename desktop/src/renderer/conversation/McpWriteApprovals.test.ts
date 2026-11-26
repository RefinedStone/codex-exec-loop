import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { ArgumentsView } from "./McpWriteApprovals";

const render = (value: unknown) => renderToStaticMarkup(createElement(ArgumentsView, { value }));

describe("MCP write approval arguments (#2830)", () => {
  it("shows short arguments whole without an expander", () => {
    const html = render({ page: "p1" });
    expect(html).toContain("&quot;page&quot;: &quot;p1&quot;");
    expect(html).not.toContain("mcp-write-arguments");
  });

  it("keeps a short preview and lets the user expand the full arguments before approving", () => {
    const tail = "END-OF-ARGUMENTS";
    const html = render({ body: `${"x".repeat(700)}${tail}` });
    const [preview, details] = html.split('<details');
    expect(preview).toContain("…");
    expect(preview).not.toContain(tail);
    expect(details).toContain('data-testid="mcp-write-arguments"');
    expect(details).toContain(tail);
  });
});
