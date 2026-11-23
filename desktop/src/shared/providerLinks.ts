// 공개 URL 검증만 한다. 자격 증명이나 네트워크 요청은 이 모듈에 없다.
const JIRA_KEY = /^[A-Z][A-Z0-9_]{0,49}-\d{1,9}$/;

export function normalizeJiraSite(input: unknown): string | null {
  if (typeof input !== "string" || input.length > 4_096) return null;
  try {
    const url = new URL(input.trim());
    if (url.protocol !== "https:" || url.username || url.password || /[\\\u0000-\u001f\u007f]/.test(input)) return null;
    return `${url.origin}${url.pathname.replace(/\/+$/, "")}`;
  } catch { return null; }
}

export function isJiraIssueKey(value: unknown): value is string {
  return typeof value === "string" && JIRA_KEY.test(value);
}

export function isJiraIssueUrl(site: unknown, input: unknown): input is string {
  const normalized = normalizeJiraSite(site);
  if (!normalized || typeof input !== "string" || input.length > 8_192) return false;
  const prefix = `${normalized}/browse/`;
  return input.startsWith(prefix) && isJiraIssueKey(input.slice(prefix.length));
}

export function isLinearIssueUrl(input: unknown): input is string {
  if (typeof input !== "string" || input.length > 2_048
    || !input.startsWith("https://linear.app/") || /[\s\\\u0000-\u001f\u007f]/.test(input)) return false;
  try {
    const url = new URL(input);
    return url.protocol === "https:" && url.hostname === "linear.app" && !url.port
      && !url.username && !url.password
      && /^\/[A-Za-z0-9_-]+\/issue\/[A-Za-z][A-Za-z0-9_]*-\d+(?:\/[A-Za-z0-9_%.-]+)?\/?$/.test(url.pathname)
      && !/%(?:2f|5c|00|0a|0d)/i.test(url.pathname)
      && !input.split(/[?#]/, 1)[0]!.split("/").some((part) => part === "." || part === "..");
  } catch { return false; }
}
