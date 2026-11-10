// Notion 업무 패널이다 (plan 21 3단계). Gateway 의 MCP hub 로 Notion 을 읽는다. 로그인은 hub 가 하므로 패널에는
// 연결 화면이 없다. 등록이나 로그인이 필요하면 `akra mcp` 명령을 보여 준다. 열 때 최근 페이지를 읽고, 검색은
// 사용자가 요청할 때만 한다. "스레드로 넘기기" 는 페이지 내용으로 새 스레드를 만들고 입력창만 채운다.
import { type FormEvent, useEffect, useMemo, useState } from "react";

import type { NotionBridge } from "../../shared/bridge";
import { JsonRpcError } from "../rpc/jsonRpcClient";
import { type GatewayRequest, NOTION_SERVER, NotionHub, type NotionHubState, type NotionPage, type NotionTools } from "./notionHub";

const ADD_COMMAND = `akra mcp add ${NOTION_SERVER} --url https://mcp.notion.com/mcp`;
const LOGIN_COMMAND = `akra mcp login ${NOTION_SERVER}`;
const JSON_RPC_METHOD_NOT_FOUND = -32601;

type HubView = { phase: "loading" } | { phase: "unsupported" } | { phase: "error"; message: string } | { phase: "state"; state: NotionHubState };
type ListState =
  | { phase: "idle" }
  | { phase: "loading" }
  | { phase: "loaded"; title: string; pages: NotionPage[] }
  | { phase: "failed"; message: string };

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function Commands({ commands, note }: { commands: string[]; note: string }) {
  return (
    <div className="work-section" data-testid="notion-setup">
      <p className="muted">{note}</p>
      {commands.map((command) => (
        <pre key={command} className="work-command">
          {command}
        </pre>
      ))}
      <p className="muted">로그인은 모든 세션과 Akra 가 같이 씁니다. 끝나면 "다시 확인" 을 누르세요.</p>
    </div>
  );
}

function PageRow({
  page,
  notion,
  onHandOff,
}: {
  page: NotionPage;
  notion: NotionBridge;
  onHandOff(page: NotionPage): Promise<void>;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const handOff = async () => {
    setBusy(true);
    setError(null);
    try {
      await onHandOff(page);
    } catch (cause) {
      setError(message(cause));
    } finally {
      setBusy(false);
    }
  };
  return (
    <li className="work-item" data-testid="notion-page">
      <button type="button" className="work-link" onClick={() => notion.openExternal(page.url)} title={page.url}>
        {page.title}
      </button>
      <div className="work-meta">
        {page.type && <span className="muted">{page.type}</span>}
        <button type="button" onClick={() => void handOff()} disabled={busy} data-testid="notion-hand-off">
          {busy ? "읽는 중…" : "스레드로 넘기기"}
        </button>
      </div>
      {error && <div className="error">{error}</div>}
    </li>
  );
}

export function NotionPanel({
  request,
  notion,
  onHandOff,
}: {
  request: GatewayRequest;
  notion: NotionBridge;
  onHandOff(prompt: string): Promise<void>;
}) {
  const hub = useMemo(() => new NotionHub(request), [request]);
  const [view, setView] = useState<HubView>({ phase: "loading" });
  const [list, setList] = useState<ListState>({ phase: "idle" });
  const [query, setQuery] = useState("");
  const tools: NotionTools | null = view.phase === "state" && view.state.kind === "ready" ? view.state.tools : null;

  const check = () => {
    setView({ phase: "loading" });
    hub.state().then(
      (state) => setView({ phase: "state", state }),
      (error: unknown) =>
        setView(
          error instanceof JsonRpcError && error.code === JSON_RPC_METHOD_NOT_FOUND
            ? { phase: "unsupported" }
            : { phase: "error", message: message(error) },
        ),
    );
  };

  const load = (title: string, read: (ready: NotionTools) => Promise<NotionPage[]>) => {
    if (!tools) return;
    setList({ phase: "loading" });
    read(tools).then(
      (pages) => setList({ phase: "loaded", title, pages }),
      (error: unknown) => setList({ phase: "failed", message: message(error) }),
    );
  };

  useEffect(check, [hub]);
  useEffect(() => {
    if (tools?.recent) load("최근 본 페이지", (ready) => hub.recent(ready));
  }, [tools]);

  const search = (event: FormEvent) => {
    event.preventDefault();
    const words = query.trim();
    if (words) load(`검색: ${words}`, (ready) => hub.search(ready, words));
  };
  const handOff = async (page: NotionPage) => {
    if (!tools) return;
    await onHandOff(await hub.pagePrompt(tools, page.url));
  };

  return (
    <aside className="work-panel" data-testid="notion-panel">
      <div className="work-header">
        <strong>Notion</strong>
        <span className="muted">MCP hub</span>
        <span className="work-actions">
          <button type="button" onClick={check} disabled={view.phase === "loading"} data-testid="notion-recheck">
            {view.phase === "loading" ? "확인 중…" : "다시 확인"}
          </button>
        </span>
      </div>
      {view.phase === "unsupported" && (
        <div className="error work-section" data-testid="notion-unsupported">
          이 Gateway 는 MCP hub 를 지원하지 않습니다. akra-gateway 를 새 버전으로 다시 시작하세요.
        </div>
      )}
      {view.phase === "error" && (
        <div className="error work-section" data-testid="notion-error">
          {view.message}
        </div>
      )}
      {view.phase === "state" && view.state.kind === "unregistered" && (
        <Commands commands={[ADD_COMMAND, LOGIN_COMMAND]} note="MCP hub 에 Notion 서버가 없습니다. 터미널에서 등록하고 로그인하세요." />
      )}
      {view.phase === "state" && view.state.kind === "needsLogin" && (
        <Commands commands={[LOGIN_COMMAND]} note="Notion 에 로그인해야 합니다. 명령이 보여 주는 주소를 브라우저에서 여세요." />
      )}
      {view.phase === "state" && view.state.kind === "failed" && (
        <div className="error work-section" data-testid="notion-error">
          {view.state.detail}
        </div>
      )}
      {tools && (
        <div className="work-scroll">
          <form className="work-section work-form" onSubmit={search} data-testid="notion-search">
            <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Notion 검색" aria-label="Notion 검색" />
            <button type="submit" disabled={!query.trim() || list.phase === "loading"}>
              검색
            </button>
          </form>
          {list.phase === "loading" && <p className="muted work-section">읽는 중…</p>}
          {list.phase === "failed" && (
            <div className="error work-section" data-testid="notion-error">
              {list.message}
            </div>
          )}
          {list.phase === "loaded" && (
            <section className="work-section">
              <h3>
                {list.title} ({list.pages.length})
              </h3>
              {list.pages.length === 0 && <p className="muted">페이지가 없습니다.</p>}
              <ul>
                {list.pages.map((page) => (
                  <PageRow key={page.url} page={page} notion={notion} onHandOff={handOff} />
                ))}
              </ul>
            </section>
          )}
        </div>
      )}
    </aside>
  );
}
