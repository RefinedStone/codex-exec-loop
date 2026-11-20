import type { JsonRpcClient } from "../rpc/jsonRpcClient";
import { type ChatsLimitState, chatsListProblem, staleNotice } from "./chatsProblem";
import type { ChatConversation, ChatSummary, SessionStore } from "./desktopStore";

export class ChatHistory {
  private listRequest: object | undefined;
  private readRequest: { readonly id: string } | undefined;
  private attempted = false;
  private unavailable = false;
  private ids = new Set<string>();

  constructor(
    private readonly client: JsonRpcClient,
    private readonly store: Pick<SessionStore, "setChats" | "setChatConversation">,
  ) {}

  connected(): void {
    if (!this.attempted) void this.reload();
  }

  disconnected(): void {
    if (this.attempted && !this.unavailable) this.invalidate("대화 목록을 보려면 새로 고침을 누르세요.");
  }

  async reload(): Promise<void> {
    if (this.listRequest) return;
    const request = {};
    this.listRequest = request;
    this.attempted = true;
    try {
      const response = await this.client.request<{ readonly chats: ChatSummary[] } & ChatsLimitState>("akra/chats/list");
      if (this.listRequest !== request) return;
      this.unavailable = false;
      // The Gateway exposes conversation IDs, not an account ID. Only keep a body whose ID is in the new list.
      this.ids = new Set(response.chats.map((chat) => chat.id));
      if (this.readRequest && !this.ids.has(this.readRequest.id)) this.readRequest = undefined;
      this.store.setChats(response.chats, staleNotice(response));
    } catch (error) {
      if (this.listRequest === request) this.invalidate(chatsListProblem(error));
    } finally {
      if (this.listRequest === request) this.listRequest = undefined;
    }
  }

  async read(id: string): Promise<void> {
    if (!this.ids.has(id)) return;
    const request = { id };
    this.readRequest = request;
    try {
      const conversation = await this.client.request<ChatConversation>("akra/chats/read", { id });
      if (this.readRequest === request) this.store.setChatConversation(conversation);
    } catch (error) {
      if (this.readRequest === request) this.invalidate(chatsListProblem(error));
    } finally {
      if (this.readRequest === request) this.readRequest = undefined;
    }
  }

  // An error carries no account-scoped data. Pending replies must not restore the cleared history.
  invalidate(problem: string | null): void {
    this.unavailable = problem === null;
    this.listRequest = undefined;
    this.readRequest = undefined;
    this.ids.clear();
    this.store.setChats(problem === null ? null : [], problem);
  }
}
