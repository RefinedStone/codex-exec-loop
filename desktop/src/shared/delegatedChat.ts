export const CHAT_CONVERSATION = /^https:\/\/chatgpt\.com\/c\/[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}$/i;
export const MAX_CHAT_RESUMES = 2;

export type DelegatedChatSession = { mode: "new" } | { mode: "continue"; chatUrl: string };
