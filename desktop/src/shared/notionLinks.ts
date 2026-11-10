// Notion 패널이 기본 브라우저로 열 수 있는 주소이다. main 이 renderer 가 보낸 주소를 이것으로 확인한다.
const NOTION_HOSTS = ["notion.so", "www.notion.so", "app.notion.com", "notion.com", "www.notion.com"];

export function isNotionPageUrl(value: unknown): value is string {
  if (typeof value !== "string") return false;
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    return false;
  }
  if (url.protocol !== "https:" || url.username || url.password || url.port) return false;
  // 공개 사이트(Notion Sites)는 `<이름>.notion.site` 이다.
  return NOTION_HOSTS.includes(url.hostname) || url.hostname.endsWith(".notion.site");
}
