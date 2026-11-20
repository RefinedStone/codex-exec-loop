import type { ChatConversation, ChatSummary } from "../../src/renderer/session/desktopStore";

export interface FakeChatAccount {
  readonly chats?: readonly ChatSummary[];
  readonly chatConversations?: Readonly<Record<string, ChatConversation>>;
}

export interface FakeChatOptions extends FakeChatAccount {
  readonly chatsRateLimited?: boolean;
}

const limited = { limited: true, retryAfterSeconds: 30 } as const;
const rateLimitError = { error: { code: -32029, message: "ChatGPT limited history reads; retry later", data: { retryAfterSeconds: limited.retryAfterSeconds } } } as const;

export class FakeChatHistory {
  private account: FakeChatAccount;
  private cachedList: readonly ChatSummary[] | undefined;
  private readonly cachedConversations = new Map<string, ChatConversation>();
  rateLimited: boolean;

  constructor(options: FakeChatOptions) {
    this.account = options;
    this.rateLimited = options.chatsRateLimited === true;
  }

  switchAccount(account: FakeChatAccount): void {
    this.account = account;
    this.cachedList = undefined;
    this.cachedConversations.clear();
  }

  reply(method: string, params: unknown) {
    if (this.account.chats === undefined) return undefined;
    if (method === "akra/chats/list") {
      if (this.rateLimited) {
        return this.cachedList === undefined ? rateLimitError : { result: { chats: this.cachedList, ...limited } };
      }
      this.cachedList = [...this.account.chats];
      return { result: { chats: this.cachedList } };
    }
    if (method !== "akra/chats/read") return undefined;
    const id = typeof params === "object" && params !== null && "id" in params ? params.id : undefined;
    if (typeof id !== "string") return { error: { code: -32602, message: "conversation id required" } };
    if (this.rateLimited) {
      const cached = this.cachedConversations.get(id);
      return cached === undefined ? rateLimitError : { result: { ...cached, ...limited } };
    }
    const conversation = this.account.chatConversations?.[id];
    if (conversation === undefined) return { error: { code: -32000, message: "conversation not found" } };
    this.cachedConversations.set(id, conversation);
    return { result: conversation };
  }
}
