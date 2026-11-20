import type { JsonRpcClient } from "../rpc/jsonRpcClient";
import { type ChatsLimitState, chatsListProblem, staleNotice } from "./chatsProblem";
import type { ChatSummary, SessionStore } from "./desktopStore";

export class ChatHistoryList {
  private loading = false;
  private attempted = false;

  constructor(
    private readonly client: JsonRpcClient,
    private readonly store: Pick<SessionStore, "setChats" | "setChatsProblem">,
  ) {}

  connected(): void {
    if (!this.attempted) void this.reload();
  }

  async reload(): Promise<void> {
    if (this.loading) return;
    this.attempted = true;
    this.loading = true;
    try {
      const response = await this.client.request<{ readonly chats: ChatSummary[] } & ChatsLimitState>("akra/chats/list");
      this.store.setChats(response.chats);
      const stale = staleNotice(response);
      if (stale !== null) this.store.setChatsProblem(stale);
    } catch (error) {
      const problem = chatsListProblem(error);
      if (problem === null) this.store.setChats(null);
      else this.store.setChatsProblem(problem);
    } finally {
      this.loading = false;
    }
  }
}
